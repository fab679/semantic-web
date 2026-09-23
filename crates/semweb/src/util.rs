//! Small shared helpers with no external dependencies of their own.

/// Fingerprint of the ontology surface: sha256 over the sorted class and
/// predicate URI lists. Carried on the agent manifest and on
/// /topics/schema content, so consumers detect missed schema-change
/// events and diff safely.
pub fn schema_fingerprint(class_uris: &[String], predicate_uris: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"classes\n");
    for uri in class_uris {
        hasher.update(uri.as_bytes());
        hasher.update(b"\n");
    }
    hasher.update(b"predicates\n");
    for uri in predicate_uris {
        hasher.update(uri.as_bytes());
        hasher.update(b"\n");
    }
    format!("sha256:{}", hex(&hasher.finalize()))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_stable_and_order_insensitive_within_sorted_input() {
        let a = schema_fingerprint(&["http://x/A".into()], &["http://x/p".into()]);
        let b = schema_fingerprint(&["http://x/A".into()], &["http://x/p".into()]);
        assert_eq!(a, b);
        let c = schema_fingerprint(&["http://x/B".into()], &["http://x/p".into()]);
        assert_ne!(a, c);
        assert!(a.starts_with("sha256:"));
    }
}