//! GET /ui — the Semantic Graph Explorer: a single embedded page that is
//! a pure client over the live API.
//!
//! Design note: the UI is deliberately *self-describing like everything
//! else*. It builds itself from `/manifest` (classes, descriptions,
//! SHACL shapes, prefixes, fingerprint), reads patterns via
//! `/fragments`, previews topics, subscribes to `/events` via
//! EventSource, and surfaces `/health` + `/metrics`. There is no extra
//! backend and no build tooling — one static HTML file embedded in the
//! binary, so the whole project still ships as one artifact. It is a
//! lens on the graph, not an admin panel: the service remains an API.

use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

const UI_INDEX: &str = include_str!("../../assets/ui/index.html");

pub async fn explorer() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        UI_INDEX,
    )
        .into_response()
}