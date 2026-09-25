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

pub(crate) mod publish;
pub(crate) mod reload;
pub(crate) mod verification;
pub(crate) mod delivery;
pub(crate) mod crypto;
pub(crate) mod rate_limit;
pub(crate) mod metrics;

/// The topics this service publishes. Spec §1: a topic is an HTTP
/// resource URL -- served with full content at `GET /topics/...` and
/// advertised via Link headers (spec §4). With the open-hub policy,
/// third-party topics (any publisher's URL) are accepted too.
pub const TOPICS: [&str; 2] = ["/topics/data", "/topics/schema"];

/// Sanity cap on third-party topic content fetched at publish time.
pub const MAX_EXTERNAL_CONTENT_BYTES: usize = 16 * 1024 * 1024; // 16 MiB

/// How long a delivery's durable-log claim covers. Must exceed the
/// worst-case delivery duration (retry sleeps 1+5+15s plus up to 4
/// attempts x 10s HTTP timeout ≈ 61s) so the periodic replica scan can
/// never re-enqueue a delivery that is still being retried in-flight.
pub const CLAIM_SECS: u64 = 180;

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
/// A URL only matches when the topic path is the ENTIRE path component —
/// "http://x/topics/data" resolves, "http://x/other/topics/data" does not.
pub fn resolve_topic(topic: &str) -> Option<&'static str> {
    TOPICS
        .into_iter()
        .find(|&t| {
            topic == t
                || (topic.ends_with(t)
                    && url_is_scheme_authority(&topic[..topic.len() - t.len()]))
        })
        .map(|v| v as _)
}

/// True when `base` is exactly `scheme://authority` (no extra path), the
/// only prefix that leaves the topic path as the full path component.
fn url_is_scheme_authority(base: &str) -> bool {
    let authority = base
        .strip_prefix("http://")
        .or_else(|| base.strip_prefix("https://"));
    match authority {
        Some(authority) => !authority.is_empty() && !authority.contains('/'),
        None => false,
    }
}

#[derive(Clone)]
pub(crate) enum Intent {
    Subscribe { secret: Option<String>, lease: u64 },
    Unsubscribe,
}

/// Content of a topic at publish time (spec §7: the hub MUST send the
/// full contents of the topic URL, with a Content-Type matching the
/// topic's; payloads may be reduced to diffs only for Atom/RSS).
///
/// The body is `Bytes` so per-subscriber clones (one queued delivery
/// each) are O(1) reference-count bumps, not full-content copies.
#[derive(Clone)]
pub struct TopicContent {
    pub body: bytes::Bytes,
    /// String (not &'static) because third-party topics fetch their
    /// content type from the publisher's response at publish time.
    pub content_type: String,
}

/// §5.1 hub policy for a submitted hub.topic:
///   - canonical topics (ours) are always accepted;
///   - with the open-hub policy (SEMWEB_OPEN_HUB), any http(s) topic URL
///     is accepted too -- the hub then serves third-party publishers
///     (spec §1: "Any hub MAY implement its own policies on who can use
///     it"). Returns the accepted topic unchanged.
pub fn topic_allowed(topic: &str, open_hub: bool) -> Option<String> {
    if resolve_topic(topic).is_some() {
        return Some(topic.to_string());
    }
    if open_hub && (topic.starts_with("http://") || topic.starts_with("https://")) {
        return Some(topic.to_string());
    }
    None
}

/// Does a subscribed topic match a publishing topic? Canonical topics
/// match loosely (bare path == absolute URL); third-party topics match
/// exactly (both sides are the publisher's canonical rel=self URL).
pub(crate) fn topic_matches(subscribed: &str, publishing: &str) -> bool {
    if resolve_topic(subscribed).is_some() {
        return resolve_topic(publishing) == resolve_topic(subscribed);
    }
    subscribed == publishing
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
    topic: String,
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
    /// §4 fault tolerance: external hubs we advertise and notify on every
    /// mutation (e.g. a public hub like Superfeedr). Empty = only ours.
    pub external_hubs: Vec<String>,
    /// §5.1 policy: accept third-party topics (any publisher's URL) —
    /// turns this hub into a general-purpose hub for anyone's content.
    pub open_hub: bool,
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

    /// External hub URLs (absolute; ours excluded) for §4 advertise+notify.
    pub fn external_hubs(&self) -> Vec<String> {
        self.config
            .external_hubs
            .iter()
            .map(|h| h.trim_end_matches('/').to_string())
            .filter(|h| !h.is_empty() && h != &format!("{}/hub", self.config.public_url))
            .collect()
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
    /// topic (canonical or third-party): (callback, secret) pairs ready
    /// for fan-out. Matching is by the exact submitted topic string --
    /// third-party topics are subscribed with the publisher's canonical
    /// rel=self URL, so no normalization is possible (or needed).
    async fn owned_subs(&self, topic: &str) -> Vec<(String, Option<String>)> {
        let now = Instant::now();
        let (count, index) = (self.config.replica_count, self.config.replica_index);
        self.subs
            .read()
            .await
            .iter()
            .filter(|((sub_topic, callback), sub)| {
                topic_matches(sub_topic, topic)
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
        let canonical = topic_allowed(&topic, self.config.open_hub)
            .ok_or_else(|| HubError::UnknownTopic(format!("unknown topic: {topic}")))?;

        // §5.1 policy enforcement (hubs MAY reject callback/topic URLs):
        // HTTPS requirement when secrets are used, and the optional
        // callback-host allowlist. Third-party topics (open hub) must
        // respect the same allowlist — the publish-time fetch is an SSRF
        // surface identical to callback delivery.
        if resolve_topic(&topic).is_none()
            && !allowlist_ok(&self.config.callback_allowlist, &topic)
        {
            return Err(HubError::BadRequest(
                "topic URL host is not in the configured allowlist".into(),
            ));
        }
        if secret.is_some()
            && self.config.require_https_callbacks
            && !callback.starts_with("https://")
        {
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
    /// Unsubscribe requests are also rate-limited (same per-callback
    /// bucket as subscribe — §5.1 covers both as subscription requests).
    pub async fn unsubscribe(self: Arc<Self>, topic: String, callback: String) -> Result<(), HubError> {
        let canonical = topic_allowed(&topic, self.config.open_hub)
            .ok_or_else(|| HubError::UnknownTopic(format!("unknown topic: {topic}")))?;
        if !self.rate.check(&callback).await {
            self.metrics.rate_limited.fetch_add(1, Ordering::Relaxed);
            return Err(HubError::TooManyRequests);
        }
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
}

/// Random per-mutation event id (ties write -> publish -> fan-out).
pub fn random_event_id() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    let bytes: [u8; 8] = rng.random();
    crate::util::hex(&bytes)
}

/// §5.1 callback-host policy: allow-list of host suffixes. Empty list =
/// allow everything. Used for subscriber callbacks AND (when the open-hub
/// policy is enabled) for third-party topic URLs — the same lever gates
/// both SSRF surfaces (callback delivery and publish-time fetch).
pub(crate) fn allowlist_ok(allowlist: &[String], url: &str) -> bool {
    if allowlist.is_empty() {
        return true;
    }
    allowlist
        .iter()
        .any(|allowed| host_matches(url, allowed))
}

fn host_matches(url: &str, allowed_suffix: &str) -> bool {
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
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
        // extra path segments before the topic path do NOT resolve
        assert_eq!(resolve_topic("http://x/other/topics/data"), None);
        assert_eq!(resolve_topic("http://x/a/b/topics/data"), None);
        assert_eq!(resolve_topic("/other/topics/data"), None);
        // query strings are not part of a topic URL match
        assert_eq!(resolve_topic("http://x/topics/data?x=1"), None);
    }

    #[test]
    fn topic_policy_respects_the_open_hub_flag() {
        // §5.1: "Any hub MAY implement its own policies on who can use it"
        // canonical topics always accepted
        assert!(topic_allowed("/topics/data", false).is_some());
        assert!(topic_allowed("http://x/topics/data", false).is_some());
        // third-party topics only under the open-hub policy
        assert!(topic_allowed("https://other.example/feed.xml", false).is_none());
        assert!(topic_allowed("https://other.example/feed.xml", true).is_some());
        // and never non-URL garbage
        assert!(topic_allowed("not-a-url", true).is_none());
    }

    #[test]
    fn topic_matching_canonical_vs_third_party() {
        assert!(topic_matches("/topics/data", "http://localhost:8484/topics/data"));
        assert!(topic_matches("http://localhost:8484/topics/schema", "/topics/schema"));
        // third-party: exact match only
        assert!(topic_matches("https://a.example/feed", "https://a.example/feed"));
        assert!(!topic_matches("https://a.example/feed", "https://b.example/feed"));
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