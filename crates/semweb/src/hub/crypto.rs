//! Secret protection at rest.
//!
//! When SEMWEB_SECRET_KEY is configured (32-byte hex), subscriber
//! `hub.secret` values are encrypted with AES-256-GCM before they are
//! persisted into the hub's own store, and decrypted on load/delivery.
//! The serialized form is self-describing:
//!
//!   plaintext (no key configured)      -> the raw secret
//!   encrypted                          -> "enc:v1:<b64 nonce>:<b64 ciphertext>"
//!
//! The "enc:v1:" prefix keeps decryption explicit and lets plaintext
//! values written before key adoption keep working (decrypt falls back
//! to the raw string).

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;

#[allow(dead_code)] // self-documenting wire constant
const PREFIX: &str = "enc:v1:";
const NONCE_LEN: usize = 12;

#[derive(Clone)]
pub struct SecretCrypto {
    key: Option<[u8; 32]>,
}

impl SecretCrypto {
    /// Build from an explicit key (tests).
    #[allow(dead_code)]
    pub fn new(key: Option<[u8; 32]>) -> Self {
        SecretCrypto { key }
    }

    /// Parse the env var: 64 hex chars = 32 bytes.
    pub fn from_env() -> Self {
        let key = std::env::var("SEMWEB_SECRET_KEY").ok().and_then(|hex| {
            if hex.len() != 64 {
                tracing::warn!("SEMWEB_SECRET_KEY must be 64 hex chars; ignoring");
                return None;
            }
            let mut key = [0u8; 32];
            for (i, chunk) in hex.as_bytes().chunks(2).enumerate().take(32) {
                key[i] = u8::from_str_radix(
                    std::str::from_utf8(chunk).ok()?,
                    16,
                )
                .ok()?;
            }
            Some(key)
        });
        SecretCrypto { key }
    }

    #[allow(dead_code)]
    pub fn enabled(&self) -> bool {
        self.key.is_some()
    }

    /// Serialize a secret for persistence. Without a key this is a no-op
    /// (documented trade-off: plaintext persistence inside the hub's own
    /// store; encryption is the recommended configuration).
    pub fn encrypt(&self, plaintext: &str) -> String {
        let key = match self.key {
            Some(k) => k,
            None => return plaintext.to_string(),
        };
        let cipher = Aes256Gcm::new_from_slice(&key).expect("valid 32-byte key");
        let nonce: [u8; NONCE_LEN] = rand::random();
        let ct = cipher
            .encrypt(Nonce::from_slice(&nonce), plaintext.as_bytes())
            .expect("aes-gcm encryption of in-memory data cannot fail");
        format!("enc:v1:{}:{}", b64(&nonce), b64(&ct))
    }

    /// Deserialize a persisted secret. Handles both encrypted ("enc:v1:")
    /// and plaintext forms (pre-encryption values keep working).
    pub fn decrypt(&self, stored: &str) -> Option<String> {
        let Some(stripped) = stored.strip_prefix("enc:v1:") else {
            return Some(stored.to_string()); // legacy plaintext
        };
        let key = self.key?;
        let (nonce_b64, ct_b64) = stripped.split_once(':')?;
        
        
        let nonce = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(nonce_b64).ok()?;
        if nonce.len() != NONCE_LEN {
            return None;
        }
        let ct = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(ct_b64).ok()?;
        let cipher = Aes256Gcm::new_from_slice(&key).ok()?;
        let plaintext = cipher.decrypt(Nonce::from_slice(&nonce), ct.as_ref()).ok()?;
        String::from_utf8(plaintext).ok()
    }
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[allow(dead_code)]
fn un_b64(s: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: Option<[u8; 32]> = Some([7u8; 32]);

    #[test]
    fn roundtrip_without_key_is_plaintext() {
        let c = SecretCrypto::new(None);
        assert!(!c.enabled());
        assert_eq!(c.encrypt("hunter2"), "hunter2");
        assert_eq!(c.decrypt("hunter2"), Some("hunter2".to_string()));
    }

    #[test]
    fn roundtrip_with_key_is_encrypted_and_reversible() {
        let c = SecretCrypto::new(KEY);
        assert!(c.enabled());
        let stored = c.encrypt("hunter2");
        assert!(stored.starts_with("enc:v1:"));
        assert!(!stored.contains("hunter"));
        assert_eq!(c.decrypt(&stored).as_deref(), Some("hunter2"));
    }

    #[test]
    fn decrypt_accepts_legacy_plaintext_and_rejects_tampering() {
        let c = SecretCrypto::new(KEY);
        assert_eq!(c.decrypt("legacy-plain").as_deref(), Some("legacy-plain"));
        let stored = c.encrypt("hunter");
        // flip the first character of the CIPHERTEXT segment (the last
        // "enc:v1:<nonce>:<ct>" part) -- carries real bits
        let ct_start = stored.rfind(':').unwrap() + 1;
        let mut tampered = stored.clone();
        let ch = tampered.as_bytes()[ct_start];
        let flipped = if ch == b'A' { b'B' } else { b'A' };
        tampered.replace_range(ct_start..ct_start + 1, &(flipped as char).to_string());
        assert_ne!(tampered, stored);
        assert_eq!(c.decrypt(&tampered), None);
    }
}
