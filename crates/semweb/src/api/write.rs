//! POST /admin/insert — the write path: one triple (optional named
//! graph), cardinality-counter updates, new-ontology-term detection
//! (O(1) from counters) and topic publishes. Token-gated by
//! SEMWEB_WRITE_TOKEN when configured.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use crate::api::{bearer_ok, bump_inserts, internal, SharedState};

#[derive(serde::Deserialize)]
pub struct InsertRequest {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    /// Optional named graph (multi-tenancy: tenants map to graphs).
    /// Absent = default graph.
    #[serde(default)]
    pub graph: Option<String>,
}

pub async fn admin_insert(
    State(state): SharedState,
    headers: HeaderMap,
    Json(body): Json<InsertRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Write auth: when SEMWEB_WRITE_TOKEN is set, mutating endpoints
    // require `Authorization: Bearer <token>`. Read endpoints stay open.
    if !bearer_ok(&state.write_token, &headers) {
        return Err((StatusCode::UNAUTHORIZED, "missing or invalid bearer token".into()));
    }

    // One event id ties the whole mutation together across logs, metrics
    // and fan-out.
    let event_id = crate::hub::random_event_id();
    let binding = state
        .store
        .insert_triple(&body.subject, &body.predicate, &body.object, body.graph.as_deref())
        .await
        .map_err(internal)?;
    bump_inserts();

    // Detect new ontology terms BEFORE bumping counters (O(1), replacing
    // the old full before/after schema diff per insert).
    let object_uri = if body.object.starts_with("http://") || body.object.starts_with("https://") {
        Some(body.object.as_str())
    } else {
        None
    };
    let was_new_predicate = !state.cardinality.has_predicate(&body.predicate);
    let was_new_class = object_uri
        .map(|o| body.predicate == crate::context::RDF_TYPE && !state.cardinality.has_class(o))
        .unwrap_or(false);
    let schema_changed = was_new_predicate || was_new_class;
    state
        .cardinality
        .record_insert(&body.subject, &body.predicate, object_uri, crate::context::RDF_TYPE);

    // Publish /topics/data for every instance-data change; when the
    // ontology surface changed, publish /topics/schema as well so
    // subscribers can distinguish "new data" from "the shape of the
    // graph changed" without inspecting every triple.
    state.hub.clone().notify_external_hubs("/topics/data", &event_id);
    if schema_changed {
        state.hub.clone().notify_external_hubs("/topics/schema", &event_id);
    }
    let data_notified = state.hub.clone().publish("/topics/data", &event_id).await;
    let schema_notified = if schema_changed {
        state.hub.clone().publish("/topics/schema", &event_id).await
    } else {
        0
    };

    let triple = crate::jsonld::binding_to_line(&binding, &state.prefixes).unwrap_or(binding);
    Ok(Json(json!({
        "event_id": event_id,
        "inserted": triple,
        "data_subscribers_notified": data_notified,
        "schema_changed": schema_changed,
        "schema_subscribers_notified": schema_notified,
    })))
}