//! MCP tools: the graph capabilities an agent can call, with bounded
//! outputs. Each tool maps 1:1 onto a semweb capability:
//!
//!   search_graph    TPF fragment lookup (subject/predicate/object/graph)
//!   sparql_query    read-only SPARQL (SELECT/ASK/DESCRIBE/CONSTRUCT)
//!   get_manifest    the agent manifest (shapes, cardinalities, examples)
//!   get_topic       full topic content (NDJSON data / JSON schema)
//!   insert_triple   write path (honours the write-token policy)
//!   subscribe       WebSub subscription on behalf of a callback URL

use serde_json::{json, Value};

use crate::state::AppState;
use crate::hub::resolve_topic;

/// Tool declarations (inputSchema as JSON Schema).
pub(crate) fn list() -> Value {
    json!({
        "tools": [
            {
                "name": "search_graph",
                "description": "Triple Pattern Fragment lookup over the semantic graph. \
                                Leave positions out to treat them as wildcards. Returns \
                                compacted JSON-LD statements (one per line).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "subject": { "type": "string", "description": "URI (optional)" },
                        "predicate": { "type": "string", "description": "URI (optional)" },
                        "object": { "type": "string", "description": "URI or literal (optional)" },
                        "graph": { "type": "string", "description": "named graph URI (optional)" },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 20 }
                    }
                }
            },
            {
                "name": "sparql_query",
                "description": "Run a read-only SPARQL query (SELECT/ASK/DESCRIBE/CONSTRUCT) \
                                against the graph. Results are SPARQL JSON.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "SPARQL query string" }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "get_manifest",
                "description": "The live agent manifest: classes, predicates, cardinalities, \
                                human descriptions, SHACL shapes, schema fingerprint and \
                                example SPARQL. The planning surface -- read this first.",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "get_topic",
                "description": "Full content of a WebSub topic: '/topics/data' (all triples \
                                as NDJSON) or '/topics/schema' (the schema surface).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "topic": { "type": "string", "enum": ["/topics/data", "/topics/schema"] }
                    },
                    "required": ["topic"]
                }
            },
            {
                "name": "insert_triple",
                "description": "Write one triple. Requires the configured write token \
                                (SEMWEB_WRITE_TOKEN) when the deployment gates writes.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "subject": { "type": "string" },
                        "predicate": { "type": "string" },
                        "object": { "type": "string" },
                        "graph": { "type": "string", "description": "optional named graph" },
                        "token": { "type": "string", "description": "write bearer token (optional)" }
                    },
                    "required": ["subject", "predicate", "object"]
                }
            },
            {
                "name": "subscribe",
                "description": "Subscribe a callback URL to WebSub push notifications for \
                                '/topics/data' or '/topics/schema'. The callback must be a \
                                reachable HTTP(S) endpoint implementing the WebSub subscriber \
                                contract (echo the hub.challenge on GET).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "topic": { "type": "string", "enum": ["/topics/data", "/topics/schema"] },
                        "callback": { "type": "string" },
                        "secret": { "type": "string" },
                        "lease_seconds": { "type": "integer" }
                    },
                    "required": ["topic", "callback"]
                }
            }
        ]
    })
}

pub(crate) async fn call(state: &AppState, params: &Value) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let result = match name {
        "search_graph" => search_graph(state, &args).await,
        "sparql_query" => sparql_query(state, &args).await,
        "get_manifest" => get_manifest(state).await,
        "get_topic" => get_topic(state, &args).await,
        "insert_triple" => insert_triple(state, &args).await,
        "subscribe" => subscribe(state, &args).await,
        other => super::tool_error(&format!("unknown tool: {other}")),
    };
    result
}

async fn search_graph(state: &AppState, args: &Value) -> Value {
    let get_str = |k: &str| args.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
    let query = crate::store::FragmentQuery {
        subject: get_str("subject"),
        predicate: get_str("predicate"),
        object: get_str("object"),
        graph: get_str("graph"),
        limit: args.get("limit").and_then(Value::as_u64).unwrap_or(20).min(200),
        offset: 0,
        after: None,
    };
    match state
        .store
        .pattern_fragment(query, Some(&state.cardinality))
        .await
    {
        Ok(page) => {
            let mut body = String::new();
            for b in &page.bindings {
                if let Some(line) = crate::jsonld::binding_to_line(b, &state.prefixes) {
                    body.push_str(&serde_json::to_string(&line).unwrap_or_default());
                    body.push('\n');
                }
            }
            body.push_str(&format!(
                "\n[count_estimate: {}, has_more: {}]",
                page.total, page.has_more
            ));
            super::tool_text(body)
        }
        Err(e) => super::tool_error(&e.0),
    }
}

async fn sparql_query(state: &AppState, args: &Value) -> Value {
    let query = match args.get("query").and_then(Value::as_str) {
        Some(q) if !q.is_empty() => q,
        _ => return super::tool_error("query is required"),
    };
    match state.store.query_raw(query, "application/sparql-results+json").await {
        Ok((status, _, body)) if (200..300).contains(&status) => super::tool_text(body),
        Ok((status, _, body)) => super::tool_error(&format!("store returned {status}: {body}")),
        Err(e) => super::tool_error(&e.0),
    }
}

async fn get_manifest(state: &AppState) -> Value {
    match crate::api::manifest_document(state).await {
        Ok(doc) => super::tool_text(serde_json::to_string_pretty(&doc).unwrap_or_default()),
        Err((_, msg)) => super::tool_error(&msg),
    }
}

async fn get_topic(state: &AppState, args: &Value) -> Value {
    let topic = match args.get("topic").and_then(Value::as_str) {
        Some(t) => t,
        None => return super::tool_error("topic is required"),
    };
    let canonical = match resolve_topic(topic) {
        Some(t) => t,
        None => return super::tool_error(&format!("unknown topic: {topic}")),
    };
    match state.hub.build_topic_content(canonical).await {
        Ok(content) => super::tool_text(String::from_utf8_lossy(&content.body).into_owned()),
        Err(e) => super::tool_error(&e.0),
    }
}

async fn insert_triple(state: &AppState, args: &Value) -> Value {
    let (subject, predicate, object) = match (
        args.get("subject").and_then(Value::as_str),
        args.get("predicate").and_then(Value::as_str),
        args.get("object").and_then(Value::as_str),
    ) {
        (Some(s), Some(p), Some(o)) if !s.is_empty() && !p.is_empty() && !o.is_empty() => {
            (s, p, o)
        }
        _ => return super::tool_error("subject, predicate and object are required"),
    };
    // The write path honours SEMWEB_WRITE_TOKEN; the token may arrive as a
    // tool argument (agents receive their own credential) or, when the
    // deployment runs ungated, none is needed.
    let token = args.get("token").and_then(Value::as_str);
    if let Some(expected) = &state.write_token {
        if token != Some(expected.as_str()) {
            return super::tool_error(
                "write token required (pass it as the 'token' tool argument)",
            );
        }
    }
    let event_id = crate::hub::random_event_id();
    match state
        .store
        .insert_triple(subject, predicate, object, args.get("graph").and_then(Value::as_str))
        .await
    {
        Ok(binding) => {
            crate::api::counters_insert();
            let object_uri = object.starts_with("http://").then_some(object);
            state.cardinality.record_insert(
                subject,
                predicate,
                object_uri,
                crate::context::RDF_TYPE,
            );
            let _ = state.hub.clone().publish("/topics/data", &event_id).await;
            let triple = crate::jsonld::binding_to_line(&binding, &state.prefixes).unwrap_or(binding);
            super::tool_text(format!(
                "inserted (event {event_id}):\n{}",
                serde_json::to_string_pretty(&triple).unwrap_or_default()
            ))
        }
        Err(e) => super::tool_error(&e.0),
    }
}

async fn subscribe(state: &AppState, args: &Value) -> Value {
    let (Some(topic), Some(callback)) = (
        args.get("topic").and_then(Value::as_str),
        args.get("callback").and_then(Value::as_str),
    ) else {
        return super::tool_error("topic and callback are required");
    };
    match state
        .hub
        .clone()
        .subscribe(
            topic.to_string(),
            callback.to_string(),
            args.get("secret").and_then(Value::as_str).map(str::to_string),
            args.get("lease_seconds").and_then(Value::as_u64),
        )
        .await
    {
        // §5.1.2: 202 semantics -- accepted, verification is asynchronous.
        Ok(()) => super::tool_text(format!(
            "subscription accepted for {topic} -> {callback}; the hub will verify intent \
             by GETing the callback with hub.challenge (202-accepted)"
        )),
        Err(e) => super::tool_error(&format!("{e:?}")),
    }
}