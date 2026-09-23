//! GET /sparql — read-only SPARQL 1.1 Protocol passthrough: the
//! execution plane, one hop from the discovery plane. Read-only by
//! construction: queries are forwarded to the store's /query endpoint,
//! which only executes SPARQL Query forms (SELECT/ASK/DESCRIBE/CONSTRUCT);
//! updates live on a separate /update URL this handler never touches.

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
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
