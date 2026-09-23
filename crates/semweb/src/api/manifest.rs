//! GET /manifest — the agent manifest: the planning surface for agents
//! and developers, regenerated live on every request.
//!
//! Carries:
//!   - schema fingerprint (sha256 over the sorted class+predicate URI set;
//!     also on /topics/schema content, so consumers detect missed
//!     schema-change events and diff safely),
//!   - live cardinalities (GROUP BY per request, never counter drift),
//!   - human descriptions from rdfs:comment / skos:definition,
//!   - SHACL property shapes from the shapes graph (when loaded),
//!   - a worked, executable SPARQL example per class (runnable at /sparql).

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::{json, Value};

use crate::api::{internal, SharedState};

pub async fn manifest(State(state): SharedState) -> Result<impl IntoResponse, (StatusCode, String)> {
    Ok(axum::Json(manifest_document(&state).await?))
}

/// Build the manifest document. Shared with the MCP resource server
/// (mcp/resources.rs), so both transports serve byte-identical content.
pub(crate) async fn manifest_document(
    state: &crate::state::AppState,
) -> Result<Value, (StatusCode, String)> {
    let info = state.store.describe_schema().await.map_err(internal)?;
    let descriptions = state.store.descriptions().await.map_err(internal)?;
    let class_counts = state.store.class_counts().await.map_err(internal)?;
    let predicate_counts = state.store.predicate_counts().await.map_err(internal)?;
    let shapes = state.store.shacl_shapes().await.map_err(internal)?;

    let fingerprint = crate::util::schema_fingerprint(&info.class_uris, &info.predicate_uris);

    let classes: Vec<Value> = info
        .class_uris
        .iter()
        .map(|uri| {
            let count = class_counts.get(uri).copied().unwrap_or(0);
            let shapes = shapes.get(uri).map(|props| {
                json!(props
                    .iter()
                    .map(|s| {
                        json!({
                            "path": s.path,
                            "minCount": s.min_count,
                            "maxCount": s.max_count,
                            "datatype": s.datatype,
                        })
                    })
                    .collect::<Vec<_>>())
            });
            json!({
                "uri": uri,
                "compact": state.prefixes.compact(uri),
                "instances": count,
                "description": descriptions.get(uri),
                "shapes": shapes,
                "exampleQuery": format!("SELECT ?s WHERE {{ ?s a <{uri}> }} LIMIT 10"),
            })
        })
        .collect();
    let predicates: Vec<Value> = info
        .predicate_uris
        .iter()
        .map(|uri| {
            let count = predicate_counts.get(uri).copied().unwrap_or(0);
            json!({
                "uri": uri,
                "compact": state.prefixes.compact(uri),
                "triples": count,
                "description": descriptions.get(uri),
            })
        })
        .collect();

    // SPARQL prefixes: what agents can (and should) PREFIX-declare. Derived
    // from the namespaces in use -- only registered prefixes are listed.
    let namespaces = crate::context::namespaces_of(&info.class_uris, &info.predicate_uris);
    let prefixes = state.prefixes.prefix_pairs(&namespaces);

    Ok(json!({
        "@context": "/context.jsonld",
        "kind": "agent-manifest",
        "generatedFrom": "live store state (not a cached build)",
        "schemaFingerprint": fingerprint,
        "prefixes": prefixes,
        "classes": classes,
        "predicates": predicates,
        "topics": crate::hub::TOPICS,
        "controls": {
            "fragments": "/fragments{?subject,predicate,object,after,limit,offset,graph}",
            "sparql": "/sparql{?query}",
            "hub": "/hub",
            "events": "/events{?topic}",
        },
    }))
}