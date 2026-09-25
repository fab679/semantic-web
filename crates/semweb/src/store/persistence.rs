//! Hub durability in the store, via named graphs.
//!
//! - Subscriptions persist in `HUB_GRAPH` so they survive service
//!   restarts; lease deadlines are unix-epoch seconds; secrets are
//!   stored already-encrypted by the caller when a key is configured
//!   (see hub/crypto.rs -- the store never sees plaintext decisions).
//! - Pending deliveries persist in `HUB_DELIVERIES_GRAPH` as a durable
//!   delivery log: an event is written before it is queued and deleted
//!   once delivered (or exhaustively failed), so a crash mid-flight
//!   redelivers on startup instead of silently dropping the notification.
//! - SHACL shapes load into `SHAPES_GRAPH` and are read live by the
//!   agent manifest.
//!
//! All three graphs live inside the same store that holds the data, so
//! durability needs no extra infrastructure.

use std::collections::HashMap;

use serde_json::Value;

use super::{escape_literal, SparqlStore, StoreError};

/// Named graph where the hub persists subscriptions.
pub const HUB_GRAPH: &str = "http://semweb.dev/graph/hub/subscriptions";
/// Named graph holding the durable delivery log (in-flight events).
pub const HUB_DELIVERIES_GRAPH: &str = "http://semweb.dev/graph/hub/deliveries";
/// Named graph where optional SHACL shapes are loaded from
/// SEMWEB_SHACL_PATH and read by the agent manifest.
pub const SHAPES_GRAPH: &str = "http://semweb.dev/graph/shapes";
/// Vocabulary for hub-internal persistence resources.
const NS: &str = "http://semweb.dev/ns/hub#";
/// SHACL vocabulary.
const SH: &str = "http://www.w3.org/ns/shacl#";
const HUB_SUBJ_PREFIX: &str = "urn:semweb:sub:";
const DELIVERY_SUBJ_PREFIX: &str = "urn:semweb:delivery:";

/// One durable-log delivery record (see Hub::publish).
pub struct DeliveryRecord<'a> {
    pub id: &'a str,
    pub event_id: &'a str,
    pub topic: &'a str,
    pub callback: &'a str,
    /// Already serialized (encrypted when a key is configured).
    pub secret: Option<&'a str>,
    pub self_url: &'a str,
    pub hub_url: &'a str,
}

    /// Record a delivery in the durable log BEFORE it is queued. When the
    /// service crashes mid-flight, startup re-enqueues everything still
    /// in this graph.
#[derive(Clone, Debug)]
pub struct Shape {
    #[allow(dead_code)] // the grouping key; kept for completeness
    pub target_class: String,
    pub path: String,
    pub min_count: Option<u64>,
    pub max_count: Option<u64>,
    pub datatype: Option<String>,
}

/// Graph URIs the service manages itself (see HUB_GRAPH,
/// HUB_DELIVERIES_GRAPH, SHAPES_GRAPH). The write path rejects these so
/// hub state (subscriptions, delivery log, shapes) cannot be forged
/// through the public write API — bypassing the §5.3 verification
/// handshake.
pub fn is_reserved_graph(graph: &str) -> bool {
    graph.starts_with("http://semweb.dev/graph/")
}

/// Stable identity for a (topic, callback) subscription: the first 16
/// bytes of sha256(topic \0 callback), hex. Used as the persistence key,
/// for replica ownership (see hub::mod) and as the durable delivery-log key.
pub fn subscription_persist_id(topic: &str, callback: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("{topic}\0{callback}").as_bytes());
    hex(&digest.as_slice()[..16])
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Owner replica of a subscription id under `replica_count` shards:
/// consistent hashing keeps a (topic, callback) on exactly one replica.
pub fn owner_of(id: &str, replica_count: u64) -> u64 {
    let first_bytes: Vec<u8> = (0..16)
        .filter_map(|i| u8::from_str_radix(&id[i * 2..i * 2 + 2], 16).ok())
        .collect();
    let mut v: u64 = 0;
    for b in first_bytes.iter().take(8) {
        v = (v << 8) | *b as u64;
    }
    if replica_count == 0 { 0 } else { v % replica_count }
}

impl SparqlStore {
    /// Persist one subscription. The id is the stable sha256-derived key
    /// of (topic, callback); `secret` arrives already serialized
    /// (encrypted when SEMWEB_SECRET_KEY is set -- the caller encrypts,
    /// this module never sees plaintext).
    pub async fn persist_hub_subscription(
        &self,
        id: &str,
        topic: &str,
        callback: &str,
        secret: Option<&str>,
        expires_epoch: u64,
    ) -> Result<(), StoreError> {
        let secret_triple = match secret {
            Some(s) => {
                format!("<{HUB_SUBJ_PREFIX}{id}> <{NS}secret> {} .\n", escape_literal(s))
            }
            None => String::new(),
        };
        self.update(&format!(
            "INSERT DATA {{ GRAPH <{HUB_GRAPH}> {{ \
             <{HUB_SUBJ_PREFIX}{id}> <{NS}topic> {} . \
             <{HUB_SUBJ_PREFIX}{id}> <{NS}callback> {} . \
             {secret_triple}\
             <{HUB_SUBJ_PREFIX}{id}> <{NS}leaseExpires> {expires_epoch} }}}}",
            escape_literal(topic),
            escape_literal(callback),
        ))
        .await
    }

    /// Remove one persisted subscription by id.
    pub async fn delete_hub_subscription(&self, id: &str) -> Result<(), StoreError> {
        self.update(&format!(
            "DELETE WHERE {{ GRAPH <{HUB_GRAPH}> {{ <{HUB_SUBJ_PREFIX}{id}> ?p ?o }} }}"
        ))
        .await
    }

    /// Load persisted subscriptions:
    /// (topic, callback, serialized-secret, expires_epoch).
    /// Expired entries are filtered by the caller (§5.3).
    pub async fn load_hub_subscriptions(
        &self,
    ) -> Result<Vec<(String, String, Option<String>, u64)>, StoreError> {
        let rows = self
            .query_bindings(&format!(
                "SELECT ?topic ?callback ?secret ?expires WHERE {{ \
                 GRAPH <{HUB_GRAPH}> {{ \
                 ?sub <{NS}topic> ?topic ; \
                 <{NS}callback> ?callback ; \
                 <{NS}leaseExpires> ?expires . \
                 OPTIONAL {{ ?sub <{NS}secret> ?secret }} \
                 }}}}"
            ))
            .await?;
        Ok(rows
            .iter()
            .filter_map(|r| {
                Some((
                    r.pointer("/topic/value")?.as_str()?.to_string(),
                    r.pointer("/callback/value")?.as_str()?.to_string(),
                    r.pointer("/secret/value")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    r.pointer("/expires/value")?.as_str()?.parse().ok()?,
                ))
            })
            .collect())
    }

    // ------------------------------------------------------------------
    // Durable delivery log (see hub::mod for the enqueue/ack lifecycle)
    // ------------------------------------------------------------------

    /// Record a delivery in the durable log BEFORE it is queued. When the
    /// service crashes mid-flight, startup re-enqueues everything still
    /// in this graph.
    pub async fn persist_delivery(
        &self,
        rec: &DeliveryRecord<'_>,
    ) -> Result<(), StoreError> {
        let DeliveryRecord {
            id,
            event_id,
            topic,
            callback,
            secret,
            self_url,
            hub_url,
        } = rec;
        let secret_triple = match secret {
            Some(s) => format!("<{DELIVERY_SUBJ_PREFIX}{id}> <{NS}secret> {} .\n", escape_literal(s)),
            None => String::new(),
        };
        self.update(&format!(
            "INSERT DATA {{ GRAPH <{HUB_DELIVERIES_GRAPH}> {{ \
             <{DELIVERY_SUBJ_PREFIX}{id}> <{NS}event> {} . \
             <{DELIVERY_SUBJ_PREFIX}{id}> <{NS}topic> {} . \
             <{DELIVERY_SUBJ_PREFIX}{id}> <{NS}callback> {} . \
             <{DELIVERY_SUBJ_PREFIX}{id}> <{NS}selfUrl> {} . \
             <{DELIVERY_SUBJ_PREFIX}{id}> <{NS}hubUrl> {} . \
             {secret_triple}\
             <{DELIVERY_SUBJ_PREFIX}{id}> <{NS}enqueuedAt> {} }}}}",
            escape_literal(event_id),
            escape_literal(topic),
            escape_literal(callback),
            escape_literal(self_url),
            escape_literal(hub_url),
            now_epoch(),
        ))
        .await
    }

    /// Claim a logged delivery before working on it: the claim is an
    /// absolute deadline after which the entry is fair game again (the
    /// claiming process crashed mid-flight). A live claim prevents other
    /// workers/replicas from re-enqueueing the same in-flight delivery
    /// during its retry window -- duplicates only happen if the claimer
    /// crashes before acking, which is exactly the at-least-once case.
    pub async fn claim_delivery(&self, id: &str, until_epoch: u64) -> Result<(), StoreError> {
        self.update(&format!(
            "INSERT DATA {{ GRAPH <{HUB_DELIVERIES_GRAPH}> {{ \
             <{DELIVERY_SUBJ_PREFIX}{id}> <{NS}claimedUntil> {until_epoch} }}}}"
        ))
        .await
    }

    /// Ack (delete) one logged delivery.
    pub async fn delete_delivery(&self, id: &str) -> Result<(), StoreError> {
        self.update(&format!(
            "DELETE WHERE {{ GRAPH <{HUB_DELIVERIES_GRAPH}> {{ <{DELIVERY_SUBJ_PREFIX}{id}> ?p ?o }} }}"
        ))
        .await
    }

    /// Load pending (unacked) deliveries for crash recovery:
    /// (id, event_id, topic, callback, secret, self_url, hub_url, enqueued_at).
    pub async fn load_pending_deliveries(
        &self,
    ) -> Result<
        Vec<(
            String, // id
            String, // event_id
            String, // topic
            String, // callback
            Option<String>, // secret (serialized)
            String, // self_url
            String, // hub_url
            u64,    // enqueued_at
            u64,    // claimed_until (0 = unclaimed)
        )>,
        StoreError,
    > {
        let rows = self
            .query_bindings(&format!(
                "SELECT ?id ?event ?topic ?callback ?secret ?selfUrl ?hubUrl ?at ?claimed WHERE {{ \
                 GRAPH <{HUB_DELIVERIES_GRAPH}> {{ \
                 ?d <{NS}event> ?event ; \
                 <{NS}topic> ?topic ; \
                 <{NS}callback> ?callback ; \
                 <{NS}selfUrl> ?selfUrl ; \
                 <{NS}hubUrl> ?hubUrl ; \
                 <{NS}enqueuedAt> ?at . \
                 BIND(STRAFTER(STR(?d), \"{DELIVERY_SUBJ_PREFIX}\") AS ?id) \
                 OPTIONAL {{ ?d <{NS}secret> ?secret }} \
                 OPTIONAL {{ ?d <{NS}claimedUntil> ?claimed }} \
                 }}}}"
            ))
            .await?;
        Ok(rows
            .iter()
            .filter_map(|r| {
                Some((
                    r.pointer("/id/value")?.as_str()?.to_string(),
                    r.pointer("/event/value")?.as_str()?.to_string(),
                    r.pointer("/topic/value")?.as_str()?.to_string(),
                    r.pointer("/callback/value")?.as_str()?.to_string(),
                    r.pointer("/secret/value")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    r.pointer("/selfUrl/value")?.as_str()?.to_string(),
                    r.pointer("/hubUrl/value")?.as_str()?.to_string(),
                    r.pointer("/at/value")?.as_str()?.parse().ok()?,
                    r.pointer("/claimed/value")
                        .and_then(Value::as_str)
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0),
                ))
            })
            .collect())
    }

    // ------------------------------------------------------------------
    // SHACL shapes (agent manifest)
    // ------------------------------------------------------------------

    /// SHACL property shapes for target classes, read live from the
    /// shapes graph. Only shapes that declare a targetClass and property
    /// shapes are returned, grouped by target class.
    pub async fn shacl_shapes(&self) -> Result<HashMap<String, Vec<Shape>>, StoreError> {
        let rows = self
            .query_bindings(&format!(
                "SELECT ?class ?path ?min ?max ?datatype WHERE {{ \
                 GRAPH <{SHAPES_GRAPH}> {{ \
                 ?shape <{SH}targetClass> ?class ; \
                 <{SH}property> ?prop . \
                 ?prop <{SH}path> ?path . \
                 OPTIONAL {{ ?prop <{SH}minCount> ?min }} \
                 OPTIONAL {{ ?prop <{SH}maxCount> ?max }} \
                 OPTIONAL {{ ?prop <{SH}datatype> ?datatype }} \
                 }}}}"
            ))
            .await?;
        let mut shapes: HashMap<String, Vec<Shape>> = HashMap::new();
        for r in &rows {
            let (class, path) = match (
                r.pointer("/class/value").and_then(Value::as_str),
                r.pointer("/path/value").and_then(Value::as_str),
            ) {
                (Some(c), Some(p)) => (c.to_string(), p.to_string()),
                _ => continue,
            };
            shapes.entry(class.clone()).or_default().push(Shape {
                target_class: class,
                path,
                min_count: r
                    .pointer("/min/value")
                    .and_then(Value::as_str)
                    .and_then(|v| v.parse().ok()),
                max_count: r
                    .pointer("/max/value")
                    .and_then(Value::as_str)
                    .and_then(|v| v.parse().ok()),
                datatype: r
                    .pointer("/datatype/value")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            });
        }
        Ok(shapes)
    }
}

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_reserved_graph_matches_the_service_namespace() {
        assert!(is_reserved_graph(HUB_GRAPH));
        assert!(is_reserved_graph(HUB_DELIVERIES_GRAPH));
        assert!(is_reserved_graph(SHAPES_GRAPH));
        assert!(is_reserved_graph("http://semweb.dev/graph/other"));
        assert!(!is_reserved_graph("http://example.org/graphs/tenant-a"));
        assert!(!is_reserved_graph("http://semweb.dev/graphs/lookalike"));
    }

    #[test]
    fn subscription_id_is_stable_and_distinct() {
        let a = subscription_persist_id("/topics/data", "http://c/1");
        let b = subscription_persist_id("/topics/data", "http://c/b");
        assert_eq!(a.len(), 32);
        assert_eq!(a, subscription_persist_id("/topics/data", "http://c/1"));
        assert_ne!(a, b);
        assert_ne!(a, subscription_persist_id("/topics/schema", "http://c/1"));
        assert_ne!(a, subscription_persist_id("/topics/data", "http://c/1x"));
    }

    #[test]
    fn owner_of_shards_by_callback() {
        // consistent hashing: every id lands in [0, count) and a given id
        // always lands on the same shard
        let id = subscription_persist_id("/topics/data", "http://callback/1");
        assert_eq!(owner_of(&id, 2), owner_of(&id, 2));
        let owners: std::collections::HashSet<u64> =
            (0..50u32).map(|i| {
                let cb = format!("http://callback/{i}");
                owner_of(&subscription_persist_id("/topics/data", &cb), 2)
            }).collect();
        assert_eq!(owners, [0, 1].into_iter().collect::<std::collections::HashSet<u64>>());
    }
}
