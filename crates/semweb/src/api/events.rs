//! GET /events — the SSE change feed.
//!
//! WebSub covers durable backend subscribers (they can be offline between
//! notifications); a live agent session wants a persistent stream instead.
//! This endpoint is the complementary channel: same in-process publish
//! bus, streamed as Server-Sent Events.
//!
//! Each event carries the notification header only ({topic, event_id});
//! clients refetch GET /topics/{name} for content, keeping the stream
//! small regardless of topic size. `?topic=/topics/data` filters.

use std::convert::Infallible;

use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use std::collections::HashMap;

use crate::api::SharedState;
use crate::hub::resolve_topic;

pub async fn events(
    State(state): SharedState,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, (StatusCode, String)> {
    let topic_filter = params.get("topic").filter(|t| !t.is_empty());
    if let Some(t) = topic_filter {
        if resolve_topic(t).is_none() {
            return Err((StatusCode::BAD_REQUEST, format!("unknown topic: {t}")));
        }
    }
    let filter = topic_filter.map(|t| t.to_string());
    let receiver = state.hub.sse_receiver();
    let keepalive_interval = std::time::Duration::from_secs(15);

    let stream = futures::stream::unfold(
        (receiver, filter, keepalive_interval),
        |(mut rx, filter, keepalive)| async move {
            loop {
                // Broadcast channel: with no live sessions publishing, recv
                // would block forever; use a bounded wait so keepalives
                // flow and the connection stays observable.
                match tokio::time::timeout(keepalive, rx.recv()).await {
                    Ok(Ok(event)) => {
                        if let Some(filter) = &filter {
                            if resolve_topic(&event.topic) != resolve_topic(filter) {
                                continue;
                            }
                        }
                        let payload = serde_json::to_string(&event)
                            .unwrap_or_else(|_| "{}".into());
                        return Some((Ok::<_, Infallible>(sse_line(&payload)), (rx, filter, keepalive)));
                    }
                    Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(n))) => {
                        // The receiver fell behind; tell the client so it can
                        // resync (e.g. refetch the topic content).
                        let payload = serde_json::json!({
                            "error": "lagged", "missed": n.to_string(),
                        })
                        .to_string();
                        return Some((Ok(sse_line(&payload)), (rx, filter, keepalive)));
                    }
                    Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => return None,
                    Err(_) => {
                        // Timeout -> keepalive comment (SSE clients ignore it;
                        // it keeps intermediaries from closing the stream).
                        return Some((Ok(": keepalive\n\n".to_string()), (rx, filter, keepalive)));
                    }
                }
            }
        },
    );

    let body = axum::body::Body::from_stream(stream);
    Ok((
        [
            (header::CONTENT_TYPE, "text/event-stream"),
            (axum::http::header::HeaderName::from_static("cache-control"), "no-cache"),
        ],
        body,
    )
        .into_response())
}

fn sse_line(payload: &str) -> String {
    format!("data: {payload}\n\n")
}
