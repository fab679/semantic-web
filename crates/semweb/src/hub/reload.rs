//! Persistence reload: startup recovery and the multi-replica refresh
//! loop. Subscriptions are persisted to the shared store by ANY replica,
//! so replicas poll for new subscriptions and for pending deliveries in
//! the durable log that belong to their shard.

use std::time::{Duration, Instant};
use std::sync::atomic::Ordering;

use crate::hub::verification;
use crate::hub::{resolve_topic, Delivery, Hub, Subscription};


impl Hub {
    /// Periodic refresh for multi-replica deployments: subscriptions are
    /// persisted to the shared store by ANY replica (whichever receives
    /// the subscribe), so replicas poll persistence to learn about
    /// subscriptions created after their startup. Newer leases win
    /// (re-request/renewal semantics); removal happens only via
    /// unsubscribe and lease expiry, so this never resurrects deleted
    /// subscriptions.
    pub async fn refresh_from_persistence(&self) {
        let Ok(rows) = self.store.load_hub_subscriptions().await else { return };
        let mut subs = self.subs.write().await;
        for (topic, callback, stored_secret, expires) in rows {
            if expires <= verification::now_epoch() {
                continue;
            }
            let key = (topic, callback);
            match subs.get(&key) {
                Some(existing) if existing.lease_expires_epoch >= expires => {} // unchanged
                _ => {
                    let secret = stored_secret.as_deref().and_then(|s| self.crypto.decrypt(s));
                    subs.insert(
                        key,
                        Subscription {
                            secret,
                            lease_expires: Instant::now()
                                + Duration::from_secs(expires.saturating_sub(verification::now_epoch())),
                            lease_expires_epoch: expires,
                        },
                    );
                }
            }
        }
        // Also pick up deliveries logged for this shard by other replicas.
        self.redeliver_pending().await;
    }

    /// Reload state from persistence at startup:
    ///   1. subscriptions (crash recovery; expired leases are dropped per
    ///      §5.3),
    ///   2. pending deliveries from the durable log -- rebuilt with the
    ///      CURRENT topic content and re-enqueued (at-least-once).
    pub async fn load_persisted(&self) {
        match self.store.load_hub_subscriptions().await {
            Ok(rows) => {
                let now = verification::now_epoch();
                let mut subs = self.subs.write().await;
                let mut loaded = 0;
                let mut expired = 0;
                for (topic, callback, stored_secret, expires) in rows {
                    if expires <= verification::now_epoch() {
                        expired += 1;
                        continue;
                    }
                    let secret = stored_secret.as_deref().and_then(|s| self.crypto.decrypt(s));
                    if stored_secret.is_some() && secret.is_none() {
                        tracing::warn!("secret decryption failed for a persisted subscription; \
                                        storing without signing capability");
                    }
                    subs.insert(
                        (topic, callback),
                        Subscription {
                            secret,
                            lease_expires: Instant::now()
                                + Duration::from_secs(expires.saturating_sub(now)),
                            lease_expires_epoch: expires,
                        },
                    );
                    loaded += 1;
                }
                tracing::info!(
                    "subscription persistence: {loaded} active loaded, {expired} expired dropped"
                );
            }
            Err(e) => tracing::warn!("subscription persistence load failed: {e}"),
        }

        self.redeliver_pending().await;
    }

    /// Re-enqueue owned deliveries still in the durable log: startup
    /// crash-recovery AND the periodic scan (a publish on replica X logs
    /// deliveries for all shards; owners pick them up on their next scan
    /// within SEMWEB_SUBS_REFRESH_SECS). Workers ack (delete) after
    /// success / 410 / retry exhaustion.
    pub(crate) async fn redeliver_pending(&self) {
        match self.store.load_pending_deliveries().await {
            Ok(pending) => {
                let now = verification::now_epoch();
                for (id, event_id, topic, callback, stored_secret, self_url, hub_url, enqueued_at, claimed_until) in pending {
                    // Skip entries with a LIVE claim: another worker/replica
                    // is already delivering them. Entries are age-guarded
                    // too: a claim that expired means the claiming process
                    // died before acking -- exactly the at-least-once case.
                    if claimed_until > now || now.saturating_sub(enqueued_at) < 60 {
                        continue;
                    }
                    if let Err(e) = self.store.claim_delivery(&id, now + super::CLAIM_SECS).await {
                        tracing::warn!(event = event_id, callback, "claim failed: {e}");
                        continue;
                    }
                    // Resolve canonical topics ("/topics/data"); third-party
                    // topics (open hub) keep their URL as-is -- that URL is
                    // fetched at delivery time (§7).
                    let topic_static = resolve_topic(&topic).map(|t| t.to_string())
                        .unwrap_or_else(|| topic.clone());

                    // Only the owning replica redelivers.
                    if !self.owns(&topic, &callback) {
                        continue;
                    }
                    let content = match self.build_topic_content_or_fetch(&topic_static).await {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::error!(event = event_id, "redelivery content build failed: {e:?}");
                            continue;
                        }
                    };
                    let secret = stored_secret.and_then(|s| self.crypto.decrypt(&s));
                    match self.queue.try_send(Delivery {
                        id: id.clone(),
                        topic: topic.clone(),
                        callback,
                        secret,
                        content,
                        self_url,
                        hub_url,
                        event_id: event_id.clone(),
                        from_log: true,
                    }) {
                        Ok(()) => {
                            self.metrics
                                .redelivered_on_restart
                                .fetch_add(1, Ordering::Relaxed);
                        }
                        Err(_) => {
                            // Keep the log entry; the next restart retries.
                            tracing::warn!(event = event_id, "redelivery queue full; log entry kept");
                        }
                    }
                }
            }
            Err(e) => tracing::warn!("pending delivery log load failed: {e}"),
        }
    }
}
