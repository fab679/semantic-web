//! Store layer: SPARQL 1.1 Protocol client for a standalone Oxigraph
//! server (or any conforming SPARQL endpoint).
//!
//! The store-boundary module -- the only code that talks to the store
//! over the wire. Exclusively standard interfaces:
//!
//! - Queries   : GET {query}?query=... with Accept:
//!               application/sparql-results+json
//! - Updates   : POST {update} with Content-Type: application/sparql-update
//! - Seed load : Graph Store Protocol POST {store}?default (the `?default`
//!               is load-bearing: a bare POST to /store is accepted with
//!               201 by oxigraph 0.5.10 but lands in a server-generated
//!               named graph that /query never sees)
//!
//! Sub-modules:
//! - `fragments`    : Triple Pattern Fragment lookup + pagination cursor
//! - `cardinality`  : maintained per-class/per-predicate counters
//! - `persistence`  : hub subscription/delivery durability (named graphs)
//!
//! Raw SPARQL bindings are returned; JSON-LD compaction happens in
//! `crate::jsonld` so the wire data and the presentation stay separable.

mod cardinality;
pub mod fragments;
pub mod persistence;

pub use cardinality::Cardinality;
#[allow(unused_imports)]
pub use fragments::{Cursor, FragmentPage};

#[allow(unused_imports)]
pub use persistence::{HUB_DELIVERIES_GRAPH, HUB_GRAPH, SHAPES_GRAPH};

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};

#[derive(Debug)]
pub struct StoreError(pub String);

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "store error: {}", self.0)
    }
}

impl std::error::Error for StoreError {}

impl From<reqwest::Error> for StoreError {
    fn from(e: reqwest::Error) -> Self {
        StoreError(format!("http: {e}"))
    }
}

/// Raw class/predicate URIs currently in use (live self-description data;
/// callers compact for display and derive namespaces for /context.jsonld).
pub struct SchemaInfo {
    pub class_uris: Vec<String>,
    pub predicate_uris: Vec<String>,
}

/// The SPARQL 1.1 Protocol client. Constructed once (see state.rs) and
/// shared through Arc.
pub struct SparqlStore {
    pub query_url: String,
    pub update_url: String,
    pub graph_store_url: String,
    http: reqwest::Client,
}

impl SparqlStore {
    pub fn new(query_url: String, update_url: String) -> Self {
        // Graph Store endpoint conventionally sits next to /update as /store.
        let graph_store_url = match update_url.rsplit_once('/') {
            Some((base, _)) => format!("{base}/store"),
            None => format!("{update_url}/store"),
        };
        Self {
            query_url,
            update_url,
            graph_store_url,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("reqwest client"),
        }
    }

    /// Run a SELECT and return the raw bindings list from the
    /// SPARQL JSON results document.
    pub(crate) async fn query_bindings(&self, sparql: &str) -> Result<Vec<Value>, StoreError> {
        let resp = self
            .http
            .get(&self.query_url)
            .query(&[("query", sparql)])
            .header("Accept", "application/sparql-results+json")
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(StoreError(format!("query {} -> {}", status, sparql)));
        }
        let doc: Value = resp.json().await?;
        Ok(doc
            .pointer("/results/bindings")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    /// Run a SPARQL UPDATE.
    pub(crate) async fn update(&self, sparql: &str) -> Result<(), StoreError> {
        let resp = self
            .http
            .post(&self.update_url)
            .header("Content-Type", "application/sparql-update")
            .body(sparql.to_string())
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(StoreError(format!("update -> {status}")));
        }
        Ok(())
    }

    /// Load a Turtle seed file through the Graph Store Protocol.
    ///
    /// Retries while the store is still booting (docker-compose starts
    /// containers concurrently and the Oxigraph image is distroless, so
    /// compose cannot healthcheck-gate it). Idempotent for a fixed file:
    /// RDF is set semantics, so re-loading duplicates nothing.
    pub async fn load_seed(&self, path: &Path, graph: Option<&str>) -> Result<(), StoreError> {
        let body = std::fs::read(path)
            .map_err(|e| StoreError(format!("read seed {:?}: {e}", path.display())))?;
        // `?default` targets the default graph; `?graph=<uri>` a named one.
        // (A bare POST to /store lands in a server-generated named graph.)
        let query: [(&str, &str); 1] = match graph {
            Some(g) => [("graph", g)],
            None => [("default", "")],
        };
        for attempt in 0..10 {
            match self
                .http
                .post(&self.graph_store_url)
                .query(&query)
                .header("Content-Type", "text/turtle")
                .body(body.clone())
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => return Ok(()),
                other => {
                    if attempt == 9 {
                        return Err(other
                            .err()
                            .map(|e| StoreError(format!("seed load: {e}")))
                            .unwrap_or_else(|| StoreError("seed load failed".into())));
                    }
                    tracing::warn!("seed load attempt {attempt} failed, retrying");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }
        unreachable!()
    }

    /// Clear a named graph. Used before re-loading blank-node-bearing
    /// files (SHACL shapes): blank nodes get fresh labels on every load,
    /// so re-loading without clearing would duplicate structures.
    pub async fn clear_graph(&self, graph: &str) -> Result<(), StoreError> {
        self.update(&format!(
            "DELETE WHERE {{ GRAPH <{graph}> {{ ?s ?p ?o }} }}"
        ))
        .await
    }

    /// Live self-description data: two cheap SPARQL queries against the
    /// *current* store state, every call. No build step, nothing to
    /// regenerate -- the description is always exactly as current as the
    /// data (the "live catalog" piece of the architecture).
    pub async fn describe_schema(&self) -> Result<SchemaInfo, StoreError> {
        let class_rows = self
            .query_bindings("SELECT DISTINCT ?class WHERE { ?s a ?class }")
            .await?;
        let pred_rows = self
            .query_bindings("SELECT DISTINCT ?p WHERE { ?s ?p ?o }")
            .await?;

        let mut class_uris: Vec<String> = class_rows
            .iter()
            .filter_map(|r| r.pointer("/class/value").and_then(Value::as_str))
            .map(str::to_string)
            .collect();
        class_uris.sort();
        class_uris.dedup();

        let mut predicate_uris: Vec<String> = pred_rows
            .iter()
            .filter_map(|r| r.pointer("/p/value").and_then(Value::as_str))
            .map(str::to_string)
            .collect();
        predicate_uris.sort();
        predicate_uris.dedup();

        Ok(SchemaInfo {
            class_uris,
            predicate_uris,
        })
    }

    /// Human-readable descriptions for terms, pulled live from
    /// `rdfs:comment` and `skos:definition` (one UNION query). This is
    /// the grounding text the agent manifest carries: agents plan
    /// against what terms *mean*, not just which exist.
    pub async fn descriptions(&self) -> Result<HashMap<String, String>, StoreError> {
        let rows = self
            .query_bindings(
                "SELECT ?term ?text WHERE { \
                 { ?term <http://www.w3.org/2000/01/rdf-schema#comment> ?text } \
                 UNION \
                 { ?term <http://www.w3.org/2004/02/skos/core#definition> ?text } \
                 }",
            )
            .await?;
        Ok(rows
            .iter()
            .filter_map(|r| {
                Some((
                    r.pointer("/term/value")?.as_str()?.to_string(),
                    r.pointer("/text/value")?.as_str()?.to_string(),
                ))
            })
            .collect())
    }

    /// Health probe: an empty ASK query. Success means the store is
    /// reachable and answering SPARQL.
    pub async fn ping(&self) -> Result<(), StoreError> {
        self.query_bindings("ASK {}").await.map(|_| ())
    }

    /// Live per-class instance counts (GROUP BY; used by the agent
    /// manifest, which never inherits counter drift).
    pub async fn class_counts(&self) -> Result<HashMap<String, u64>, StoreError> {
        let rows = self
            .query_bindings(
                "SELECT ?class (COUNT(DISTINCT ?s) AS ?c) WHERE { ?s a ?class } GROUP BY ?class",
            )
            .await?;
        Ok(binding_counts(&rows, "class"))
    }

    /// Live per-predicate triple counts (GROUP BY; used by the agent
    /// manifest).
    pub async fn predicate_counts(&self) -> Result<HashMap<String, u64>, StoreError> {
        let rows = self
            .query_bindings("SELECT ?p (COUNT(*) AS ?c) WHERE { ?s ?p ?o } GROUP BY ?p")
            .await?;
        Ok(binding_counts(&rows, "p"))
    }

    /// Forward a raw SPARQL query and return (status, content-type, body)
    /// — the read-only passthrough behind `GET /sparql`. Read-only by
    /// construction: this hits the store's /query endpoint, which only
    /// executes SPARQL Query forms, never updates.
    pub async fn query_raw(
        &self,
        query: &str,
        accept: &str,
    ) -> Result<(u16, String, String), StoreError> {
        let resp = self
            .http
            .get(&self.query_url)
            .query(&[("query", query)])
            .header("Accept", accept)
            .send()
            .await?;
        let status = resp.status().as_u16();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/sparql-results+json")
            .to_string();
        let body = resp.text().await?;
        Ok((status, content_type, body))
    }

    /// All triples in the default graph, raw bindings (used for the
    /// /topics/data topic content, which WebSub requires to be the *full*
    /// topic contents).
    pub async fn all_bindings(&self) -> Result<Vec<Value>, StoreError> {
        self.query_bindings("SELECT ?s ?p ?o WHERE { ?s ?p ?o }")
            .await
    }

    /// Add one triple (SPARQL UPDATE) and return its raw binding (?s ?p
    /// ?o), from which callers build the compacted notification metadata.
    pub async fn insert_triple(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        graph: Option<&str>,
    ) -> Result<Value, StoreError> {
        let obj = if object.starts_with("http://") || object.starts_with("https://") {
            format!("<{object}>")
        } else {
            escape_literal(object)
        };
        let body = match graph {
            Some(g) => format!("GRAPH <{g}> {{ <{subject}> <{predicate}> {obj} }}"),
            None => format!("<{subject}> <{predicate}> {obj}"),
        };
        let sparql = format!("INSERT DATA {{ {body} }}");
        self.update(&sparql).await.map_err(StoreError::from)?;

        // Rebuild the raw binding of the inserted triple so change-event
        // metadata matches what the fragments stream emits.
        let obj_term = if object.starts_with("http://") || object.starts_with("https://") {
            json!({ "type": "uri", "value": object })
        } else {
            json!({ "type": "literal", "value": object })
        };
        Ok(json!({
            "s": { "type": "uri", "value": subject },
            "p": { "type": "uri", "value": predicate },
            "o": obj_term,
        }))
    }

}

/// Serialize a plain string as a SPARQL literal with basic escaping
/// (escape backslash, quote and newline).
pub(crate) fn escape_literal(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    format!("\"{escaped}\"")
}

/// Map COUNT query bindings ({var: {value}, c: {value}}) to (var -> count).
pub(crate) fn binding_counts(rows: &[Value], var: &str) -> HashMap<String, u64> {
    rows.iter()
        .filter_map(|r| {
            Some((
                r.pointer(&format!("/{var}/value"))?.as_str()?.to_string(),
                r.pointer("/c/value")?.as_str()?.parse().ok()?,
            ))
        })
        .collect()
}