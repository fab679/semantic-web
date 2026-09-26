//! Local trust registry: which issuer DIDs this deployment trusts (with
//! their public keys) and which credential ids are revoked.
//!
//! Sources, merged (file wins on conflict):
//!   SEMWEB_TRUSTED_ISSUERS       "did=zMk...,did2=zMk..."   (inline pairs)
//!   SEMWEB_TRUSTED_ISSUERS_PATH  JSON file {"issuers": {did: multibase},
//!                                            "revoked": [credential ids]}
//!   SEMWEB_REVOKED_CREDENTIAL_IDS  "urn:..,urn:.."          (inline ids)
//!
//! Flat revoked-id list is the documented prototype simplification
//! (a production deployment would use the Bitstring Status List).

use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Default)]
pub struct TrustedIssuers {
    issuers: HashMap<String, String>,
    revoked: HashSet<String>,
}

impl TrustedIssuers {
    /// Empty registry (self-identity, when configured, is still trusted).
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, did: String, public_key_multibase: String) -> Result<(), String> {
        if crate::trust::multikey_decode(&public_key_multibase).is_none() {
            return Err(format!(
                "not a valid Ed25519 Multikey (expected z-base58btc, 34 bytes): {public_key_multibase}"
            ));
        }
        self.issuers.insert(did, public_key_multibase);
        Ok(())
    }

    pub fn revoke(&mut self, credential_id: &str) {
        self.revoked.insert(credential_id.to_string());
    }

    pub fn public_key_for(&self, did: &str) -> Option<&str> {
        self.issuers.get(did).map(String::as_str)
    }

    pub fn is_revoked(&self, credential_id: &str) -> bool {
        self.revoked.contains(credential_id)
    }

    pub fn issuer_count(&self) -> usize {
        self.issuers.len()
    }

    pub fn revoked_count(&self) -> usize {
        self.revoked.len()
    }

    /// Build from the environment variables described in the module docs.
    /// Missing vars produce an empty registry; malformed entries are
    /// logged and skipped (a bad entry must not take the service down).
    pub fn from_env() -> Self {
        let mut reg = Self::new();

        for pair in std::env::var("SEMWEB_TRUSTED_ISSUERS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            match pair.split_once('=') {
                Some((did, pk)) if !did.is_empty() => {
                    if let Err(e) = reg.insert(did.to_string(), pk.to_string()) {
                        tracing::warn!("SEMWEB_TRUSTED_ISSUERS: skipping {did}: {e}");
                    }
                }
                _ => tracing::warn!("SEMWEB_TRUSTED_ISSUERS: skipping malformed entry {pair:?} (want did=multibase)"),
            }
        }

        if let Some(path) = std::env::var("SEMWEB_TRUSTED_ISSUERS_PATH")
            .ok()
            .filter(|p| !p.is_empty())
        {
            match std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            {
                Some(doc) => {
                    if let Some(map) = doc["issuers"].as_object() {
                        for (did, pk) in map {
                            match pk.as_str().map(str::to_string) {
                                Some(pk) => {
                                    if let Err(e) = reg.insert(did.clone(), pk) {
                                        tracing::warn!("registry file: skipping {did}: {e}");
                                    }
                                }
                                None => {
                                    tracing::warn!("registry file: {did} has no string publicKeyMultibase")
                                }
                            }
                        }
                    }
                    if let Some(ids) = doc["revoked"].as_array() {
                        for id in ids {
                            if let Some(id) = id.as_str() {
                                reg.revoke(id);
                            }
                        }
                    }
                }
                None => tracing::warn!("SEMWEB_TRUSTED_ISSUERS_PATH={path}: unreadable or not JSON"),
            }
        }

        for id in std::env::var("SEMWEB_REVOKED_CREDENTIAL_IDS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            reg.revoke(id);
        }

        reg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk() -> String {
        // deterministic valid Multikey (0xed01 ++ 32 bytes)
        let mut key = [0u8; 32];
        key[0] = 1;
        crate::trust::multikey_encode(&key)
    }

    #[test]
    fn insert_validates_multikey_shape() {
        let mut reg = TrustedIssuers::new();
        assert!(reg.insert("did:web:x".into(), "z6MkNotAKey".into()).is_err());
        assert!(reg.insert("did:web:x".into(), mk()).is_ok());
        assert_eq!(reg.public_key_for("did:web:x"), Some(mk().as_str()));
        assert_eq!(reg.public_key_for("did:web:other"), None);
    }

    #[test]
    fn revocation_membership() {
        let mut reg = TrustedIssuers::new();
        reg.revoke("urn:claim:1");
        assert!(reg.is_revoked("urn:claim:1"));
        assert!(!reg.is_revoked("urn:claim:2"));
    }
}