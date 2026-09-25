//! Publish fan-out (spec §6/§7).
//!
//! Two publish paths, one fan-out:
//!   - `publish` (canonical topics): rebuild content from the store.
//!   - `publish_external` (open-hub policy): the publisher told us
//!     `hub.mode=publish&hub.url=<topic>`; per spec §7 the hub sends "the
//!     full contents of the topic URL", so the topic resource is fetched
//!     at publish time and distributed exactly as served by the
//!     publisher (Content-Type preserved).
//!
//! Both: durable-log every owned delivery, enqueue, broadcast SSE, and
//! notify configured external hubs (§4 fault tolerance: "the publisher
//! notifies each hub").

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use crate::hub::MAX_EXTERNAL_CONTENT_BYTES;
use crate::hub::{resolve_topic, topic_matches, Delivery, Hub, SseEvent, TopicContent, TOPICS};
use crate::store::persistence::subscription_persist_id;

impl Hub {
    /// Content for any topic: canonical topics rebuild from the store;
    /// third-party topics (open hub) fetch the publisher's URL (§7).
    pub async fn build_topic_content_or_fetch(
        &self,
        topic: &str,
    ) -> Result<TopicContent, String> {
        if resolve_topic(topic).is_some() {
            self.build_topic_content(topic).await.map_err(|e| e.0)
        } else {
            self.fetch_external_content(topic).await
        }
    }

    /// Publish a canonical topic (§6): rebuild content from the store and
    /// fan out. Also notifies any configured external hubs.
    pub async fn publish(self: Arc<Self>, topic: &'static str, event_id: &str) -> usize {
        let content = match self.build_topic_content(topic).await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("publish of {topic} skipped: {e:?}");
                return 0;
            }
        };
        self.fan_out(topic, content, event_id).await
    }

    /// Publish a THIRD-PARTY topic (open-hub policy): per spec §7 the hub
    /// sends "the full contents of the topic URL", so the topic resource
    /// is fetched at publish time and distributed exactly as served.
    pub async fn publish_external(self: Arc<Self>, topic: &str, event_id: &str) -> usize {
        let content = match self.fetch_external_content(topic).await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(event = event_id, topic, "publish skipped: {e:?}");
                return 0;
            }
        };
        self.fan_out(topic, content, event_id).await
    }

    async fn fetch_external_content(&self, topic: &str) -> Result<TopicContent, String> {
        let resp = self
            .http
            .get(topic)
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| format!("topic fetch failed: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("topic fetch -> {status}"));
        }
        // §7: the Content-Type of the distribution MUST match the topic's.
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(';').next().unwrap_or(v).trim().to_string())
            .unwrap_or_else(|| "application/octet-stream".to_string());
        let body = resp.bytes().await.map_err(|e| e.to_string())?;
        // Sanity cap so a huge third-party topic cannot exhaust memory.
        if body.len() > MAX_EXTERNAL_CONTENT_BYTES {
            return Err("topic content exceeds the size cap".to_string());
        }
        Ok(TopicContent {
            body,
            content_type,
        })
    }

    /// Shared fan-out: durable-log + queue + SSE for all owned subscribers
    /// of `topic`, plus metrics. Canonical topics mint `rel=self` from our
    /// public URL; third-party topics use the PUBLISHER's canonical URL
    /// (the topic URL itself, per §7's "Link headers are metadata of the
    /// topic, not of the subscription").
    async fn fan_out(self: Arc<Self>, topic: &str, content: TopicContent, event_id: &str) -> usize {
        let topic = topic.to_string();
        let self_url = if resolve_topic(&topic).is_some() {
            format!("{}{topic}", self.config.public_url)
        } else {
            topic.to_string()
        };
        let hub_url = format!("{}/hub", self.config.public_url);
        let subs = self.owned_subs(&topic).await;
        // Observability: subscriptions this replica did NOT deliver for
        // (owned by other replicas sharing the store).
        let total_active = self
            .subs
            .read()
            .await
            .iter()
            .filter(|((sub_topic, _), sub)| {
                topic_matches(sub_topic, &topic)
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
            let id = subscription_persist_id(&topic, &callback);
            let stored_secret = secret.as_deref().map(|s| self.crypto.encrypt(s));
            // Durable log FIRST: a crash between log and delivery causes
            // redelivery on startup (at-least-once semantics).
            let rec = crate::store::DeliveryRecord {
                id: &id,
                event_id,
                topic: &topic,
                callback: &callback,
                secret: stored_secret.as_deref(),
                self_url: &self_url,
                hub_url: &hub_url,
            };
            if let Err(e) = self.store.persist_delivery(&rec).await {
                tracing::warn!(event = event_id, callback, "delivery log write failed: {e}");
            } else {
                // Claim for this delivery's own retry window so the
                // periodic scan never re-enqueues a live delivery. The
                // claim must cover the WORST-CASE delivery duration:
                // 21s of retry sleeps + 4 attempts x 10s HTTP timeout.
                let _ = self
                    .store
                    .claim_delivery(&id, verification::now_epoch() + super::CLAIM_SECS)
                    .await;
            }
            let delivery = Delivery {
                id: id.clone(),
                topic: topic.clone(),
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
                        topic = &topic,
                        "delivery queue full; notification kept in the durable log \
                         (redelivered on restart)"
                    );
                }
            }
        }
        let _ = self.broadcast.send(SseEvent {
            topic: topic.clone(),
            event_id: event_id.to_string(),
        });
        scheduled
    }

    /// §4 fault tolerance: a publisher MAY use more than one hub. Our hub
    /// advertises every configured hub and notifies the external ones on
    /// each mutation (fire-and-forget POSTs, the common §6 convention).
    pub fn notify_external_hubs(self: Arc<Self>, topic: &str, event_id: &str) {
        for hub_url in self.external_hubs() {
            let hub = self.clone();
            let (topic, event_id, hub_url) =
                (topic.to_string(), event_id.to_string(), hub_url);
            tokio::spawn(async move {
                if let Err(e) = hub
                    .http
                    .post(hub_url.clone())
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body(format!(
                        "hub.mode=publish&hub.url={}",
                        urlencoding::encode(&topic)
                    ))
                    .send()
                    .await
                {
                    tracing::warn!(event = event_id, hub = hub_url, "external hub notify failed: {e}");
                } else {
                    tracing::info!(event = event_id, hub = hub_url, topic, "external hub notified");
                }
            });
        }
    }
}

use crate::hub::verification;

impl Hub {
    /// The full topic content (§7: the hub MUST send the full
    /// contents of the topic URL; diffs are only allowed for Atom/RSS, so
    /// JSON/NDJSON topics get the full body). Canonical topics are built
    /// from the store; lives here so the durable-log redelivery path can
    /// rebuild content.
    pub async fn build_topic_content(
        &self,
        topic: &str,
    ) -> Result<TopicContent, crate::store::StoreError> {
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
                    body: bytes::Bytes::from(body.into_bytes()),
                    content_type: "application/x-ndjson".to_string(),
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
                    body: bytes::Bytes::from(serde_json::to_vec(&schema).unwrap_or_default()),
                    content_type: "application/json".to_string(),
                })
            }
            _ => Err(crate::store::StoreError(format!(
                "unknown topic: {topic} \
                 (third-party topics are fetched at publish time, not here)"
            ))),
        }
    }
}