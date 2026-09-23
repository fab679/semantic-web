//! GET / (live self-description + discovery Link headers) and
//! GET /context.jsonld (live-generated JSON-LD @context).

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::json;

use crate::api::{discovery_headers, internal, topics_value, SharedState};

pub async fn root(State(state): SharedState) -> Result<impl IntoResponse, (StatusCode, String)> {
    let info = state.store.describe_schema().await.map_err(internal)?;
    // Compact class/predicate URIs through the (runtime extensible) prefix
    // map: standard vocabularies become foaf:name-style CURIEs, unknown
    // namespaces stay full URIs. No domain terms are hardcoded anywhere.
    let classes: Vec<String> = info.class_uris.iter().map(|u| state.prefixes.compact(u)).collect();
    let predicates: Vec<String> = info
        .predicate_uris
        .iter()
        .map(|u| state.prefixes.compact(u))
        .collect();
    // Advertise the hub per WebSub discovery (§4): any resource can carry
    // rel=hub / rel=self Link headers; the topic resource below is where
    // the spec makes it mandatory.
    let links = discovery_headers(&state, "/");
    Ok((
        links,
        axum::Json(json!({
            "@context": "/context.jsonld",
            "generatedFrom": "live store state (not a cached build)",
            "storeMode": "sparql-rust",
            "classes": classes,
            "predicates": predicates,
            "controls": {
                "fragments": "/fragments{?subject,predicate,object,after,limit,offset,graph}",
                "sparql": "/sparql{?query}",
                "manifest": "/manifest",
                "mcp": "/mcp",
                "events": "/events{?topic}",
                "hub": "/hub",
                "topics": topics_value(),
            },
        })),
    ))
}

/// The shared JSON-LD @context, generated LIVE from the namespaces
/// currently in use in the store -- same no-build-step principle as the
/// self-description. Adding a term from a known vocabulary needs no code
/// change; a brand-new namespace needs at most an SEMWEB_EXTRA_PREFIXES
/// entry to get a friendly prefix.
pub async fn context_jsonld(
    State(state): SharedState,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let info = state.store.describe_schema().await.map_err(internal)?;
    let namespaces = crate::context::namespaces_of(&info.class_uris, &info.predicate_uris);
    Ok(axum::Json(state.prefixes.context_document(&namespaces)))
}