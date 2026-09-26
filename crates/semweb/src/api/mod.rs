//! The HTTP surface (axum). Plain REST only -- the architecture rule is
//! that every interaction rides on standards: GET/POST, query params,
//! form-encoded WebSub subscription requests, NDJSON streaming, JSON-LD,
//! SSE, and a minimal MCP resource server.
//!
//! Sub-modules (one per endpoint group, kept small for easy editing):
//! - `self_description` : GET / and /context.jsonld
//! - `fragments`        : GET /fragments (TPF, cursor pagination)
//! - `sparql`           : GET /sparql (read-only passthrough)
//! - `manifest`         : GET /manifest (agent manifest + SHACL shapes)
//! - `events`           : GET /events (SSE change feed)
//! - `observability`    : GET /health, GET /metrics
//! - `hub_endpoints`    : POST|GET /hub, GET /topics/{name}
//! - `write`            : POST /admin/insert (token-gated)
//! - `mcp`              : POST /mcp (minimal MCP resource server)

mod events;
mod fragments;
mod health_metrics;
mod hub_endpoints;
mod manifest;
mod mcp;
mod did;
mod self_description;
mod sparql;
mod ui;
mod write;

pub use did::did_document;
pub use events::events;
pub use fragments::fragments;
pub use health_metrics::{health, metrics};
pub use hub_endpoints::{hub_get, hub_post, topic};
pub use manifest::manifest;
pub(crate) use manifest::manifest_document;
pub use mcp::mcp;
pub use self_description::{context_jsonld, root};
pub use sparql::{sparql, sparql_post};
pub use ui::explorer;
pub use write::admin_insert;

use axum::extract::State;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::body::Body;
use bytes::Bytes;
use futures::stream;
use serde_json::Value;
use subtle::ConstantTimeEq;

use crate::hub::{HubError, TOPICS};
use crate::state::AppState;

pub type SharedState = State<Arc<AppState>>;

/// Service-side request counters (hub counters live in hub::metrics).
pub(crate) mod counters {
    use std::sync::atomic::AtomicU64;
    pub static FRAGMENTS_REQUESTS: AtomicU64 = AtomicU64::new(0);
    pub static INSERTS_TOTAL: AtomicU64 = AtomicU64::new(0);
}

pub(crate) fn bump_fragments_requests() {
    counters::FRAGMENTS_REQUESTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn bump_inserts() {
    counters::INSERTS_TOTAL.fetch_add(1, Ordering::Relaxed);
}


/// WebSub discovery Link headers (§4): exactly one rel=self (the canonical
/// topic URL) and one rel=hub, combined into a single Link header (§7
/// allows combining; subscribers must handle either form).
pub(crate) fn discovery_headers(state: &AppState, topic_path: &str) -> HeaderMap {
    let self_url = format!("{}{topic_path}", state.public_url);
    // §4: at least one rel=hub; a publisher MAY advertise several for
    // fault tolerance, and subscribers may subscribe at any of them.
    let mut hubs = vec![format!("{}/hub", state.public_url)];
    for h in state.hub.external_hubs() {
        hubs.push(h);
    }
    let hub_links = hubs
        .iter()
        .map(|h| format!("<{h}>; rel=\"hub\""))
        .collect::<Vec<_>>()
        .join(", ");
    let value = format!("<{self_url}>; rel=\"self\", {hub_links}");
    let mut headers = HeaderMap::new();
    headers.insert(
        header::LINK,
        header::HeaderValue::from_str(&value).expect("link header"),
    );
    headers
}

/// NDJSON streaming response: one compacted JSON-LD line per triple.
pub(crate) fn ndjson_response(lines: Vec<Value>) -> axum::response::Response {
    let body = Body::from_stream(stream::iter(lines.into_iter().map(|line| {
        let mut buf = serde_json::to_string(&line).unwrap_or_default();
        buf.push('\n');
        Ok::<_, std::convert::Infallible>(Bytes::from(buf))
    })));
    ([(header::CONTENT_TYPE, "application/x-ndjson")], body).into_response()
}

pub(crate) fn hub_error(e: HubError) -> (StatusCode, String) {
    match e {
        HubError::UnknownTopic(msg) => (StatusCode::NOT_FOUND, msg),
        HubError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
        HubError::TooManyRequests => (
            StatusCode::TOO_MANY_REQUESTS,
            "subscription rate limit exceeded for this callback".into(),
        ),
    }
}

fn internal(e: crate::store::StoreError) -> (StatusCode, String) {
    tracing::error!("store error: {e}");
    (StatusCode::INTERNAL_SERVER_ERROR, e.0)
}

/// Bearer-token check shared by the write path and (when configured) the
/// hub endpoint. Comparison is constant-time (subtle::ConstantTimeEq) so
/// token validity is not observable through timing.
pub(crate) fn bearer_ok(expected: &Option<String>, headers: &HeaderMap) -> bool {
    let Some(expected) = expected else {
        return true; // endpoint is ungated
    };
    let provided = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    match provided {
        Some(p) => bool::from(p.as_bytes().ct_eq(expected.as_bytes())),
        None => false,
    }
}

/// Topics list for controls blocks.
pub(crate) fn topics_value() -> Value {
    serde_json::json!(TOPICS)
}

#[cfg(test)]
mod tests {
    use super::bearer_ok;
    use axum::http::{header, HeaderMap, HeaderValue};

    fn hdr(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::AUTHORIZATION, HeaderValue::from_str(value).unwrap());
        h
    }

    #[test]
    fn bearer_ok_requires_the_exact_token() {
        let ungated: Option<String> = None;
        assert!(bearer_ok(&ungated, &HeaderMap::new()));

        let tok = Some("secret-token".to_string());
        assert!(!bearer_ok(&tok, &HeaderMap::new()));
        assert!(!bearer_ok(&tok, &hdr("Basic secret-token")));
        assert!(!bearer_ok(&tok, &hdr("Bearer secret-toke")));
        assert!(!bearer_ok(&tok, &hdr("Bearer  secret-token")));
        assert!(bearer_ok(&tok, &hdr("Bearer secret-token")));
    }
}