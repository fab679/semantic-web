//! Trust layer: verifiable provenance for the graph surface.
//!
//! Claims and documents can carry a Data Integrity-style Ed25519 proof,
//! and readers verify before use -- signature, issuer trust, temporal
//! window, revocation/supersession -- instead of citing unverified data.
//!
//! Philosophy preserved: only existing standards are used (W3C VC 2.0
//! shape, DataIntegrityProof, did:web, Multikey) and the layer is strictly
//! opt-in. Without SEMWEB_SIGNING_KEY nothing is signed and the surface
//! behaves exactly as before.
//!
//! Prototype simplifications (documented in docs/trust.md): sorted-key
//! JSON canonicalization instead of URDNA2015, and signature bytes
//! defined as canonical(doc sans proof) || canonical(proof sans
//! proofValue).

pub mod registry;

use serde_json::{json, Map, Value};

use crate::state::SigningIdentity;

/// Proof cryptosuite id (the prototype suite documented in docs/trust.md).
pub const CRYPTOSUITE: &str = "ed25519-canonicaljson-2025-prototype";
pub const CONTEXT_CREDENTIALS: &str = "https://www.w3.org/ns/credentials/v2";
const MULTIKEY_ED25519_PREFIX: [u8; 2] = [0xed, 0x01];

// ---------------------------------------------------------------------------
// canonical JSON (sorted keys, compact) -- the prototype canonicalization
// ---------------------------------------------------------------------------

/// Deterministic serialization: object keys sorted recursively (UTF-8
/// byte order), no whitespace. serde_json's preserve_order feature keeps
/// insertion order, so sorting is done explicitly here.
pub fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let body = keys
                .iter()
                .map(|k| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap_or_default(),
                        canonical_json(&map[*k])
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{body}}}")
        }
        Value::Array(items) => {
            let body = items
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",");
            format!("[{body}]")
        }
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

// ---------------------------------------------------------------------------
// base58btc + Multikey (multicodec 0xed01 for Ed25519 public keys)
// ---------------------------------------------------------------------------

const B58_ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

pub fn b58_encode(bytes: &[u8]) -> String {
    let mut digits: Vec<u8> = Vec::with_capacity(bytes.len() * 2);
    for &b in bytes {
        let mut carry = b as u32;
        for d in digits.iter_mut().rev() {
            carry += (*d as u32) << 8;
            *d = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.insert(0, (carry % 58) as u8);
            carry /= 58;
        }
    }
    let zeros = bytes.iter().take_while(|&&b| b == 0).count();
    let mut out = "1".repeat(zeros);
    for d in &digits {
        out.push(B58_ALPHABET[*d as usize] as char);
    }
    if out.is_empty() {
        out.push('1');
    }
    out
}

pub fn b58_decode(s: &str) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::new();
    for &c in bytes {
        let val = B58_ALPHABET.iter().position(|&b| b == c)? as u32;
        let mut carry = val;
        for b in out.iter_mut().rev() {
            carry += (*b as u32) * 58;
            *b = (carry % 256) as u8;
            carry /= 256;
        }
        while carry > 0 {
            out.insert(0, (carry % 256) as u8);
            carry /= 256;
        }
    }
    let zeros = bytes.iter().take_while(|&&c| c == b'1').count();
    let mut result = vec![0u8; zeros];
    result.extend_from_slice(&out);
    Some(result)
}

/// Multikey encoding: 'z' (base58btc) over multicodec 0xed01 ++ key bytes,
/// e.g. "z6Mk..." for an Ed25519 public key.
pub fn multikey_encode(public_key: &[u8; 32]) -> String {
    let mut prefixed = Vec::with_capacity(34);
    prefixed.extend_from_slice(&MULTIKEY_ED25519_PREFIX);
    prefixed.extend_from_slice(public_key);
    format!("z{}", b58_encode(&prefixed))
}

pub fn multikey_decode(multibase: &str) -> Option<[u8; 32]> {
    let raw = b58_decode(multibase.strip_prefix('z')?)?;
    if raw.len() == 34 && raw[..2] == MULTIKEY_ED25519_PREFIX {
        let mut key = [0u8; 32];
        key.copy_from_slice(&raw[2..]);
        Some(key)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// RFC3339 (UTC, seconds) -- hand-rolled to stay dependency-free
// ---------------------------------------------------------------------------

/// Days since the Unix epoch -> (year, month, day). Howard Hinnant's
/// civil_from_days.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = if m > 2 { m - 3 } else { m + 9 } as u64;
    let doy = (153 * mp + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

/// Format epoch seconds as RFC3339 UTC ("2026-09-26T12:34:56Z").
pub fn rfc3339(epoch_secs: i64) -> String {
    let days = epoch_secs.div_euclid(86_400);
    let sod = epoch_secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    )
}

pub fn now_rfc3339() -> String {
    rfc3339(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
    )
}

/// Tolerant RFC3339 parser -> epoch seconds. Accepts 'T' or space
/// separator, optional fractional seconds, and 'Z' or ±HH:MM offsets.
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    let s = s.trim();
    let (date, rest) = s.split_once(['T', ' '])?;
    let rest = rest.trim_end_matches(['Z', 'z']);
    let (time, offset) = if let Some(pos) = rest.find(['+', '-']) {
        let (t, off) = rest.split_at(pos);
        let sign: i64 = if off.starts_with('-') { -1 } else { 1 };
        let (oh, om) = off[1..].split_once(':').unwrap_or((&off[1..], "0"));
        (
            t,
            sign * (oh.parse::<i64>().ok()? * 3600 + om.parse::<i64>().ok()? * 60),
        )
    } else {
        (rest, 0)
    };
    let mut dp = date.split('-');
    let y: i64 = dp.next()?.parse().ok()?;
    let m: u32 = dp.next()?.parse().ok()?;
    let d: u32 = dp.next()?.parse().ok()?;
    let mut tp = time.split(':');
    let h: i64 = tp.next().unwrap_or("0").parse().ok()?;
    let mi: i64 = tp.next().unwrap_or("0").parse().ok()?;
    let sec: i64 = tp
        .next()
        .unwrap_or("0")
        .split('.')
        .next()
        .unwrap_or("0")
        .parse()
        .ok()?;
    Some(days_from_civil(y, m, d) * 86_400 + h * 3600 + mi * 60 + sec - offset)
}

// ---------------------------------------------------------------------------
// proof creation (Data Integrity-style)
// ---------------------------------------------------------------------------

/// Signature bytes = canonical(doc without proof) || canonical(proof
/// without proofValue). Sign / verify must agree on exactly this.
fn signing_bytes(doc: &Value, proof: &Value) -> Vec<u8> {
    let mut doc_sans = doc.clone();
    if let Value::Object(map) = &mut doc_sans {
        map.remove("proof");
    }
    let mut proof_sans = proof.clone();
    if let Value::Object(map) = &mut proof_sans {
        map.remove("proofValue");
    }
    let mut bytes = canonical_json(&doc_sans).into_bytes();
    bytes.extend_from_slice(canonical_json(&proof_sans).as_bytes());
    bytes
}

/// Build the proof object for `doc` and attach it (mutates the doc).
pub fn attach_proof(doc: &mut Value, signer: &SigningIdentity) {
    let proof = json!({
        "type": "DataIntegrityProof",
        "cryptosuite": CRYPTOSUITE,
        "verificationMethod": format!("{}#key-1", signer.did),
        "created": now_rfc3339(),
        "proofPurpose": "assertionMethod",
    });
    use ed25519_dalek::Signer;
    let sig = signer.key.sign(&signing_bytes(doc, &proof));
    let mut proof_obj = proof.as_object().cloned().unwrap_or_else(Map::new);
    proof_obj.insert(
        "proofValue".into(),
        Value::String(format!("z{}", b58_encode(&sig.to_bytes()))),
    );
    if let Value::Object(map) = doc {
        map.insert("proof".into(), Value::Object(proof_obj));
    }
}

/// Wrap a claim into a VerifiableCredential (VC 2.0 shape) with a proof,
/// in the attestation format documented in docs/trust.md (id,
/// validFrom/validUntil, supersedes, credentialSubject).
pub fn issue_credential(
    signer: &SigningIdentity,
    credential_subject: Value,
    valid_until: Option<String>,
    supersedes: Option<String>,
    id: Option<String>,
) -> Value {
    let mut vc = json!({
        "@context": [CONTEXT_CREDENTIALS],
        "type": ["VerifiableCredential"],
        "issuer": signer.did,
        "validFrom": now_rfc3339(),
        "credentialSubject": credential_subject,
    });
    if let Value::Object(map) = &mut vc {
        if let Some(id) = id {
            map.insert("id".into(), Value::String(id));
        }
        if let Some(u) = valid_until {
            map.insert("validUntil".into(), Value::String(u));
        }
    }
    if let Some(s) = supersedes {
        if let Value::Object(sub) = &mut vc["credentialSubject"] {
            sub.insert("supersedes".into(), Value::String(s));
        }
    }
    attach_proof(&mut vc, signer);
    vc
}

// ---------------------------------------------------------------------------
// verification: the four gates (signature / trust / temporal / revocation)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Verdict {
    pub verified: bool,
    pub shape_ok: bool,
    pub signature_ok: bool,
    pub issuer_trusted: bool,
    /// None = not applicable (no validity window on the document).
    pub temporal_ok: Option<bool>,
    /// None = not applicable (no id/revocation data on the document).
    pub revocation_ok: Option<bool>,
    pub issuer: Option<String>,
    pub details: Vec<String>,
}

impl Verdict {
    pub fn to_json(&self) -> Value {
        json!({
            "verified": self.verified,
            "gates": {
                "shape": self.shape_ok,
                "signature": self.signature_ok,
                "issuerTrusted": self.issuer_trusted,
                "temporal": self.temporal_ok,
                "revocation": self.revocation_ok,
            },
            "issuer": self.issuer,
            "details": self.details,
        })
    }
}

/// What the verifier knows: the local trust registry plus (optionally)
/// this deployment's own identity, which is trusted by definition.
pub struct TrustContext<'a> {
    pub registry: &'a registry::TrustedIssuers,
    pub self_identity: Option<&'a SigningIdentity>,
    pub now: i64,
}

fn is_vc(doc: &Value) -> bool {
    doc["type"]
        .as_array()
        .map(|a| a.iter().any(|t| t == "VerifiableCredential"))
        .unwrap_or(false)
}

/// Verify any Data Integrity-signed JSON document (a VC or the signed
/// manifest) against the four gates.
pub fn verify_document(doc: &Value, ctx: &TrustContext) -> Verdict {
    let mut details = Vec::new();

    // Gate 1: shape -- a well-formed DataIntegrityProof must be present.
    let proof = doc.get("proof").cloned().unwrap_or(Value::Null);
    let Some(proof_obj) = proof.as_object() else {
        return Verdict {
            verified: false,
            shape_ok: false,
            signature_ok: false,
            issuer_trusted: false,
            temporal_ok: None,
            revocation_ok: None,
            issuer: None,
            details: vec!["no proof object present; document is unsigned".into()],
        };
    };
    let proof_value = proof_obj.get("proofValue").and_then(Value::as_str).unwrap_or("");
    let vm = proof_obj
        .get("verificationMethod")
        .and_then(Value::as_str)
        .unwrap_or("");
    let cryptosuite = proof_obj
        .get("cryptosuite")
        .and_then(Value::as_str)
        .unwrap_or("");
    let purpose = proof_obj
        .get("proofPurpose")
        .and_then(Value::as_str)
        .unwrap_or("");
    let shape_ok = !proof_value.is_empty()
        && !vm.is_empty()
        && cryptosuite == CRYPTOSUITE
        && purpose == "assertionMethod";
    if !shape_ok {
        details.push(format!(
            "malformed proof (cryptosuite={cryptosuite:?}, purpose={purpose:?})"
        ));
        return Verdict {
            verified: false,
            shape_ok: false,
            signature_ok: false,
            issuer_trusted: false,
            temporal_ok: None,
            revocation_ok: None,
            issuer: doc.get("issuer").and_then(Value::as_str).map(str::to_string),
            details,
        };
    }

    // Gate 2: issuer trust. The DID is the part of verificationMethod
    // before the '#' fragment; its key must come from the local trusted
    // registry or be this deployment's own identity.
    let did = vm.split('#').next().unwrap_or("");
    let issuer = doc
        .get("issuer")
        .and_then(Value::as_str)
        .unwrap_or(did)
        .to_string();
    let self_match = ctx
        .self_identity
        .map(|s| s.did == did)
        .unwrap_or(false);
    let registry_key = ctx.registry.public_key_for(did);
    let (issuer_trusted, verifying_key) = if self_match {
        (
            true,
            ctx.self_identity.map(|s| s.key.verifying_key()),
        )
    } else {
        match registry_key.and_then(multikey_decode) {
            Some(bytes) => {
                let vk = ed25519_dalek::VerifyingKey::from_bytes(&bytes).ok();
                (vk.is_some(), vk)
            }
            None => (false, None),
        }
    };
    if !issuer_trusted {
        details.push(format!(
            "issuer {did} is not in the trusted registry (and not this deployment)"
        ));
    }
    if is_vc(doc) && doc.get("issuer").and_then(Value::as_str) != Some(did) {
        details.push("issuer field disagrees with proof verificationMethod".into());
    }

    // Gate 3: signature over canonical bytes.
    use ed25519_dalek::Verifier;
    let signature_ok = verifying_key
        .zip(b58_decode(proof_value.strip_prefix('z').unwrap_or(proof_value)))
        .and_then(|(vk, sig_bytes)| {
            let arr: [u8; 64] = sig_bytes.as_slice().try_into().ok()?;
            let sig = ed25519_dalek::Signature::from_bytes(&arr);
            vk.verify(&signing_bytes(doc, &proof), &sig).ok()
        })
        .is_some();
    if !signature_ok {
        details.push("signature does not verify over canonical bytes".into());
    }

    // Gate 4a: temporal window (VC validity; VC 2.0 names with 1.1 fallbacks).
    let temporal_ok = if is_vc(doc) {
        let valid_from = doc
            .get("validFrom")
            .or_else(|| doc.get("issuanceDate"))
            .and_then(Value::as_str);
        let valid_until = doc
            .get("validUntil")
            .or_else(|| doc.get("expirationDate"))
            .and_then(Value::as_str);
        if valid_from.is_none() && valid_until.is_none() {
            None
        } else {
            let from_ok = valid_from
                .map(parse_rfc3339)
                .map(|t| t.map(|t| t <= ctx.now).unwrap_or(false))
                .unwrap_or(true);
            let until_ok = valid_until
                .map(parse_rfc3339)
                .map(|t| t.map(|t| t > ctx.now).unwrap_or(false))
                .unwrap_or(true);
            Some(from_ok && until_ok)
        }
    } else {
        None
    };
    if temporal_ok == Some(false) {
        details.push("validity window does not contain the current time".into());
    }

    // Gate 4b: revocation / supersession (flat local lists -- the
    // documented prototype simplification).
    let doc_id = doc.get("id").and_then(Value::as_str);
    let superseded_by = doc
        .get("credentialSubject")
        .and_then(|s| s.get("supersededBy"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let revocation_ok = if doc_id.is_some() || superseded_by.is_some() {
        let revoked = doc_id.map(|id| ctx.registry.is_revoked(id)).unwrap_or(false)
            || superseded_by.is_some();
        if revoked {
            details.push("credential is revoked or superseded".into());
        }
        Some(!revoked)
    } else {
        None
    };

    let verified = shape_ok && signature_ok && issuer_trusted
        && temporal_ok.unwrap_or(true) && revocation_ok.unwrap_or(true);
    Verdict {
        verified,
        shape_ok,
        signature_ok,
        issuer_trusted,
        temporal_ok,
        revocation_ok,
        issuer: (!issuer.is_empty()).then_some(issuer),
        details,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::SigningIdentity;

    fn identity() -> SigningIdentity {
        SigningIdentity::from_seed("did:web:test".into(), [9u8; 32])
    }

    fn ctx<'a>(id: &'a SigningIdentity, registry: &'a registry::TrustedIssuers, now: i64) -> TrustContext<'a> {
        TrustContext {
            registry,
            self_identity: Some(id),
            now,
        }
    }

    #[test]
    fn canonical_json_sorts_keys_recursively() {
        let doc = json!({"b": 1, "a": {"z": [3, 1], "y": "x"}, "c": null});
        assert_eq!(canonical_json(&doc), r#"{"a":{"y":"x","z":[3,1]},"b":1,"c":null}"#);
    }

    #[test]
    fn rfc3339_roundtrip_known_values() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(
            parse_rfc3339("2023-11-14T22:13:20Z"),
            Some(1_700_000_000)
        );
        // offset +02:00 shifts the instant back 7200s
        assert_eq!(
            parse_rfc3339("2023-11-15T00:13:20+02:00"),
            Some(1_700_000_000)
        );
        assert_eq!(parse_rfc3339("2023-11-14 22:13:20.5Z"), Some(1_700_000_000));
        assert_eq!(parse_rfc3339("not-a-date"), None);
    }

    #[test]
    fn multikey_roundtrip() {
        let id = identity();
        let mk = id.public_key_multibase.clone();
        assert!(mk.starts_with("z6Mk"));
        assert_eq!(multikey_decode(&mk), Some(id.key.verifying_key().to_bytes()));
    }

    #[test]
    fn sign_verify_roundtrip_and_tamper_rejection() {
        let id = identity();
        let mut doc = json!({"kind": "agent-manifest", "schemaFingerprint": "sha256:abc"});
        attach_proof(&mut doc, &id);

        let registry = registry::TrustedIssuers::default();
        let verdict = verify_document(&doc, &ctx(&id, &registry, 0));
        assert!(verdict.shape_ok);
        assert!(verdict.signature_ok, "signature gate: {:?}", verdict.details);
        assert!(verdict.verified, "{:?}", verdict.details);
        assert_eq!(verdict.temporal_ok, None);
        assert_eq!(verdict.revocation_ok, None);

        // tamper with the payload -> signature fails
        doc["schemaFingerprint"] = json!("sha256:evil");
        let verdict = verify_document(&doc, &ctx(&id, &registry, 0));
        assert!(!verdict.signature_ok);
        assert!(!verdict.verified);
    }

    #[test]
    fn unsigned_document_is_rejected() {
        let id = identity();
        let registry = registry::TrustedIssuers::default();
        let verdict = verify_document(&json!({"hello": "world"}), &ctx(&id, &registry, 0));
        assert!(!verdict.shape_ok);
        assert!(!verdict.verified);
    }

    #[test]
    fn vc_gates_trust_temporal_and_revocation() {
        let id = identity();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let mut registry = registry::TrustedIssuers::default();
        registry
            .insert(id.did.clone(), id.public_key_multibase.clone())
            .unwrap();

        // trusted, in-window
        let vc = issue_credential(
            &id,
            json!({"id": "urn:claim:1", "says": "hello"}),
            Some(rfc3339(now + 3600)),
            None,
            Some("urn:claim:1".into()),
        );
        let verdict = verify_document(&vc, &ctx(&id, &registry, now));
        assert_eq!(verdict.temporal_ok, Some(true));
        assert!(verdict.verified, "{:?}", verdict.details);

        // expired
        let vc = issue_credential(&id, json!({}), Some(rfc3339(now - 60)), None, None);
        let verdict = verify_document(&vc, &ctx(&id, &registry, now));
        assert_eq!(verdict.temporal_ok, Some(false));
        assert!(!verdict.verified);

        // revocation by id
        let vc = issue_credential(&id, json!({}), None, None, Some("urn:claim:2".into()));
        registry.revoke("urn:claim:2");
        let verdict = verify_document(&vc, &ctx(&id, &registry, now));
        assert_eq!(verdict.revocation_ok, Some(false));
        assert!(!verdict.verified);

        // supersession marks the credential stale
        let vc = issue_credential(&id, json!({"supersededBy": "urn:claim:3"}), None, None, None);
        let verdict = verify_document(&vc, &ctx(&id, &registry, now));
        assert_eq!(verdict.revocation_ok, Some(false));
        assert!(!verdict.verified);
    }

    #[test]
    fn untrusted_issuer_is_rejected_even_with_valid_signature() {
        let id = identity();
        let registry = registry::TrustedIssuers::default(); // empty: nobody trusted
        let mut spoof = json!({"says": "spoofed"});
        attach_proof(&mut spoof, &id);
        let ctx = TrustContext {
            registry: &registry,
            self_identity: None,
            now: 0,
        };
        let verdict = verify_document(&spoof, &ctx);
        assert!(!verdict.issuer_trusted);
        assert!(!verdict.verified);
    }
}