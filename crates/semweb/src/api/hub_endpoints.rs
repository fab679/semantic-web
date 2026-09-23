//! POST /hub (WebSub §5 subscription requests, §6 publish) with optional
//! bearer-token auth (SEMWEB_HUB_TOKEN -- who may subscribe is policy),
//! and GET /topics/{name} — the topic resources that advertise discovery
//! per spec §4.


use axum::extract::{Path, State};
use axum::http::header;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Form;
use serde_json::json;

use crate::api::{discovery_headers, hub_error, internal, SharedState};
use crate::hub::{resolve_topic, TOPICS};

pub async fn hub_post(
    State(state): SharedState,
    headers: HeaderMap,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Result<Response, (StatusCode, String)> {
    // Who may subscribe is hub policy: when SEMWEB_HUB_TOKEN is set, the
    // whole POST /hub surface requires the bearer token.
    if !crate::api::bearer_ok(&state.hub_token, &headers) {
        return Err((StatusCode::UNAUTHORIZED, "missing or invalid bearer token".into()));
    }

    let mut params: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for (k, v) in pairs {
        // §5.1: "Hubs MUST ignore additional request parameters they do
        // not understand." We collect the ones we know and drop the rest.
        if k.starts_with("hub.") {
            params.insert(k, v);
        }
    }

    match params.get("hub.mode").map(String::as_str) {
        Some("publish") => {
            // §6: the publisher informs the hub a topic changed. The
            // spec leaves the mechanism unspecified; hub.mode=publish +
            // hub.url is the widely-used convention. The hub rebuilds
            // the topic content at publish time and fans out.
            let url = params.get("hub.url").map(String::as_str).unwrap_or("");
            let topic = resolve_topic(url).ok_or_else(|| {
                (StatusCode::BAD_REQUEST, "hub.url does not resolve to a known topic".to_string())
            })?;
            state
                .hub
                .clone()
                .publish(topic, &crate::hub::random_event_id())
                .await;
            Ok(StatusCode::ACCEPTED.into_response())
        }
        Some(mode @ ("subscribe" | "unsubscribe")) => {
            let topic = params
                .get("hub.topic")
                .ok_or_else(|| (StatusCode::BAD_REQUEST, "hub.topic is required".to_string()))?;
            let callback = params
                .get("hub.callback")
                .ok_or_else(|| (StatusCode::BAD_REQUEST, "hub.callback is required".to_string()))?;
            if callback.is_empty() {
                return Err((StatusCode::BAD_REQUEST, "hub.callback is required".to_string()));
            }
            let secret = params.get("hub.secret");
            let lease = params
                .get("hub.lease_seconds")
                .and_then(|s| s.parse::<u64>().ok());

            let result = match mode {
                "subscribe" => {
                    state
                        .hub
                        .clone()
                        .subscribe(topic.clone(), callback.clone(), secret.cloned(), lease)
                        .await
                }
                _ => state.hub.clone().unsubscribe(topic.clone(), callback.clone()).await,
            };
            match result {
                Ok(()) => {
                    // §5.1.2: 202 means "received and will now be verified";
                    // the outcome of verification must not affect this
                    // response, so we return before the task completes.
                    Ok((
                        StatusCode::ACCEPTED,
                        "subscription request accepted, verifying callback",
                    )
                        .into_response())
                }
                Err(e) => Err(hub_error(e)),
            }
        }
        _ => Err((
            StatusCode::BAD_REQUEST,
            "hub.mode must be 'subscribe', 'unsubscribe' or 'publish'".to_string(),
        )),
    }
}

pub async fn hub_get(State(state): SharedState) -> impl IntoResponse {
    // Not part of the WebSub spec -- a convenience for humans/agents
    // poking at the API before subscribing.
    let counts = state.hub.subscription_counts().await;
    let mut subs = serde_json::Map::new();
    for (topic, n) in counts {
        subs.insert(topic, json!(n));
    }
    axum::Json(json!({
        "topics": TOPICS,
        "subscriptions": subs,
    }))
}

/// Topic resources -- the WebSub discovery surface (§4). Returns the full
/// topic content with exactly one rel=self Link header (the canonical
/// topic URL) and a rel=hub link, in a Content-Type that content
/// distribution will preserve (§4.1: one representation per rel=self, so
/// no content-negotiation ambiguity exists).
pub async fn topic(
    State(state): SharedState,
    Path(name): Path<String>,
) -> Result<Response, (StatusCode, String)> {
    let canonical = format!("/topics/{name}");
    let topic = resolve_topic(&canonical)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("unknown topic: {name}")))?;
    let content = state.hub.build_topic_content(topic).await.map_err(internal)?;
    let links = discovery_headers(&state, topic);
    Ok((
        links,
        [(header::CONTENT_TYPE, content.content_type)],
        content.body,
    )
        .into_response())
}

// unused-import guards kept minimal
