//! Namespace-prefix compaction.
//!
//! This replaces the earlier hardcoded per-term
//! vocabulary table. A production service must not bake domain terms
//! (foaf:name, schema:worksFor, ...) into compiled code: the ontology is
//! live data, and the surface has to stay current as it changes.
//!
//! What IS legitimately fixed: conventional prefixes for *standard*,
//! published vocabularies (RDF, RDFS, OWL, XSD, SHACL, FOAF, schema.org,
//! Dublin Core, SKOS). Those are ecosystem constants -- the same table
//! every Turtle file, SPARQL query and JSON-LD context assumes -- and
//! prefixing compacts *any* term from those vocabularies automatically
//! (foaf:mbox, schema:address, rdfs:seeAlso, ...) with zero code changes.
//!
//! Custom/private namespaces are not guessed: unknown namespaces are
//! served as full URIs (honest, stable, no invented names), and can be
//! given friendly prefixes at runtime via SEMWEB_EXTRA_PREFIXES
//! (comma-separated `name=namespace` pairs) without recompiling.
//!
//! The `@context` served at /context.jsonld is generated live from the
//! namespaces currently in use in the store -- same no-build-step
//! principle as the self-description.

pub const RDF_NS: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
pub const RDFS_NS: &str = "http://www.w3.org/2000/01/rdf-schema#";
pub const OWL_NS: &str = "http://www.w3.org/2002/07/owl#";
pub const XSD_NS: &str = "http://www.w3.org/2001/XMLSchema#";
pub const SHACL_NS: &str = "http://www.w3.org/ns/shacl#";
pub const FOAF_NS: &str = "http://xmlns.com/foaf/0.1/";
pub const SCHEMA_NS: &str = "http://schema.org/";
pub const DCTERMS_NS: &str = "http://purl.org/dc/terms/";
pub const SKOS_NS: &str = "http://www.w3.org/2004/02/skos/core#";

/// The rdf:type term, special-cased in JSON-LD output as "@type"
/// (standard JSON-LD convention, not domain vocabulary).
pub const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// Conventional prefixes for standards namespaces.
const STD_PREFIXES: &[(&str, &str)] = &[
    ("rdf", RDF_NS),
    ("rdfs", RDFS_NS),
    ("owl", OWL_NS),
    ("xsd", XSD_NS),
    ("sh", SHACL_NS),
    ("foaf", FOAF_NS),
    ("schema", SCHEMA_NS),
    ("dcterms", DCTERMS_NS),
    ("skos", SKOS_NS),
];

/// Runtime-extensible compaction table:
///   - prefixes for standard namespaces (always) + SEMWEB_EXTRA_PREFIXES
///   - term aliases (SEMWEB_TERM_ALIASES): bare friendly names for exact
///     URIs, e.g. name=http://xmlns.com/foaf/0.1/name -- the "friendly
///     names" layer, still zero hardcoded terms (operators choose).
pub struct PrefixMap {
    extra: Vec<(String, String)>,
    terms: Vec<(String, String)>,
}

impl PrefixMap {
    /// An empty map (no runtime extras) -- tests.
    #[allow(dead_code)]
    pub fn empty() -> Self {
        PrefixMap {
            extra: Vec::new(),
            terms: Vec::new(),
        }
    }

    /// Build from SEMWEB_EXTRA_PREFIXES="name=namespace,name=namespace".
    /// Malformed entries are logged and skipped; standard prefixes are
    /// never shadowed by extras (check standards first).
    /// Malformed entries are logged and skipped; standard prefixes are
    /// never shadowed by extras (check standards first).
    pub fn from_env() -> Self {
        let mut extra = Vec::new();
        let mut terms = Vec::new();
        // Term aliases: bare friendly names for exact URIs.
        if let Ok(raw) = std::env::var("SEMWEB_TERM_ALIASES") {
            for entry in raw.split(',') {
                let entry = entry.trim();
                if entry.is_empty() {
                    continue;
                }
                match entry.split_once('=') {
                    Some((name, uri))
                        if !name.is_empty()
                            && uri.starts_with("http")
                            && !name.contains(':') =>
                    {
                        terms.push((name.to_string(), uri.to_string()));
                    }
                    _ => tracing::warn!("ignoring malformed term alias: {entry}"),
                }
            }
        }
        if let Ok(raw) = std::env::var("SEMWEB_EXTRA_PREFIXES") {
            for entry in raw.split(',') {
                let entry = entry.trim();
                if entry.is_empty() {
                    continue;
                }
                match entry.split_once('=') {
                    Some((name, ns))
                        if !name.is_empty() && ns.starts_with("http") =>
                    {
                        extra.push((name.to_string(), ns.to_string()));
                    }
                    _ => tracing::warn!("ignoring malformed prefix entry: {entry}"),
                }
            }
        }
        PrefixMap { extra, terms }
    }

    /// Compact a URI to `prefix:local` when its namespace is known
    /// (standards first, then runtime extras); otherwise return it
    /// unchanged. An exact term alias (SEMWEB_TERM_ALIASES) wins over the
    /// prefix form: operators choose bare friendly names for the terms
    /// their consumers read most.
    pub fn compact(&self, uri: &str) -> String {
        for (alias, alias_uri) in &self.terms {
            if alias_uri == uri {
                return alias.clone();
            }
        }
        match split_uri(uri) {
            Some((ns, local)) => {
                for (name, std_ns) in STD_PREFIXES {
                    if *std_ns == ns {
                        return format!("{name}:{local}");
                    }
                }
                for (name, extra_ns) in &self.extra {
                    if extra_ns == ns {
                        return format!("{name}:{local}");
                    }
                }
                uri.to_string()
            }
            None => uri.to_string(),
        }
    }

    /// Registered (prefix, namespace) pairs for the namespaces currently
    /// in use -- what /manifest exposes so agents can write valid SPARQL
    /// PREFIX declarations.
    pub fn prefix_pairs(&self, namespaces: &[String]) -> Vec<(String, String)> {
        let mut pairs: Vec<(String, String)> = namespaces
            .iter()
            .filter_map(|ns| self.prefix_for(ns).map(|p| (p, ns.clone())))
            .collect();
        pairs.sort();
        pairs
    }

    /// The prefix for a namespace, if one is registered.
    fn prefix_for(&self, ns: &str) -> Option<String> {
        for (name, std_ns) in STD_PREFIXES {
            if *std_ns == ns {
                return Some((*name).to_string());
            }
        }
        for (name, extra_ns) in &self.extra {
            if extra_ns == ns {
                return Some(name.clone());
            }
        }
        None
    }

    /// The JSON-LD `@context` for the namespaces currently in use, plus
    /// term aliases as JSON-LD term definitions ({"name": {"@id": uri}}),
    /// so consumers can round-trip both. Only registered prefixes are
    /// emitted; namespaces without a registered prefix appear as full
    /// URIs in the data, which the context need not (and cannot honestly)
    /// name. Alias names that collide with a prefix name are skipped
    /// (prefixes win -- JSON-LD keywords aside, one key one meaning).
    pub fn context_document(&self, namespaces: &[String]) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        map.insert("type".into(), serde_json::Value::String("@type".into()));
        for ns in namespaces {
            if let Some(prefix) = self.prefix_for(ns) {
                map.insert(prefix, serde_json::Value::String(ns.clone()));
            }
        }
        for (alias, uri) in &self.terms {
            if !map.contains_key(alias) {
                map.insert(
                    alias.clone(),
                    serde_json::json!({ "@id": uri }),
                );
            } else {
                tracing::warn!("term alias '{alias}' collides with a prefix name; skipped");
            }
        }
        serde_json::json!({ "@context": serde_json::Value::Object(map) })
    }
}

/// Split a URI into (namespace, local-name): at `#` when present, else at
/// the last `/`. Returns None when there is no compactable local part
/// (e.g. the URI *is* a namespace).
pub fn split_uri(uri: &str) -> Option<(&str, &str)> {
    if let Some(i) = uri.rfind('#') {
        if i + 1 < uri.len() {
            return Some((&uri[..=i], &uri[i + 1..]));
        }
    }
    if let Some(i) = uri.rfind('/') {
        if i + 1 < uri.len() {
            return Some((&uri[..=i], &uri[i + 1..]));
        }
    }
    None
}

/// Distinct namespaces in use, derived from the class and predicate URIs
/// the store reports. Sorted for stable output; this is what
/// /context.jsonld is generated from.
pub fn namespaces_of(class_uris: &[String], predicate_uris: &[String]) -> Vec<String> {
    let mut ns: Vec<String> = class_uris
        .iter()
        .chain(predicate_uris.iter())
        .filter_map(|u| split_uri(u).map(|(ns, _)| ns.to_string()))
        .collect();
    ns.sort();
    ns.dedup();
    ns
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map_with(aliases: &[(&str, &str)]) -> PrefixMap {
        let mut m = PrefixMap::empty();
        m.terms = aliases
            .iter()
            .map(|(a, u)| (a.to_string(), u.to_string()))
            .collect();
        m
    }

    #[test]
    fn aliases_win_over_prefixes_for_exact_uris() {
        let m = map_with(&[("name", "http://xmlns.com/foaf/0.1/name")]);
        // exact match -> bare alias
        assert_eq!(m.compact("http://xmlns.com/foaf/0.1/name"), "name");
        // other foaf terms keep CURIE form
        assert_eq!(m.compact("http://xmlns.com/foaf/0.1/mbox"), "foaf:mbox");
        // non-matching URIs unchanged
        assert_eq!(m.compact("http://example.org/vocab/age"), "http://example.org/vocab/age");
    }

    #[test]
    fn aliases_serve_as_jsonld_term_definitions() {
        let m = map_with(&[("name", "http://xmlns.com/foaf/0.1/name")]);
        let doc = m.context_document(&["http://xmlns.com/foaf/0.1/".to_string()]);
        let ctx = &doc["@context"];
        // prefix entry still there
        assert_eq!(ctx["foaf"], serde_json::json!("http://xmlns.com/foaf/0.1/"));
        // alias as a JSON-LD term definition (round-trippable)
        assert_eq!(
            ctx["name"],
            serde_json::json!({"@id": "http://xmlns.com/foaf/0.1/name"})
        );
    }
}
