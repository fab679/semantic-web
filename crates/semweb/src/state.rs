//! Application state and configuration. Everything configurable arrives
//! via environment variables so deployment topology changes never require
//! recompilation (see docker-compose.yml).

use std::sync::Arc;

use crate::context::PrefixMap;
use crate::hub::{Hub, HubConfig};
use crate::store::{Cardinality, SparqlStore};

pub struct AppState {
    pub store: Arc<SparqlStore>,
    pub hub: Arc<Hub>,
    /// Maintained per-class/per-predicate/subject cardinalities (exact for
    /// writes through this service; the agent manifest recomputes live).
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
            replica_count,
            replica_index,
            public_url: public_url.clone(),
            require_https_callbacks,
            callback_allowlist,
        },
    );

    AppState {
        store,
        hub,
        cardinality,
        public_url,
        prefixes,
        write_token: std::env::var("SEMWEB_WRITE_TOKEN").ok().filter(|t| !t.is_empty()),
        hub_token: std::env::var("SEMWEB_HUB_TOKEN").ok().filter(|t| !t.is_empty()),
    }
}
