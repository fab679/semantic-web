//! POST /mcp — a minimal MCP (Model Context Protocol) resource server.
//!
//! The design doc's manifest is *content*; HTTP GET, WebSub and MCP are
//! transports for it. This endpoint speaks enough of the MCP JSON-RPC 2.0
//! surface for an MCP-aware agent to discover and read the live agent
//! manifest as a resource, exactly the way it lists tools:
//!
//!   initialize            -> protocol handshake + capabilities
//!   resources/list        -> the manifest resource
//!   resources/read        -> the manifest document itself
//!
//! Single POST endpoint, application/json, standard JSON-RPC framing.
//! (Not a full MCP server: no SSE streaming transport, no prompts/tools
//! -- scope is resources only, per the manifest-delivery goal. The
//! resources live at manifest://semantic-web/current.)

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::api::SharedState;

const MANIFEST_URI: &str = "manifest://semantic-web/current";
const PROTOCOL_VERSION: &str = "2025-06-18";

#[derive(serde::Deserialize)]
struct RpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

pub async fn mcp(State(state): SharedState, Json(req): Json<serde_json::Value>) -> Response {
    // Accept a single request object (MCP streamable-HTTP JSON-RPC).
    let parsed: Result<serde_json::Value, _> = serde_json::from_value(req.clone());
    let Ok(parsed) = parsed else {
        return json_rpc_error(None, -32700, "Parse error").into_response();
    };

    // MCP requires the JSON-RPC envelope; accept both wrapped and bare
    // methods for lenient clients (notifications produce no response).
    if parsed["method"] == json!("notifications/initialized") {
        return StatusCode::ACCEPTED.into_response();
    }

    let rpc: Result<RpcRequest, _> = serde_json::from_value(parsed);
    let Ok(rpc) = rpc else {
        return json_rpc_error(None, -32600, "Invalid Request").into_response();
    };
    if rpc.jsonrpc != "2.0" {
        return json_rpc_error(rpc.id, -32600, "Invalid Request").into_response();
    }

    match rpc.method.as_str() {
        "initialize" => json_rpc_result(
            rpc.id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "resources": {} },
                "serverInfo": { "name": "semantic-web", "version": env!("CARGO_PKG_VERSION") },
            }),
        )
        .into_response(),
        "resources/list" => json_rpc_result(
            rpc.id,
            json!({
                "resources": [{
                    "uri": MANIFEST_URI,
                    "name": "Agent manifest (live)",
                    "description": "Live ontology surface: classes, predicates, cardinalities, \
                                    descriptions, SHACL shapes, example SPARQL, topics.",
                    "mimeType": "application/json",
                }],
            }),
        )
        .into_response(),
        "resources/read" => {
            let uri = rpc
                .params
                .get("uri")
                .and_then(Value::as_str)
                .unwrap_or("");
            if uri != MANIFEST_URI {
                return json_rpc_error(rpc.id, -32602, &format!("unknown resource: {uri}"))
                    .into_response();
            }
            match render_manifest(&state).await {
                Ok(text) => json_rpc_result(
                    rpc.id,
                    json!({
                        "contents": [{
                            "uri": MANIFEST_URI,
                            "mimeType": "application/json",
                            "text": text,
                        }]
                    }),
                )
                .into_response(),
                Err(e) => json_rpc_error(rpc.id, -32603, &e).into_response(),
            }
        }
        other => json_rpc_error(rpc.id, -32601, &format!("method not found: {other}")).into_response(),
    }
}

async fn render_manifest(state: &crate::state::AppState) -> Result<String, String> {
    let info = state.store.describe_schema().await.map_err(|e| e.0)?;
    let descriptions = state.store.descriptions().await.map_err(|e| e.0)?;
    let class_counts = state.store.class_counts().await.map_err(|e| e.0)?;
    let predicate_counts = state.store.predicate_counts().await.map_err(|e| e.0)?;
    let shapes = state.store.shacl_shapes().await.map_err(|e| e.0)?;
    let doc = json!({
        "@context": "/context.jsonld",
        "kind": "agent-manifest",
        "generatedFrom": "live store state (not a cached build)",
        "schemaFingerprint": crate::util::schema_fingerprint(&info.class_uris, &info.predicate_uris),
        "classes": info.class_uris.iter().map(|uri| {
            let shapes = shapes.get(uri).map(|props| json!(props.iter().map(|s| json!({
                "path": s.path, "minCount": s.min_count, "maxCount": s.max_count,
                "datatype": s.datatype })).collect::<Vec<_>>()));
            json!({
                "uri": uri,
                "compact": state.prefixes.compact(uri),
                "instances": class_counts.get(uri).copied().unwrap_or(0),
                "description": descriptions.get(uri),
                "shapes": shapes,
                "exampleQuery": format!("SELECT ?s WHERE {{ ?s a <{uri}> }} LIMIT 10"),
            })
        }).collect::<Vec<_>>(),
        "predicates": info.predicate_uris.iter().map(|uri| json!({
            "uri": uri,
            "compact": state.prefixes.compact(uri),
            "triples": predicate_counts.get(uri).copied().unwrap_or(0),
            "description": descriptions.get(uri),
        })).collect::<Vec<_>>(),
        "topics": crate::hub::TOPICS,
        "controls": {
            "fragments": "/fragments{?subject,predicate,object,after,limit,offset,graph}",
            "sparql": "/sparql{?query}",
            "hub": "/hub",
            "events": "/events{?topic}",
        },
    });
    serde_json::to_string(&doc).map_err(|e| e.to_string())
}

fn json_rpc_result(id: Option<Value>, result: Value) -> axum::Json<Value> {
    axum::Json(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    }))
}

fn json_rpc_error(id: Option<Value>, code: i64, message: &str) -> axum::Json<Value> {
    axum::Json(json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    }))
}
