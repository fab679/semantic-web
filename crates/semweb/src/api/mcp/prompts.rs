//! MCP prompts: guided templates an agent (or human developer) can pull
//! before working with the graph.

use serde_json::{json, Value};

/// Prompt declarations.
pub(crate) fn list() -> Value {
    json!({
        "prompts": [
            {
                "name": "explore_graph",
                "description": "Guided exploration: read the manifest, then inspect the data.",
                "arguments": [
                    { "name": "focus", "description": "Optional class or topic to focus on",
                      "required": false }
                ]
            },
            {
                "name": "answer_from_graph",
                "description": "Answer a user question using only the graph: manifest for \
                                grounding, fragments/SPARQL for facts.",
                "arguments": [
                    { "name": "question", "description": "The user's question", "required": true }
                ]
            }
        ]
    })
}

/// Prompt content. Messages use the MCP {role, content:{type,text}} shape.
pub(crate) fn get(params: &Value) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let focus = params
        .pointer("/arguments/focus")
        .and_then(Value::as_str)
        .unwrap_or("the whole graph");
    let question = params
        .pointer("/arguments/question")
        .and_then(Value::as_str)
        .unwrap_or("");
    let result = match name {
        "explore_graph" => json!({
            "messages": [{
                "role": "user",
                "content": { "type": "text", "text": format!(
                    "You are exploring a semantic knowledge graph.\n\
                     1. Call get_manifest first: it lists the classes, predicates, their \
                     human descriptions, SHACL constraints and a schema fingerprint.\n\
                     2. Pick the classes relevant to {focus} and use search_graph with those \
                     predicates, or run the manifest's exampleQuery via sparql_query.\n\
                     3. Report what exists, citing the URIs you actually saw.\n\
                     Never invent terms that are not in the manifest."
                )}
            }]
        }),
        "answer_from_graph" => json!({
            "messages": [{
                "role": "user",
                "content": { "type": "text", "text": format!(
                    "Answer the question: \"{question}\"\n\
                     Use only the semantic graph: get_manifest for grounding (what terms \
                     exist and what they mean), search_graph/sparql_query for facts. \
                     If the graph does not contain the answer, say so explicitly."
                )}
            }]
        }),
        other => {
            return super::json_rpc_error(None, -32602, &format!("unknown prompt: {other}"))
        }
    };
    result
}