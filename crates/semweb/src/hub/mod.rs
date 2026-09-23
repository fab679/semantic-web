//! A WebSub hub implementing the W3C Recommendation of 02 June 2026
//! (`docs/WebSub.md`). This module is the normative core of the service:
//! every requirement cited below quotes that specification.
//!
//! Roles (spec §1): this service is both Publisher (it owns the topics)
//! and Hub (it validates subscriptions and distributes content);
//! subscribers are any external HTTP-callable app or agent-side service.
//!
//! Conformance map (§ numbers refer to docs/WebSub.md):
//!
//! | Spec requirement | Where |
//! |---|---|
//! | §5.1 accept hub.callback, hub.mode, hub.topic; accept hub.secret | `subscribe` / `unsubscribe` (validation in api::hub_endpoints) |
//! | §5.1.2 respond 202; MUST NOT depend on verification outcome | handlers return before the spawned verification task |
//! | §5.3 verify intent: GET callback with hub.mode/topic/challenge/lease_seconds | verification.rs |
//! | §5.3 challenge charset; §8.2 length/no-binary recommendations | `generate_challenge` |
//! | §5.3.1 2xx + body == challenge -> verified; 3xx/4xx/5xx or wrong body -> failed, state unchanged | `verify_and_commit` |
//! | §5.1 MUST allow re-request of active subscriptions; override only after verification | commit-on-success in verification.rs |
//! | §5.3 MUST enforce lease expirations, MUST NOT issue perpetual leases | `clamp_lease`, `expire_sweep` |
//! | §5.2 denied notification: GET callback hub.mode=denied&hub.topic&hub.reason | `send_denied` |
//! | §7 distribution: POST the full topic contents, Content-Type matching the topic | `build_topic_content` + delivery.rs |
//! | §7 MUST include Link headers rel=hub and rel=self | delivery.rs |
//! | §7.1 X-Hub-Signature when hub.secret supplied | delivery.rs (sha256, per §8.3) |
//! | §7 2xx ack; 410 MAY terminate subscription; retries up to self-imposed limits; subscription kept until lease end | delivery.rs |
//! | §5.1.1 preserve callback query string; never overwrite existing params | verification.rs `append_query` |
//!
//! Scale-out mechanisms implemented in this module:
//! - **Subscription persistence** (store::persistence): subscriptions
//!   survive restarts; secrets are AES-GCM-encrypted at rest when
//!   SEMWEB_SECRET_KEY is set (crypto.rs).
//! - **Durable delivery log** (HUB_DELIVERIES_GRAPH): deliveries are
//!   logged before enqueue and acked when finished; a crash mid-flight
//!   redelivers on startup instead of dropping notifications.
//! - **Multi-replica sharding**: with SEMWEB_REPLICA_COUNT > 1, a
//!   (topic, callback) is owned by exactly one replica (consistent hash
//!   of its subscription id); replicas share the store, so each replica
//!   publishes/delivers only its own shard. `hub.mode=publish` is
//!   idempotent, so any replica can accept it.
//! - **Bounded delivery queue + worker pool**: explicit backpressure --
//!   a full queue leaves the delivery in the durable log (a restart
//!   redelivers it), never unbounded memory.
//! - **SSE broadcast**: every publish event is mirrored on an in-process
//!   broadcast channel that `GET /events` streams as SSE for live agent
//!   sessions (WebSub covers durable subscribers; SSE covers open
//!   sessions -- the two are complementary).
//! - **Rate limiting** (rate_limit.rs) and **metrics** (metrics.rs).
//! - **Callback policies** (§5.1 "hubs MAY reject callback/topic URLs
//!   per their own policies"): optional HTTPS-callback requirement when
//!   secrets are used, and an optional callback-host allowlist.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{broadcast, mpsc, Mutex, RwLock};

use crate::context::PrefixMap;
use crate::store::persistence::{owner_of, subscription_persist_id};
use crate::store::SparqlStore;

pub use crate::hub::metrics::HubMetrics;
use crate::hub::crypto::SecretCrypto;
use crate::hub::rate_limit::RateLimiter;

pub(crate) mod verification;
pub(crate) mod delivery;
pub(crate) mod crypto;
pub(crate) mod rate_limit;
pub(crate) mod metrics;

/// The topics this service publishes. Spec §1: a topic is an HTTP
/// resource URL -- served with full content at `GET /topics/...` and
/// advertised via Link headers (spec §4).
pub const TOPICS: [&str; 2] = ["/topics/data", "/topics/schema"];

#[derive(Debug)]
pub enum HubError {
    /// §5.1.2: 4xx with plain-text error description for bad requests.
    BadRequest(String),
    /// §5.1.2: hubs MAY reject topic URLs per their own policies; we 404.
    UnknownTopic(String),
    /// Per-callback rate limit exhausted.
    TooManyRequests,
}

/// Resolve a submitted hub.topic to one of our canonical topic paths.
/// Subscribers should use the rel=self URL from discovery (absolute);
/// accepting the bare path keeps local development and tests friendly.
pub fn resolve_topic(topic: &str) -> Option<&'static str> {
    for t in TOPICS {
        if topic == t || topic.ends_with(t) {
            return Some(t);
        }
    }
    None
}

#[derive(Clone)]
pub(crate) enum Intent {
    Subscribe { secret: Option<String>, lease: u64 },
    Unsubscribe,
}

/// Content of a topic at publish time (spec §7: the hub MUST send the
/// full contents of the topic URL, with a Content-Type matching the
/// topic's; payloads may be reduced to diffs only for Atom/RSS).
#[derive(Clone)]
pub struct TopicContent {
    pub body: Vec<u8>,
    pub content_type: &'static str,
}

#[derive(Clone)]
pub struct Subscription {
    /// Subscriber-provided secret (§5.1 hub.secret); drives §7.1 signing.
    pub secret: Option<String>,
    /// Absolute deadline for in-memory checks.
    pub lease_expires: Instant,
    /// Deadline as unix-epoch seconds (what persistence stores).
    #[allow(dead_code)] // round-trips through persistence on re-subscribe
    pub lease_expires_epoch: u64,
}

/// One queued delivery: everything a single distribution attempt needs.
pub(crate) struct Delivery {
    /// Durable-log id (ack key).
    id: String,
    topic: &'static str,
    callback: String,
    secret: Option<String>,
    content: TopicContent,
    self_url: String,
    hub_url: String,
    event_id: String,
    /// True when re-enqueued from the durable log (metrics only).
    from_log: bool,
}

/// An event on the SSE change feed (`GET /events`): the notification
/// header only -- SSE clients refetch `GET /topics/{name}` for content,
/// keeping the stream small regardless of topic size.
#[derive(Clone, Debug, serde::Serialize)]
pub struct SseEvent {
    pub topic: String,
    pub event_id: String,
}

/// Hub configuration, from state.rs (environment).
#[derive(Clone)]
pub struct HubConfig {
    pub queue_capacity: usize,
    pub workers: usize,
    /// Sharding: total replica count and this replica's index.
    pub replica_count: u64,
    pub replica_index: u64,
    /// Public base URL for absolute rel=self / rel=hub Link headers.
    pub public_url: String,
    /// §5.1 policy: require https:// callbacks when hub.secret is used.
    pub require_https_callbacks: bool,
    /// §5.1 policy: allow-list of callback host suffixes (empty = allow all).
    pub callback_allowlist: Vec<String>,
}

pub struct Hub {
    /// Keyed by (topic-as-submitted, callback-as-submitted): the spec's
    /// subscription identity tuple (§1: "A subscription's unique key is
    /// the tuple (Topic URL, Subscriber Callback URL)").
    subs: RwLock<std::collections::HashMap<(String, String), Subscription>>,
    /// Bounded delivery queue drained by the worker pool.
    queue: mpsc::Sender<Delivery>,
    /// SSE change feed (see api::events).
    broadcast: broadcast::Sender<SseEvent>,
    rate: RateLimiter,
    store: Arc<SparqlStore>,
    crypto: SecretCrypto,
    prefixes: Arc<PrefixMap>,
    config: HubConfig,
    http: reqwest::Client,
    pub metrics: HubMetrics,
}

impl Hub {
    /// Create the hub and spawn the fixed worker pool over its delivery
    /// queue. The queue's receiver is owned by the pool.
    pub fn new(store: Arc<SparqlStore>, prefixes: Arc<PrefixMap>, config: HubConfig) -> Arc<Self> {
        let (tx, rx) = mpsc::channel(config.queue_capacity);
        let (broadcast, _) = broadcast::channel(256);
        let hub = Arc::new(Hub {
            subs: RwLock::new(std::collections::HashMap::new()),
            queue: tx,
            broadcast,
            rate: RateLimiter::new(),
            store,
            crypto: SecretCrypto::from_env(),
            prefixes,
            config,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("reqwest client"),
            metrics: HubMetrics::default(),
        });

        // Fixed worker pool: decouples publish (ingest) latency from
        // delivery latency and bounds concurrent deliveries. Workers
        // share the single receiver under a mutex: exactly one worker
        // takes each queued delivery.
        let shared_rx = Arc::new(Mutex::new(rx));
        for worker in 0..hub.config.workers.max(1) {
            let hub = hub.clone();
            let rx = shared_rx.clone();
            tokio::spawn(delivery::worker_loop(hub, rx, worker));
        }
        hub
    }

    /// Read-only access to the hub configuration (for logging/health).
    pub fn config(&self) -> &HubConfig {
        &self.config
    }

    /// Subscribe to the SSE change bus (api::events holds the receiver).
    pub(crate) fn sse_receiver(&self) -> broadcast::Receiver<SseEvent> {
        self.broadcast.subscribe()
    }

    /// True when THIS replica owns (delivers for) the given subscription.
    fn owns(&self, topic: &str, callback: &str) -> bool {
        let id = subscription_persist_id(topic, callback);
        owner_of(&id, self.config.replica_count) == self.config.replica_index
    }

    /// Active (unexpired) subscription counts per canonical topic,
    /// surfaced by `GET /hub` for observability.
    pub async fn subscription_counts(&self) -> std::collections::HashMap<String, usize> {
        let now = Instant::now();
        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for ((topic, _), sub) in self.subs.read().await.iter() {
            if sub.lease_expires > now {
                *counts.entry(topic.clone()).or_insert(0) += 1;
            }
        }
        counts
    }

/// Snapshot of active (unexpired), replica-owned subscriptions for a
    /// canonical topic: (callback, secret) pairs ready for fan-out.
    async fn owned_subs(&self, topic: &'static str) -> Vec<(String, Option<String>)> {
        let now = Instant::now();
        let (count, index) = (self.config.replica_count, self.config.replica_index);
        self.subs
            .read()
            .await
            .iter()
            .filter(|((sub_topic, callback), sub)| {
                resolve_topic(sub_topic) == Some(topic)
                    && sub.lease_expires > now
                    && owner_of(&subscription_persist_id(sub_topic, callback), count) == index
            })
            .map(|((_, callback), sub)| (callback.clone(), sub.secret.clone()))
            .collect()
    }

    /// Subscriber-initiated subscribe (spec §5.1).
    ///
    /// Schedules asynchronous intent verification and returns immediately:
    /// §5.1.2 requires the 202 response to be independent of the
    /// verification outcome. Re-requests of already active subscriptions
    /// are allowed (§5.1); the (topic, callback) state is overridden only
    /// once the new action is verified, and left unchanged otherwise --
    /// which is exactly what `verify_and_commit` guarantees.
    pub async fn subscribe(
        self: Arc<Self>,
        topic: String,
        callback: String,
        secret: Option<String>,
        requested_lease: Option<u64>,
    ) -> Result<(), HubError> {
        let canonical = resolve_topic(&topic)
            .ok_or_else(|| HubError::UnknownTopic(format!("unknown topic: {topic}")))?;

        // §5.1 policy enforcement (hubs MAY reject callback/topic URLs):
        // HTTPS requirement when secrets are used, and the optional
        // callback host allowlist.
        if secret.is_some() && self.config.require_https_callbacks && !callback.starts_with("https://") {
            return Err(HubError::BadRequest(
                "hub.secret requires an https:// callback (hub policy)".into(),
            ));
        }
        if !allowlist_ok(&self.config.callback_allowlist, &callback) {
            return Err(HubError::BadRequest(
                "hub.callback host is not in the configured allowlist".into(),
            ));
        }
        if !self.rate.check(&callback).await {
            self.metrics.rate_limited.fetch_add(1, Ordering::Relaxed);
            return Err(HubError::TooManyRequests);
        }
        // §5.1: hub.secret MUST be less than 200 bytes.
        if secret.as_ref().is_some_and(|s| s.len() >= 200) {
            return Err(HubError::BadRequest(
                "hub.secret must be less than 200 bytes".into(),
            ));
        }
        let lease = verification::clamp_lease(requested_lease);
        let canonical = canonical.to_string();
        let hub = self.clone();
        tokio::spawn(async move {
            // §5.1: re-requests of active subscriptions are allowed; the
            // state for (topic, callback) is overridden only once the new
            // action is verified, and left unchanged if verification fails.
            verification::verify_and_commit(
                hub,
                &canonical,
                &topic,
                &callback,
                Intent::Subscribe { secret, lease },
            )
            .await;
        });
        Ok(())
    }

    /// Subscriber-initiated unsubscribe (spec §5.1/§5.3). Same
    /// verification dance; lease_seconds MUST be ignored for
    /// unsubscription, so none is sent in the verification request.
    pub async fn unsubscribe(self: Arc<Self>, topic: String, callback: String) -> Result<(), HubError> {
        let canonical = resolve_topic(&topic)
            .ok_or_else(|| HubError::UnknownTopic(format!("unknown topic: {topic}")))?;
        let canonical = canonical.to_string();
        let hub = self.clone();
        tokio::spawn(async move {
            verification::verify_and_commit(hub, &canonical, &topic, &callback, Intent::Unsubscribe)
                .await;
        });
        Ok(())
    }

    /// Publisher-side publish for a canonical topic (spec §6). Called
    /// in-process on mutations, and reachable externally via
    /// `POST /hub` with hub.mode=publish (§6 leaves the mechanism to the
    /// hub/publisher pair).
    ///
    /// Fan-out: build the topic content once, then for each active
    /// replica-owned subscriber log the delivery durably (crash-safe) and
    /// enqueue it. Workers perform the actual POSTs with the spec's retry
    /// contract. Also broadcasts an SSE event for live sessions.
    /// Returns the number of deliveries enqueued (0 when nothing is
    /// subscribed / owned by this replica).
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
            if let Err(e) = self
                .store
                .persist_delivery(
                    &id,
                    event_id,
                    topic,
                    &callback,
                    stored_secret.as_deref(),
                    &self_url,
                    &hub_url,
                )
                .await
            {
                tracing::warn!(event = event_id, callback, "delivery log write failed: {e}");
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

    /// §5.2 denial notification: GET the callback with
    /// hub.mode=denied, hub.topic and an optional hub.reason.
    pub(crate) async fn send_denied(&self, callback: &str, topic: &str, reason: &str) {
        let url = verification::append_query(callback, "hub.mode", "denied");
        let url = verification::append_query(&url, "hub.topic", topic);
        let url = verification::append_query(&url, "hub.reason", reason);
        self.metrics.denied_sent.fetch_add(1, Ordering::Relaxed);
        if let Err(e) = self.http.get(&url).send().await {
            tracing::warn!("denied notification to {callback} failed: {e}");
        }
    }

    /// Drop a subscription from memory and persistence (spec §7: the hub
    /// MAY terminate the subscription when the callback returns 410 Gone).
    pub(crate) async fn remove_sub(&self, topic: &str, callback: &str) {
        let removed_id = subscription_persist_id(topic, callback);
        self.subs
            .write()
            .await
            .retain(|(t, c), _| !(t.as_str() == topic && c.as_str() == callback));
        if let Err(e) = self.store.delete_hub_subscription(&removed_id).await {
            tracing::warn!("persisted subscription {removed_id} removal failed: {e}");
        }
    }

    /// One pass of the lease-expiration sweeper (spec §5.3: hubs MUST
    /// enforce lease expirations). Removes expired subscriptions from
    /// memory and from persistence -- only for subscriptions this
    /// replica owns, so replicas never interfere with each other's
    /// renewals.
    pub async fn expire_sweep(&self) {
        let now = Instant::now();
        let (count, index) = (self.config.replica_count, self.config.replica_index);
        let mut expired_ids = Vec::new();
        self.subs
            .write()
            .await
            .retain(|(topic, callback), sub| {
                let expired = sub.lease_expires <= now;
                let keep = !expired || owner_of(&subscription_persist_id(topic, callback), count) != index;
                if expired {
                    expired_ids.push(subscription_persist_id(topic, callback));
                }
                keep
            });
        for id in expired_ids {
            if let Err(e) = self.store.delete_hub_subscription(&id).await {
                tracing::warn!("persisted subscription {id} removal failed: {e}");
            }
        }
    }

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
    async fn redeliver_pending(&self) {
        match self.store.load_pending_deliveries().await {
            Ok(pending) => {
                let now = verification::now_epoch();
                for (id, event_id, topic, callback, stored_secret, self_url, hub_url, enqueued_at) in pending {
                    // Age guard: entries younger than the full retry
                    // schedule (1+5+15s, plus slack) may be actively
                    // retried by another worker/replica right now;
                    // re-enqueueing would duplicate in-flight deliveries.
                    if now.saturating_sub(enqueued_at) < 60 {
                        continue;
                    }
                    let topic_static = match resolve_topic(&topic) {
                        Some(t) => t,
                        None => {
                            let _ = self.store.delete_delivery(&id).await;
                            continue;
                        }
                    };

                    // Only the owning replica redelivers.
                    if !self.owns(&topic, &callback) {
                        continue;
                    }
                    let content = match self.build_topic_content(topic_static).await {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::error!(event = event_id, "redelivery content build failed: {e:?}");
                            continue;
                        }
                    };
                    let secret = stored_secret.and_then(|s| self.crypto.decrypt(&s));
                    match self.queue.try_send(Delivery {
                        id: id.clone(),
                        topic: topic_static,
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

/// Random per-mutation event id (ties write -> publish -> fan-out).
pub fn random_event_id() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    let bytes: [u8; 8] = rng.random();
    crate::util::hex(&bytes)
}

/// §5.1 callback-host policy: allow-list of host suffixes. Empty list =
/// allow everything. Host extracted between "://" and the next '/' or ':'.
fn allowlist_ok(allowlist: &[String], callback: &str) -> bool {
    if allowlist.is_empty() {
        return true;
    }
    allowlist
        .iter()
        .any(|allowed| host_matches(callback, allowed))
}

fn host_matches(callback: &str, allowed_suffix: &str) -> bool {
    let host = callback
        .split("://")
        .nth(1)
        .unwrap_or(callback)
        .split(['/', ':'])
        .next()
        .unwrap_or("");
    let suffix = allowed_suffix.trim_start_matches('.');
    host == suffix || host.ends_with(&format!(".{suffix}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_topic_accepts_paths_and_absolute_urls() {
        assert_eq!(resolve_topic("/topics/data"), Some("/topics/data"));
        assert_eq!(
            resolve_topic("http://localhost:8000/topics/data"),
            Some("/topics/data")
        );
        assert_eq!(resolve_topic("http://other/topics/schema"), Some("/topics/schema"));
        assert_eq!(resolve_topic("/topics/nope"), None);
    }

    #[test]
    fn callback_allowlist_matches_suffixes_and_rejects_others() {
        let list = vec!["example.com".to_string()];
        assert!(allowlist_ok(&list, "https://demo.example.com/cb"));
        assert!(allowlist_ok(&list, "https://example.com/cb"));
        assert!(!allowlist_ok(&list, "https://evil.com/cb"));
        assert!(!allowlist_ok(&list, "http://notexample.com/cb"));
        assert!(allowlist_ok(&[], "http://anything/cb")); // empty = allow all
    }
}
