//! POST /admin/insert — the write path: one triple (optional named
//! graph), cardinality-counter updates, new-ontology-term detection
//! (O(1) from counters) and topic publishes. Token-gated by
//! SEMWEB_WRITE_TOKEN when configured.
//!
//! `insert_and_publish` is the single write pipeline shared by the HTTP
//! endpoint and the MCP `insert_triple` tool, so both paths get the same
//! duplicate suppression, schema-change detection, external-hub notify
//! and reserved-graph guard.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};

use crate::api::{bearer_ok, bump_inserts, internal, SharedState};
use crate::store::persistence::is_reserved_graph;

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

/// Outcome of one write through the shared pipeline.
pub(crate) struct InsertOutcome {
    pub event_id: String,
    pub triple: Value,
    pub data_notified: usize,
    pub schema_changed: bool,
    pub schema_notified: usize,
    /// True when the triple already existed: no store change, no counter
    /// bump, no topic publish.
    pub duplicate: bool,
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
    let outcome = insert_and_publish(
        &state,
        &body.subject,
        &body.predicate,
        &body.object,
        body.graph.as_deref(),
    )
    .await?;
    Ok(Json(json!({
        "event_id": outcome.event_id,
        "inserted": outcome.triple,
        "duplicate": outcome.duplicate,
        "data_subscribers_notified": outcome.data_notified,
        "schema_changed": outcome.schema_changed,
        "schema_subscribers_notified": outcome.schema_notified,
    })))
}

/// The single write pipeline: store insert, counter update, new-term
/// detection, topic publishes (data + schema when the ontology surface
/// changed) and external-hub notification. Used by `POST /admin/insert`
/// and the MCP `insert_triple` tool so both paths behave identically.
pub(crate) async fn insert_and_publish(
    state: &crate::state::AppState,
    subject: &str,
    predicate: &str,
    object: &str,
    graph: Option<&str>,
) -> Result<InsertOutcome, (StatusCode, String)> {
    // Reserved-graph guard: hub state lives in service-managed named
    // graphs; writing there from the outside could forge subscriptions
    // and bypass the §5.3 verification handshake.
    if let Some(g) = graph {
        if is_reserved_graph(g) {
            return Err((
                StatusCode::BAD_REQUEST,
                "graph URI is reserved for service state".into(),
            ));
        }
    }

    // One event id ties the whole mutation together across logs, metrics
    // and fan-out.
    let event_id = crate::hub::random_event_id();

    // Duplicate suppression: INSERT DATA is idempotent in the store, but
    // re-inserting an existing triple must not bump counters or push a
    // no-op notification.
    let existed = state
        .store
        .triple_exists(subject, predicate, object, graph)
        .await
        .map_err(internal)?;
    if existed {
        let binding = json!({
            "s": {"type": "uri", "value": subject},
            "p": {"type": "uri", "value": predicate},
            "o": object_binding(object),
        });
        let triple = crate::jsonld::binding_to_line(&binding, &state.prefixes).unwrap_or(binding);
        tracing::info!(event = event_id, "insert skipped: triple already exists");
        return Ok(InsertOutcome {
            event_id,
            triple,
            data_notified: 0,
            schema_changed: false,
            schema_notified: 0,
            duplicate: true,
        });
    }

    let binding = state
        .store
        .insert_triple(subject, predicate, object, graph)
        .await
        .map_err(internal)?;
    bump_inserts();

    // Detect new ontology terms BEFORE bumping counters (O(1), replacing
    // the old full before/after schema diff per insert).
    let object_uri = if object.starts_with("http://") || object.starts_with("https://") {
        Some(object)
    } else {
        None
    };
    let was_new_predicate = !state.cardinality.has_predicate(predicate);
    let was_new_class = object_uri
        .map(|o| predicate == crate::context::RDF_TYPE && !state.cardinality.has_class(o))
        .unwrap_or(false);
    let schema_changed = was_new_predicate || was_new_class;
    state
        .cardinality
        .record_insert(subject, predicate, object_uri, crate::context::RDF_TYPE);

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
    Ok(InsertOutcome {
        event_id,
        triple,
        data_notified,
        schema_changed,
        schema_notified,
        duplicate: false,
    })
}

/// Raw binding for the object term (uri vs literal), same rule the store
/// applies when writing.
fn object_binding(object: &str) -> Value {
    if object.starts_with("http://") || object.starts_with("https://") {
        json!({"type": "uri", "value": object})
    } else {
        json!({"type": "literal", "value": object})
    }
}