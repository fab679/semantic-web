//! Application state and configuration. Everything configurable arrives
//! via environment variables so deployment topology changes never require
//! recompilation (see docker-compose.yml).

use std::sync::Arc;

use crate::context::PrefixMap;
use crate::hub::{Hub, HubConfig};
use crate::store::{Cardinality, SparqlStore};
use crate::trust::registry::TrustedIssuers;

/// This deployment's own cryptographic identity (opt-in via
/// SEMWEB_SIGNING_KEY). Signs the manifest and any issued
/// attestations; the public key is served at /.well-known/did.json.
pub struct SigningIdentity {
    pub did: String,
    pub key: ed25519_dalek::SigningKey,
    pub public_key_multibase: String,
}

impl SigningIdentity {
    pub fn from_seed(did: String, seed: [u8; 32]) -> Self {
        let key = ed25519_dalek::SigningKey::from_bytes(&seed);
        let public_key_multibase = crate::trust::multikey_encode(&key.verifying_key().to_bytes());
        SigningIdentity {
            did,
            key,
            public_key_multibase,
        }
    }

    /// SEMWEB_SIGNING_KEY (64 hex chars = 32-byte seed) enables signing;
    /// SEMWEB_DID overrides the issuer identity, which otherwise derives
    /// did:web style from the public base URL (host, percent-encoded port
    /// -- did:web's non-default-port rule).
    pub fn from_env(public_url: &str) -> Option<Self> {
        let hex = std::env::var("SEMWEB_SIGNING_KEY").ok()?;
        if hex.len() != 64 {
            tracing::warn!("SEMWEB_SIGNING_KEY must be 64 hex chars; signing disabled");
            return None;
        }
        let mut seed = [0u8; 32];
        for (i, chunk) in hex.as_bytes().chunks(2).enumerate().take(32) {
            seed[i] = u8::from_str_radix(std::str::from_utf8(chunk).ok()?, 16).ok()?;
        }
        let did = std::env::var("SEMWEB_DID")
            .ok()
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| {
            let host = reqwest::Url::parse(public_url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_string))
                .unwrap_or_else(|| "localhost".into());
            let port_part = reqwest::Url::parse(public_url)
                .ok()
                .and_then(|u| u.port())
                .map(|p| format!("%3A{p}"))
                .unwrap_or_default();
            format!("did:web:{host}{port_part}")
        });
        Some(SigningIdentity::from_seed(did, seed))
    }
}

pub struct AppState {
    pub store: Arc<SparqlStore>,
    pub hub: Arc<Hub>,
    /// Maintained per-class/per-predicate/subject cardinalities (exact for
    /// writes through this service; the manifest recomputes live).
    pub cardinality: Arc<Cardinality>,
    /// Public base URL used to mint absolute rel=self / rel=hub URLs for
    /// WebSub discovery and content distribution Link headers.
    pub public_url: String,
    pub prefixes: Arc<PrefixMap>,
    /// When set, mutating endpoints (POST /admin/insert) require
    /// `Authorization: Bearer <token>`. Read endpoints stay open.
    pub write_token: Option<String>,
    /// When set, POST /hub (subscribe/unsubscribe/publish) requires
    /// `Authorization: Bearer <token>` -- who may subscribe is policy.
    pub hub_token: Option<String>,
    /// Trust layer (opt-in): identity used to sign the manifest and issue
    /// attestations; None means the deployment serves unsigned data only
    /// and every behavior is byte-identical to the pre-trust surface.
    pub signing: Option<SigningIdentity>,
    /// Local trusted-issuer registry + revocation list for verify gates.
    pub trust: TrustedIssuers,
}

/// Read configuration from the environment and assemble state. Called
/// once at startup.
pub fn build_state() -> AppState {
    let port = std::env::var("SEMWEB_PORT").unwrap_or_else(|_| "8000".into());
    let query_endpoint = std::env::var("SEMWEB_SPARQL_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:7878/query".into());
    let update_endpoint = std::env::var("SEMWEB_SPARQL_UPDATE")
        .unwrap_or_else(|_| "http://localhost:7878/update".into());
    let public_url = std::env::var("SEMWEB_PUBLIC_URL")
        .unwrap_or_else(|_| format!("http://localhost:{port}"))
        .trim_end_matches('/')
        .to_string();
    let queue_capacity = std::env::var("SEMWEB_QUEUE_CAPACITY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4096);
    let workers = std::env::var("SEMWEB_DELIVERY_WORKERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    let replica_count = std::env::var("SEMWEB_REPLICA_COUNT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let replica_index = std::env::var("SEMWEB_REPLICA_INDEX")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let require_https_callbacks = std::env::var("SEMWEB_REQUIRE_HTTPS_CALLBACKS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let callback_allowlist = std::env::var("SEMWEB_CALLBACK_ALLOWLIST")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();

    let store = Arc::new(SparqlStore::new(query_endpoint, update_endpoint));
    let cardinality = Arc::new(Cardinality::default());
    let prefixes = Arc::new(PrefixMap::from_env());
    let hub = Hub::new(
        store.clone(),
        prefixes.clone(),
        HubConfig {
            queue_capacity,
            workers,
            external_hubs: std::env::var("SEMWEB_HUB_URLS")
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
            open_hub: std::env::var("SEMWEB_OPEN_HUB")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            replica_count,
            replica_index,
            public_url: public_url.clone(),
            require_https_callbacks,
            callback_allowlist,
        },
    );

    let signing = SigningIdentity::from_env(&public_url);

    AppState {
        store,
        hub,
        cardinality,
        public_url,
        prefixes,
        write_token: std::env::var("SEMWEB_WRITE_TOKEN").ok().filter(|t| !t.is_empty()),
        hub_token: std::env::var("SEMWEB_HUB_TOKEN").ok().filter(|t| !t.is_empty()),
        signing,
        trust: TrustedIssuers::from_env(),
    }
}
