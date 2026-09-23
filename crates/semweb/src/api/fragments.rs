//! GET /fragments — Triple Pattern Fragments, streamed as NDJSON-LD
//! (one compacted JSON-LD statement per line), cursor or offset
//! pagination, optional named graph.

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{json, Value};

use crate::api::{bump_fragments_requests, internal, ndjson_response, SharedState};
use crate::store::Cursor;

pub async fn fragments(
    State(state): SharedState,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, (StatusCode, String)> {
    bump_fragments_requests();
    let subject = params.get("subject").filter(|s| !s.is_empty()).map(String::as_str);
    let predicate = params.get("predicate").filter(|s| !s.is_empty()).map(String::as_str);
    let object = params.get("object").filter(|s| !s.is_empty()).map(String::as_str);
    let graph = params.get("graph").filter(|s| !s.is_empty()).map(String::as_str);
    let limit = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(100)
        .min(1000);
    let offset = params
        .get("offset")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let after = params
        .get("after")
        .filter(|s| !s.is_empty())
        .and_then(|a| Cursor::decode(a));

    let page = state
        .store
        .pattern_fragment(
            subject,
            predicate,
            object,
            graph,
            limit,
            offset,
            after.as_ref(),
            Some(&state.cardinality),
        )
        .await
        .map_err(internal)?;

    // Stream each matching triple as soon as it's ready -- a client can
    // start acting on line 1 without waiting for the page. The final line
    // is the Hydra-style hypermedia control; clients that don't care
    // about pagination ignore lines carrying "@control".
    let mut lines: Vec<Value> = page
        .bindings
        .iter()
        .filter_map(|b| crate::jsonld::binding_to_line(b, &state.prefixes))
        .collect();

    let mut control = json!({ "@control": "metadata", "count_estimate": page.total });
    if page.has_more {
        // Cursor pagination (production mechanism): continue after the
        // last triple of this page.
        if let Some(cursor) = page.bindings.last().and_then(cursor_from_binding) {
            control["after"] = json!(cursor.encode());
        }
        // Offset pagination (legacy mechanism) for offset-based clients.
        let next_offset = offset + limit;
        control["next"] = json!(format!(
            "/fragments?subject={}&predicate={}&object={}&limit={limit}&offset={next_offset}",
            subject.unwrap_or(""),
            predicate.unwrap_or(""),
            object.unwrap_or(""),
        ));
    }
    lines.push(control);
    Ok(ndjson_response(lines))
}

/// Extract the last-seen triple from a binding (the cursor source).
fn cursor_from_binding(binding: &Value) -> Option<Cursor> {
    Some(Cursor {
        s: binding.pointer("/s/value")?.as_str()?.to_string(),
        p: binding.pointer("/p/value")?.as_str()?.to_string(),
        o: binding.pointer("/o/value")?.as_str()?.to_string(),
    })
}