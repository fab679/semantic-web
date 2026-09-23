//! Triple Pattern Fragment lookup + the pagination cursor.
//!
//! Pagination: `offset` is the legacy mechanism; `after` (a cursor of
//! the last-seen triple) is the production mechanism — OFFSET degrades
//! linearly while the cursor seeks. Results are ordered by the string
//! form of (s, p, o) so both mechanisms are stable, and the cursor
//! filter re-uses those string forms (SPARQL `<`/`>` are not defined
//! for IRIs, so we compare STR() values).
//!
//! `total`: served from the maintained cardinality counters when they
//! cover the pattern, else an exact server COUNT.


use base64::Engine as _;
use serde_json::Value;

use super::{escape_literal, Cardinality, SparqlStore, StoreError};

pub struct FragmentPage {
    /// Raw SPARQL JSON bindings (?s ?p ?o) for this page.
    pub bindings: Vec<Value>,
    /// Exact total matching the pattern (server-backed COUNT), or a
    /// maintained cardinality estimate when one covers the pattern.
    pub total: u64,
    pub has_more: bool,
}

/// A pagination cursor: the string forms of the last-seen triple
/// (STR(subject), STR(predicate), STR(object)), base64(JSON).
#[derive(Clone, Debug)]
pub struct Cursor {
    pub s: String,
    pub p: String,
    pub o: String,
}

impl Cursor {
    /// The positional "after this triple" filter, over the STR() forms
    /// bound as ?sv ?pv ?ov by pattern_fragment.
    pub(crate) fn filter_expression(&self) -> String {
        let (s, p, o) = (
            escape_literal(&self.s),
            escape_literal(&self.p),
            escape_literal(&self.o),
        );
        format!("FILTER(?sv > {s} || (?sv = {s} && (?pv > {p} || (?pv = {p} && ?ov > {o}))))")
    }

    /// base64(json([s, p, o])) — opaque to clients.
    pub fn encode(&self) -> String {
        let doc = serde_json::to_string(&[&self.s, &self.p, &self.o]).expect("cursor json");
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(doc)
    }

    pub fn decode(encoded: &str) -> Option<Cursor> {
        let doc = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .ok()?;
        let arr: [String; 3] = serde_json::from_str(std::str::from_utf8(&doc).ok()?).ok()?;
        Some(Cursor {
            s: arr[0].clone(),
            p: arr[1].clone(),
            o: arr[2].clone(),
        })
    }
}

/// Substitute concrete terms into the pattern positionally; wildcard
/// positions stay as SPARQL variables. Objects are treated as URIs
/// when they carry an http(s) scheme, else as plain literals.
fn pattern_parts(
    subject: Option<&str>,
    predicate: Option<&str>,
    object: Option<&str>,
) -> Vec<(&'static str, String)> {
    vec![
        (
            "s",
            match subject {
                Some(s) => format!("<{s}>"),
                None => "?s".to_string(),
            },
        ),
        (
            "p",
            match predicate {
                Some(p) => format!("<{p}>"),
                None => "?p".to_string(),
            },
        ),
        (
            "o",
            match object {
                Some(o) if o.starts_with("http://") || o.starts_with("https://") => {
                    format!("<{o}>")
                }
                Some(o) => escape_literal(o),
                None => "?o".to_string(),
            },
        ),
    ]
}

impl SparqlStore {
    /// One page of a Triple Pattern Fragment.
    pub async fn pattern_fragment(
        &self,
        subject: Option<&str>,
        predicate: Option<&str>,
        object: Option<&str>,
        graph: Option<&str>,
        limit: u64,
        offset: u64,
        after: Option<&Cursor>,
        cardinality: Option<&Cardinality>,
    ) -> Result<FragmentPage, StoreError> {
        let parts = pattern_parts(subject, predicate, object);
        let where_terms: Vec<&str> = parts.iter().map(|(_, t)| t.as_str()).collect();

        // Always project ?s ?p ?o and BIND concrete positions to their
        // constant: Oxigraph omits variables that do not occur in the
        // pattern, and downstream consumers (jsonld.rs) expect every
        // binding to carry all three terms regardless of which positions
        // were wildcards. A BIND is only legal for a variable not already
        // in scope, which is exactly the concrete-position case.
        let mut binds = parts
            .iter()
            .filter(|(_, t)| !t.starts_with('?'))
            .map(|(var, t)| format!("BIND({t} AS ?{var})"))
            .collect::<Vec<_>>();

        // Cursor + stable ordering ride on string forms of the terms.
        binds.push("BIND(STR(?s) AS ?sv)".to_string());
        binds.push("BIND(STR(?p) AS ?pv)".to_string());
        binds.push("BIND(STR(?o) AS ?ov)".to_string());
        if let Some(cursor) = after {
            binds.push(cursor.filter_expression());
        }
        let binds_str = binds.join(" ");
        let terms_str = where_terms.join(" ");

        // Optional named graph (multi-tenancy: tenants map to graphs).
        // Without ?graph= the query covers the default graph only.
        let pattern_where = match graph {
            Some(g) => format!("GRAPH <{g}> {{ {terms_str} {binds_str} }}"),
            None => format!("{{ {terms_str} {binds_str} }}"),
        };
        let pattern_plain = match graph {
            Some(g) => format!("GRAPH <{g}> {{ {terms_str} }}"),
            None => format!("{{ {terms_str} }}"),
        };
        // ORDER BY on the string forms matches the cursor filter's
        // comparison space, making pagination deterministic.
        let order = "ORDER BY ?sv ?pv ?ov";

        let total = match cardinality.and_then(|c| c.estimate_for(subject, predicate, object)) {
            Some(est) if after.is_none() && graph.is_none() => est,
            _ => {
                let count_rows = self
                    .query_bindings(&format!("SELECT (COUNT(*) AS ?c) WHERE {{ {pattern_plain} }}"))
                    .await?;
                count_rows
                    .first()
                    .and_then(|r| r.pointer("/c/value").and_then(Value::as_str))
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0)
            }
        };

        // Fetch limit+1 to detect has_more without a second COUNT.
        let mut bindings = self
            .query_bindings(&format!(
                "SELECT ?s ?p ?o WHERE {{ {pattern_where} }} {order} LIMIT {} OFFSET {offset}",
                limit + 1
            ))
            .await?;
        let has_more = bindings.len() as u64 > limit;
        bindings.truncate(limit as usize);

        Ok(FragmentPage {
            has_more,
            bindings,
            total,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_roundtrip_is_lossless() {
        let c = Cursor {
            s: "http://example.org/a#b".into(),
            p: "http://p".into(),
            o: "plain literal".into(),
        };
        let decoded = Cursor::decode(&c.encode()).expect("decodes");
        assert_eq!(decoded.s, c.s);
        assert_eq!(decoded.p, c.p);
        assert_eq!(decoded.o, c.o);
    }

    #[test]
    fn cursor_decode_rejects_garbage() {
        assert!(Cursor::decode("not-base64!!!").is_none());
        assert!(Cursor::decode("").is_none());
    }
}
