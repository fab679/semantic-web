//! MCP resources: the live agent manifest, discoverable and readable the
//! way an MCP-aware agent lists tools. The manifest document is the SAME
//! one `GET /manifest` serves (shared builder in api/manifest.rs).


use serde_json::{json, Value};

use crate::state::AppState;

pub(crate) const MANIFEST_URI: &str = "manifest://semantic-web/current";

pub(crate) fn list() -> Value {
    json!({
        "resources": [{
            "uri": MANIFEST_URI,
            "name": "Agent manifest (live)",
            "description": "Live ontology surface: classes, predicates, cardinalities, \
                            descriptions, SHACL shapes, example SPARQL, topics.",
            "mimeType": "application/json",
        }],
    })
}

pub(crate) async fn read(state: &AppState, id: Option<Value>, params: &Value) -> Value {
    let uri = params.get("uri").and_then(Value::as_str).unwrap_or("");
    if uri != MANIFEST_URI {
        return super::json_rpc_error(id, -32602, &format!("unknown resource: {uri}"));
    }
    match crate::api::manifest_document(state).await {
        Ok(doc) => super::json_rpc_result(
            id,
            json!({
                "contents": [{
                    "uri": MANIFEST_URI,
                    "mimeType": "application/json",
                    "text": serde_json::to_string(&doc).unwrap_or_default(),
                }]
            }),
        ),
        Err((_, msg)) => super::json_rpc_error(id, -32603, &msg),
    }
}