//! Publish fan-out (spec §6/§7): build topic content once, log each
//! delivery durably, enqueue for this replica's shard, broadcast SSE.
//! Content building lives here (not in the api layer) so the durable-log
//! redelivery path can rebuild it.

use std::sync::Arc;

use crate::hub::verification;
use crate::hub::{resolve_topic, Delivery, Hub, SseEvent, TopicContent, TOPICS};
use crate::store::persistence::subscription_persist_id;
use std::sync::atomic::Ordering;

impl Hub {
    pub async fn publish(self: Arc<Self>, topic: &'static str, event_id: &str) -> usize {
        let content = match self.build_topic_content(topic).await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("publish of {topic} skipped: {e:?}");
                return 0;
            }
        };
        let self_url = format!("{}{topic}", self.config.public_url);
        let hub_url = format!("{}/hub", self.config.public_url);
        let subs = self.owned_subs(topic).await;
        // Observability: subscriptions this replica did NOT deliver for
        // (owned by other replicas sharing the store).
        let total_active = self
            .subs
            .read()
            .await
            .iter()
            .filter(|((sub_topic, _), sub)| {
                resolve_topic(sub_topic) == Some(topic)
                    && sub.lease_expires > std::time::Instant::now()
            })
            .count();
        if total_active > subs.len() {
            self.metrics
                .routed_to_other_replica
                .fetch_add((total_active - subs.len()) as u64, Ordering::Relaxed);
        }
        let mut scheduled = 0;
        for (callback, secret) in subs {
            let id = subscription_persist_id(topic, &callback);
            let stored_secret = secret.as_deref().map(|s| self.crypto.encrypt(s));
            // Durable log FIRST: a crash between log and delivery causes
            // redelivery on startup (at-least-once semantics).
            let rec = crate::store::DeliveryRecord {
                id: &id,
                event_id,
                topic,
                callback: &callback,
                secret: stored_secret.as_deref(),
                self_url: &self_url,
                hub_url: &hub_url,
            };
            if let Err(e) = self.store.persist_delivery(&rec).await {
                tracing::warn!(event = event_id, callback, "delivery log write failed: {e}");
            } else {
                // Claim for this delivery's own retry window so the
                // periodic scan never re-enqueues a live delivery.
                let _ = self
                    .store
                    .claim_delivery(&id, verification::now_epoch() + 90)
                    .await;
            }
            let delivery = Delivery {
                id: id.clone(),
                topic,
                callback,
                secret,
                content: content.clone(),
                self_url: self_url.clone(),
                hub_url: hub_url.clone(),
                event_id: event_id.to_string(),
                from_log: false,
            };
            match self.queue.try_send(delivery) {
                Ok(()) => scheduled += 1,
                Err(_) => {
                    // Explicit backpressure: the delivery stays in the
                    // durable log and is redelivered after a restart.
                    self.metrics.queue_dropped.fetch_add(1, Ordering::Relaxed);
                    tracing::error!(
                        event = event_id,
                        topic,
                        "delivery queue full; notification kept in the durable log \
                         (redelivered on restart)"
                    );
                }
            }
        }
        let _ = self.broadcast.send(SseEvent {
            topic: topic.to_string(),
            event_id: event_id.to_string(),
        });
        scheduled
    }

    /// Full topic content (§7: the hub MUST send the full
    /// contents of the topic URL; diffs are only allowed for Atom/RSS, so
    /// JSON/NDJSON topics get the full body). Lives here (not in the api
    /// layer) so the durable-log redelivery path can rebuild content.
    pub async fn build_topic_content(&self, topic: &str) -> Result<TopicContent, crate::store::StoreError> {
        match topic {
            "/topics/data" => {
                let bindings = self.store.all_bindings().await?;
                let mut body = String::new();
                for b in &bindings {
                    if let Some(line) = crate::jsonld::binding_to_line(b, &self.prefixes) {
                        body.push_str(&serde_json::to_string(&line).unwrap_or_default());
                        body.push('\n');
                    }
                }
                Ok(TopicContent {
                    body: body.into_bytes(),
                    content_type: "application/x-ndjson",
                })
            }
            "/topics/schema" => {
                let info = self.store.describe_schema().await?;
                let schema = serde_json::json!({
                    "@context": "/context.jsonld",
                    "generatedFrom": "live store state (not a cached build)",
                    "schemaFingerprint": crate::util::schema_fingerprint(
                        &info.class_uris, &info.predicate_uris),
                    "classes": info.class_uris.iter().map(|u| self.prefixes.compact(u)).collect::<Vec<_>>(),
                    "predicates": info.predicate_uris.iter().map(|u| self.prefixes.compact(u)).collect::<Vec<_>>(),
                    "controls": {
                        "fragments": "/fragments{?subject,predicate,object,after,limit,offset,graph}",
                        "sparql": "/sparql{?query}",
                        "manifest": "/manifest",
                        "hub": "/hub",
                        "topics": TOPICS,
                    },
                });
                Ok(TopicContent {
                    body: serde_json::to_vec(&schema).unwrap_or_default(),
                    content_type: "application/json",
                })
            }
            _ => Err(crate::store::StoreError(format!("unknown topic: {topic}"))),
        }
    }
}
