//! Maintained per-class/per-predicate/total triple counts.
//!
//! Loaded once at startup with aggregate SPARQL queries, then updated
//! in-process on every write. This takes the cardinality work off the
//! read path: fragment requests for patterns the counters cover get
//! their `count_estimate` for free instead of paying a COUNT round-trip.
//!
//! Honest trade-off: counters are exact for writes through this service
//! and approximate if data changes out-of-band (e.g. direct SPARQL
//! UPDATE against the store) -- state.rs carries the periodic reload
//! interval for that case, and the agent manifest always computes its
//! numbers with live GROUP BY queries, so it never inherits drift.

use std::collections::HashMap;

use serde_json::Value;

use super::{SparqlStore, StoreError};

#[derive(Default)]
pub struct Cardinality {
    classes: std::sync::RwLock<HashMap<String, u64>>,
    predicates: std::sync::RwLock<HashMap<String, u64>>,
    /// Per-subject outgoing-triple degrees (for `?subject`-only patterns).
    subjects: std::sync::RwLock<HashMap<String, u64>>,
    total_triples: std::sync::atomic::AtomicU64,
}

impl Cardinality {
    /// Seed the counters from the current store state (GROUP BY queries)
    /// plus a total-triples count.
    pub async fn load_initial(&self, store: &SparqlStore) -> Result<(), StoreError> {
        let class_rows = store
            .query_bindings(
                "SELECT ?class (COUNT(DISTINCT ?s) AS ?c) WHERE { ?s a ?class } GROUP BY ?class",
            )
            .await?;
        let pred_rows = store
            .query_bindings("SELECT ?p (COUNT(*) AS ?c) WHERE { ?s ?p ?o } GROUP BY ?p")
            .await?;
        let subj_rows = store
            .query_bindings("SELECT ?s (COUNT(*) AS ?c) WHERE { ?s ?p ?o } GROUP BY ?s")
            .await?;
        let total_rows = store
            .query_bindings("SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }")
            .await?;

        *self.classes.write().unwrap() = counts_from(&class_rows, "class");
        *self.predicates.write().unwrap() = counts_from(&pred_rows, "p");
        *self.subjects.write().unwrap() = counts_from(&subj_rows, "s");
        self.total_triples.store(
            total_rows
                .first()
                .and_then(|r| r.pointer("/c/value").and_then(Value::as_str))
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            std::sync::atomic::Ordering::Relaxed,
        );
        Ok(())
    }

    /// Update counters for one inserted triple (s, p, o): the predicate
    /// always gains one; the class gains an instance iff the predicate is
    /// rdf:type; the subject's degree and the total always grow.
    pub fn record_insert(
        &self,
        subject_uri: &str,
        predicate_uri: &str,
        object_uri: Option<&str>,
        rdf_type_uri: &str,
    ) {
        if predicate_uri == rdf_type_uri {
            if let Some(class) = object_uri {
                *self.classes.write().unwrap().entry(class.to_string()).or_insert(0) += 1;
            }
        }
        *self
            .predicates
            .write()
            .unwrap()
            .entry(predicate_uri.to_string())
            .or_insert(0) += 1;
        *self
            .subjects
            .write()
            .unwrap()
            .entry(subject_uri.to_string())
            .or_insert(0) += 1;
        self.total_triples
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Estimate for a fragment pattern, when the counters cover it.
    /// None => caller must fall back to a server COUNT.
    pub fn estimate_for(
        &self,
        subject: Option<&str>,
        predicate: Option<&str>,
        object: Option<&str>,
    ) -> Option<u64> {
        match (subject, predicate, object) {
            // (?s ?p ?o): everything.
            (None, None, None) => {
                Some(self.total_triples.load(std::sync::atomic::Ordering::Relaxed))
            }
            // (?s <p> ?o): predicate cardinality.
            (None, Some(p), None) => self.predicates.read().unwrap().get(p).copied(),
            // (<s> ?p ?o): subject degree.
            (Some(s), None, None) => self.subjects.read().unwrap().get(s).copied(),
            _ => None,
        }
    }

    /// Whether the predicate has been seen at all (used for O(1)
    /// new-ontology-term detection on the write path).
    pub fn has_predicate(&self, uri: &str) -> bool {
        self.predicates.read().unwrap().contains_key(uri)
    }

    pub fn has_class(&self, uri: &str) -> bool {
        self.classes.read().unwrap().contains_key(uri)
    }
}

fn counts_from(rows: &[Value], var: &str) -> HashMap<String, u64> {
    rows.iter()
        .filter_map(|r| {
            Some((
                r.pointer(&format!("/{var}/value"))?.as_str()?.to_string(),
                r.pointer("/c/value")?.as_str()?.parse().ok()?,
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

    #[test]
    fn record_insert_tracks_class_predicate_subject_and_total() {
        let c = Cardinality::default();
        c.record_insert("http://s/1", RDF_TYPE, Some("http://C"), RDF_TYPE);
        c.record_insert("http://s/1", "http://p", None, RDF_TYPE);
        assert_eq!(c.estimate_for(None, Some("http://p"), None), Some(1));
        assert_eq!(c.estimate_for(Some("http://s/1"), None, None), Some(2));
        assert_eq!(c.estimate_for(None, None, None), Some(2));
        assert!(c.has_class("http://C"));
        assert!(!c.has_predicate("http://q"));
        // a second type triple on the same class bumps instances (counted
        // under the rdf:type predicate), the total reaches 3
        c.record_insert("http://s/2", RDF_TYPE, Some("http://C"), RDF_TYPE);
        assert_eq!(c.estimate_for(None, Some(RDF_TYPE), None), Some(2));
    }

    #[test]
    fn uncovered_patterns_return_none() {
        let c = Cardinality::default();
        assert_eq!(c.estimate_for(Some("s"), Some("p"), None), None);
        assert_eq!(c.estimate_for(None, Some("p"), Some("o")), None);
    }
}
