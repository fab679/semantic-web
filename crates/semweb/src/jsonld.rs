//! RDF term -> compacted JSON-LD line conversion.
//!
//! Converts SPARQL JSON bindings into NDJSON-friendly JSON-LD lines:
//! one triple per line, one minimal JSON-LD statement each.
//!
//! Design note: one *triple* per line, not one grouped *node* object.
//! Grouping would force the server to buffer all triples of a subject
//! before emitting anything and kill streaming for patterns that match
//! millions of subjects. Grouping is a cheap client-side operation;
//! ungrouping a large buffered node server-side is not.

use serde_json::{json, Value};

use crate::context::{PrefixMap, RDF_TYPE};

/// Convert one SPARQL JSON results binding (`?s ?p ?o`) into the compact
/// JSON-LD line, compacting predicates, classes and datatypes through the
/// (runtime extensible) prefix map. Returns None if the binding does not
/// look like a triple (defensive; should not happen).
pub fn binding_to_line(binding: &Value, prefixes: &PrefixMap) -> Option<Value> {
    let subject = binding.get("s")?;
    let predicate = binding.get("p")?;
    let object = binding.get("o")?;

    let subject_id = match subject.get("type")?.as_str()? {
        "uri" => subject.get("value")?.as_str()?.to_string(),
        "bnode" => format!("_:{}", subject.get("value")?.as_str()?),
        _ => return None,
    };

    let pred_uri = predicate.get("value")?.as_str()?;

    // rdf:type triples compact to "@type" (standard JSON-LD convention),
    // with the object compacted to a short name too.
    if pred_uri == RDF_TYPE && object.get("type").and_then(Value::as_str) == Some("uri") {
        let type_short = prefixes.compact(object.get("value")?.as_str()?);
        return Some(json!({ "@id": subject_id, "@type": type_short }));
    }

    let short_pred = prefixes.compact(pred_uri);
    Some(json!({ "@id": subject_id, short_pred: object_value(object, prefixes) }))
}

/// Render the object term of a binding. Datatype/language must survive:
/// losing them would downgrade typed literals to plain strings and break
/// RDF fidelity over the wire. Datatypes are prefix-compacted like
/// predicates.
fn object_value(obj: &Value, prefixes: &PrefixMap) -> Value {
    let term_type = obj.get("type").and_then(Value::as_str).unwrap_or("");
    let value = obj.get("value").cloned().unwrap_or(Value::Null);

    match term_type {
        "uri" => json!({ "@id": value.as_str().unwrap_or_default() }),
        "bnode" => json!({ "@id": format!("_:{}", value.as_str().unwrap_or_default()) }),
        _ => {
            if let Some(lang) = obj.get("xml:lang").and_then(Value::as_str) {
                json!({ "@value": value, "@language": lang })
            } else if let Some(dt) = obj.get("datatype").and_then(Value::as_str) {
                if dt != "http://www.w3.org/2001/XMLSchema#string" {
                    json!({ "@value": value, "@type": prefixes.compact(dt) })
                } else {
                    value
                }
            } else {
                value
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::PrefixMap;
    use serde_json::json;

    fn prefixes() -> PrefixMap {
        PrefixMap::from_env()
    }

    #[test]
    fn type_triples_compact_to_at_type() {
        let b = json!({
            "s": {"type": "uri", "value": "http://example.org/alice"},
            "p": {"type": "uri", "value": crate::context::RDF_TYPE},
            "o": {"type": "uri", "value": "http://xmlns.com/foaf/0.1/Person"}
        });
        let line = binding_to_line(&b, &PrefixMap::from_env()).unwrap();
        assert_eq!(line, json!({"@id": "http://example.org/alice", "@type": "foaf:Person"}));
    }

    #[test]
    fn typed_literals_keep_datatype_compacted() {
        let b = json!({
            "s": {"type": "uri", "value": "http://example.org/acme"},
            "p": {"type": "uri", "value": "http://schema.org/foundingDate"},
            "o": {"type": "literal", "value": "2001-04-03",
                  "datatype": "http://www.w3.org/2001/XMLSchema#date"}
        });
        let line = binding_to_line(&b, &PrefixMap::from_env()).unwrap();
        assert_eq!(
            line,
            json!({"@id": "http://example.org/acme",
                   "schema:foundingDate": {"@value": "2001-04-03", "@type": "xsd:date"}})
        );
    }

    #[test]
    fn unknown_namespaces_stay_full_uris() {
        let b = json!({
            "s": {"type": "uri", "value": "http://example.org/dave"},
            "p": {"type": "uri", "value": "http://example.org/vocab/age"},
            "o": {"type": "literal", "value": "30"}
        });
        let line = binding_to_line(&b, &PrefixMap::from_env()).unwrap();
        assert_eq!(line["http://example.org/vocab/age"], json!("30"));
    }

    #[test]
    fn bnodes_and_lang_literals_are_preserved() {
        let pm = PrefixMap::empty();
        let b = json!({
            "s": {"type": "bnode", "value": "b0"},
            "p": {"type": "uri", "value": "http://xmlns.com/foaf/0.1/name"},
            "o": {"type": "literal", "value": "X", "xml:lang": "en"}
        });
        let line = binding_to_line(&b, &pm).unwrap();
        assert_eq!(line["@id"], json!("_:b0"));
        assert_eq!(line["foaf:name"], json!({"@value": "X", "@language": "en"}));
    }
}
