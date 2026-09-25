//! GET /sparql and POST /sparql — read-only SPARQL 1.1 Protocol
//! passthrough: the execution plane, one hop from the discovery plane.
//! Read-only by construction: queries are forwarded to the store's
//! /query endpoint, which only executes SPARQL Query forms
//! (SELECT/ASK/DESCRIBE/CONSTRUCT); updates live on a separate /update
//! URL this handler never touches.
//!
//! POST support (SPARQL 1.1 Protocol §2.2): form-encoded (`query=`) and
//! raw `application/sparql-query` bodies — long queries do not fit in a
//! GET query string. `application/sparql-update` is rejected outright.

use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::api::SharedState;

pub async fn sparql(
    State(state): SharedState,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, (StatusCode, String)> {
    let query = params
        .get("query")
        .filter(|q| !q.is_empty())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "query parameter is required".to_string()))?;
    run_readonly_query(&state, query).await
}

pub async fn sparql_post(
    State(state): SharedState,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, (StatusCode, String)> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    let query = match content_type.as_str() {
        // SPARQL Protocol §2.1.2: POST with form-encoded parameters.
        "application/x-www-form-urlencoded" => form_query_param(&body).ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "form body must carry a non-empty 'query' parameter".to_string(),
            )
        })?,
        // SPARQL Protocol §2.2.1: POST with the query as the raw body.
        "application/sparql-query" => {
            let q = String::from_utf8_lossy(&body).trim().to_string();
            if q.is_empty() {
                return Err((StatusCode::BAD_REQUEST, "empty query body".into()));
            }
            q
        }
        // Updates are not accepted here — read-only by construction.
        "application/sparql-update" => {
            return Err((
                StatusCode::BAD_REQUEST,
                "SPARQL updates are not accepted on /sparql (read-only execution plane)".into(),
            ))
        }
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!(
                    "unsupported content type '{other}': use GET ?query=, form-encoded POST, \
                     or Content-Type: application/sparql-query"
                ),
            ))
        }
    };
    run_readonly_query(&state, &query).await
}

/// Shared forwarder: send the query to the store's /query endpoint and
/// relay status + content-type + body.
async fn run_readonly_query(
    state: &crate::state::AppState,
    query: &str,
) -> Result<Response, (StatusCode, String)> {
    let (status, content_type, body) = state
        .store
        .query_raw(query, "application/sparql-results+json")
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.0))?;
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let ct = header::HeaderValue::from_str(&content_type)
        .unwrap_or_else(|_| header::HeaderValue::from_static("application/sparql-results+json"));
    Ok((code, [(header::CONTENT_TYPE, ct)], body).into_response())
}

/// Minimal form parser: find `query=...` among the urlencoded pairs.
/// Form encoding uses `+` for spaces (application/x-www-form-urlencoded),
/// so both `+` and `%XX` escapes are decoded.
fn form_query_param(body: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(body);
    for pair in text.split('&') {
        let (k, v) = pair.split_once('=')?;
        if form_decode(k)? == "query" {
            return form_decode(v);
        }
    }
    None
}

fn form_decode(s: &str) -> Option<String> {
    urlencoding::decode(&s.replace('+', " "))
        .ok()
        .map(|v| v.into_owned())
}