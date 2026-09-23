//! A minimal, spec-conformant WebSub subscriber binary.
//!
//! Implements the Subscriber conformance class from docs/WebSub.md
//! (§3.1.1.2), useful for demos, tests and as a starting point for
//! real consumers:
//!
//! - GET  /callback : intent verification -- echoes hub.challenge back
//!   with a 2xx, served with a safe media type and
//!   X-Content-Type-Options: nosniff (§8.2 anti-XSS rules).
//! - POST /callback : content distribution -- MUST acknowledge with 2xx;
//!   when SUBSCRIBER_SECRET is set, validates the hub's X-Hub-Signature
//!   and locally discards invalid messages (§7.1.2) while still
//!   acknowledging receipt.
//!
//! Configuration (environment):
//!   DEMO_SUBSCRIBER_PORT   listen port (default 9000)
//!   SUBSCRIBER_SECRET      hub.secret the subscription was made with
//!                          (enables signature validation)

use axum::extract::{Request as AxumRequest, State};
use axum::http::header;
use axum::response::IntoResponse;
use hmac::{Hmac, Mac};
use sha2::Sha256;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let port: u16 = std::env::var("DEMO_SUBSCRIBER_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(9000);
    let secret = std::env::var("SUBSCRIBER_SECRET").ok();

    tracing::info!("demo subscriber listening on 0.0.0.0:{port} (signature validation: {})",
        if secret.is_some() { "enabled" } else { "off" });

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .expect("bind port");
    let app = axum::Router::new()
        .route("/callback", axum::routing::get(verify).post(deliver))
        .with_state(secret);
    axum::serve(listener, app).await.expect("server error");
}

/// §5.3.1: respond 2xx with the challenge as the body. Safe media type +
/// nosniff per §8.2 (reflected-XSS mitigation).
async fn verify(req: AxumRequest) -> axum::response::Response {
    let query: std::collections::HashMap<String, String> =
        req.uri().query().map(|q| {
            q.split('&')
                .filter_map(|kv| kv.split_once('='))
                .map(|(k, v)| (kv_decode(k), kv_decode(v)))
                .collect()
        }).unwrap_or_default();
    let challenge = query.get("hub.challenge").cloned().unwrap_or_default();
    tracing::info!(
        "intent verification: mode={}, topic={}",
        query.get("hub.mode").map(String::as_str).unwrap_or("?"),
        query.get("hub.topic").map(String::as_str).unwrap_or("?")
    );
    (
        axum::http::StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (header::HeaderName::from_static("x-content-type-options"), "nosniff"),
        ],
        challenge,
    )
        .into_response()
}

/// §7: acknowledge content distribution with 2xx (the ack means
/// "received", not "processed"); validate X-Hub-Signature when a secret
/// is registered and locally discard failures (§7.1.2).
async fn deliver(
    State(secret): State<Option<String>>,
    req: AxumRequest,
) -> axum::response::Response {
    let (parts, body) = req.into_parts();
    let body = axum::body::to_bytes(body, usize::MAX)
        .await
        .unwrap_or_default();

    if let Some(secret) = &secret {
        let valid = parts
            .headers
            .get("X-Hub-Signature")
            .and_then(|v| v.to_str().ok())
            .map(|provided| {
                match signature(secret.as_bytes(), &body) {
                    Some(expected) => provided == format!("sha256={expected}"),
                    None => false,
                }
            })
            .unwrap_or(false);
        if !valid {
            tracing::warn!("content distribution failed signature validation; discarded locally");
            return axum::http::StatusCode::ACCEPTED.into_response();
        }
        tracing::info!("content distribution received; signature verified (sha256)");
    } else {
        tracing::info!("content distribution received (no secret; unsigned)");
    }

    tracing::info!(
        "content-type={:?} link={:?} bytes={}",
        parts.headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()),
        parts.headers.get(header::LINK).and_then(|v| v.to_str().ok()),
        body.len()
    );
    for line in String::from_utf8_lossy(&body).lines() {
        if !line.trim().is_empty() {
            tracing::info!("  {line}");
        }
    }
    axum::http::StatusCode::OK.into_response()
}

/// Recompute the §7.1 signature: HMAC-SHA256 over the body, keyed by the
/// subscription secret, lowercase hex.
fn signature(secret: &[u8], body: &[u8]) -> Option<String> {
    let mut mac = <Hmac<Sha256> as hmac::Mac>::new_from_slice(secret).ok()?;
    mac.update(body);
    Some(
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
    )
}

fn kv_decode(s: &str) -> String {
    // hub.challenge and hub.topic arrive percent-encoded in the query.
    urlencoding::decode(s).map(|c| c.into_owned()).unwrap_or_else(|_| s.to_string())
}