//! POST /mcp — MCP (Model Context Protocol) server, JSON-RPC 2.0 over
//! the streamable-HTTP POST transport (single-JSON responses).
//!
//! Full MCP surface served for the semantic graph:
//!
//!   initialize              -> handshake: resources + tools + prompts
//!   notifications/initialized -> 202 (no response)
//!   ping                    -> {}
//!   resources/list, resources/read -> the live agent manifest
//!   tools/list, tools/call  -> the graph tools (see tools.rs):
//!       search_graph    TPF fragment lookup (subject/predicate/object/graph)
//!       sparql_query    read-only SPARQL SELECT/ASK/DESCRIBE/CONSTRUCT
//!       get_manifest    the agent manifest (schema fingerprint, shapes,
//!                       cardinalities, descriptions, example queries)
//!       get_topic       full topic content (data as NDJSON / schema JSON)
//!       insert_triple   write path (honours the write token policy)
//!       verify_claim    four-gate verification of a signed document
//!                       (signature, issuer trust, temporal, revocation)
//!       issue_attestation  mint a signed VC under this deployment's DID
//!       subscribe       WebSub subscription on behalf of a callback URL
//!   prompts/list, prompts/get -> guided-exploration prompt templates
//!
//! Tool results follow the MCP shape { content: [{type:"text", text}] }
//! with output truncated to MCP_MAX_TOOL_TEXT so an LLM caller never
//! receives unbounded payloads.

mod prompts;
mod resources;
mod tools;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::api::SharedState;

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
/// Cap tool payloads so LLM consumers never get unbounded text.
const MCP_MAX_TOOL_TEXT: usize = 8_000;

#[derive(serde::Deserialize)]
struct RpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

pub async fn mcp(State(state): SharedState, Json(req): Json<serde_json::Value>) -> Response {
    // Notifications produce no response (202) per JSON-RPC/MCP.
    if req["method"] == json!("notifications/initialized")
        || req["method"] == json!("notifications/cancelled")
    {
        return StatusCode::ACCEPTED.into_response();
    }

    let rpc: RpcRequest = match serde_json::from_value(req) {
        Ok(r) => r,
        Err(_) => return Json(json_rpc_error(None, -32700, "Parse error")).into_response(),
    };
    if rpc.jsonrpc != "2.0" {
        return Json(json_rpc_error(rpc.id, -32600, "Invalid Request")).into_response();
    }

    let response = match rpc.method.as_str() {
        "initialize" => json_rpc_result(
            rpc.id,
            json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {
                    "resources": { "listChanged": false },
                    "tools": { "listChanged": false },
                    "prompts": { "listChanged": false },
                },
                "serverInfo": {
                    "name": "semantic-web",
                    "version": env!("CARGO_PKG_VERSION"),
                },
            }),
        ),
        "ping" => json_rpc_result(rpc.id, json!({})),
        "resources/list" => json_rpc_result(rpc.id, resources::list()),
        "resources/read" => resources::read(&state, rpc.id, &rpc.params).await,
        "tools/list" => json_rpc_result(rpc.id, tools::list()),
        "tools/call" => json_rpc_result(rpc.id, tools::call(&state, &rpc.params).await),
        "prompts/list" => json_rpc_result(rpc.id, prompts::list()),
        "prompts/get" => json_rpc_result(rpc.id, prompts::get(&rpc.params)),
        other => json_rpc_error(rpc.id, -32601, &format!("method not found: {other}")),
    };
    Json(response).into_response()
}

pub(crate) fn json_rpc_result(id: Option<Value>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub(crate) fn json_rpc_error(id: Option<Value>, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    })
}

/// MCP tool-result wrapper with a hard text cap.
pub(crate) fn tool_text(text: String) -> Value {
    let (text, truncated) = if text.len() > MCP_MAX_TOOL_TEXT {
        let mut cut = MCP_MAX_TOOL_TEXT;
        while !text.is_char_boundary(cut) {
            cut += 1;
        }
        (format!("{}\n…[truncated]", &text[..cut]), true)
    } else {
        (text, false)
    };
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": false,
        "_truncated": truncated,
    })
}

pub(crate) fn tool_error(message: &str) -> Value {
    json!({
        "content": [{ "type": "text", "text": message }],
        "isError": true,
    })
}