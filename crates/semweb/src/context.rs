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

/// Runtime-extensible prefix table: standards prefixes plus any
/// SEMWEB_EXTRA_PREFIXES entries from the environment.
pub struct PrefixMap {
    extra: Vec<(String, String)>,
}

impl PrefixMap {
    /// An empty map (no runtime extras) -- tests.
    #[allow(dead_code)]
    pub fn empty() -> Self {
        PrefixMap { extra: Vec::new() }
    }

    /// Build from SEMWEB_EXTRA_PREFIXES="name=namespace,name=namespace".
    /// Malformed entries are logged and skipped; standard prefixes are
    /// never shadowed by extras (check standards first).
    /// Malformed entries are logged and skipped; standard prefixes are
    /// never shadowed by extras (check standards first).
    pub fn from_env() -> Self {
        let mut extra = Vec::new();
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
        PrefixMap { extra }
    }

    /// Compact a URI to `prefix:local` when its namespace is known
    /// (standards first, then runtime extras); otherwise return it
    /// unchanged.
    pub fn compact(&self, uri: &str) -> String {
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

    /// The JSON-LD `@context` document for the namespaces currently in
    /// use. Only known prefixes are emitted; namespaces without a
    /// registered prefix appear as full URIs in the data, which the
    /// context need not (and cannot honestly) name.
    pub fn context_document(&self, namespaces: &[String]) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        map.insert("type".into(), serde_json::Value::String("@type".into()));
        for ns in namespaces {
            if let Some(prefix) = self.prefix_for(ns) {
                map.insert(prefix, serde_json::Value::String(ns.clone()));
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