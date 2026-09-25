//! Streaming Semantic Fragments -- Rust service.
//!
//! Binary wiring only: configuration, router assembly, background tasks
//! (seed + shapes load, cardinality warm-up + periodic refresh,
//! subscription/delivery-log persistence load, lease sweeper). The WebSub
//! normative logic lives in hub/, the store contract in store/, the HTTP
//! surface in api/.

mod api;
mod context;
mod hub;
mod jsonld;
mod state;
mod store;
mod util;

use std::sync::Arc;
use std::time::Duration;

use crate::state::{build_state, AppState};
use crate::store::persistence::SHAPES_GRAPH;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    // Dedicated project port: 8484 (docker maps host 8484 -> container
    // 8000; local `cargo run` defaults to 8484 directly so no mapping is
    // needed and the project never conflicts with anything else).
    let port: u16 = std::env::var("SEMWEB_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8484);
    let state = Arc::new(build_state());

    tracing::info!(
        "semantic-web (rust) starting: store query={}, public_url={}, replicas={}/{}, port={}",
        state.store.query_url,
        state.public_url,
        state.hub.config().replica_index,
        state.hub.config().replica_count,
        port
    );
    tracing::info!(
        "hub config: queue_capacity={}, workers={}, open_hub={}, external_hubs={}",
        state.hub.config().queue_capacity,
        state.hub.config().workers,
        state.hub.config().open_hub,
        state.hub.config().external_hubs.len()
    );

    // Background task 1: seed + optional SHACL shapes, retried while the
    // store boots (compose starts containers concurrently and the
    // Oxigraph image is distroless, so compose cannot healthcheck-gate
    // it — a fixed small retry budget is not enough). Then warm the
    // cardinality counters and load persisted subscriptions/deliveries.
    {
        let state = state.clone();
        tokio::spawn(async move {
            load_startup_files(&state).await;
            warm_state(&state).await;
        });
    }

    // Background task 2: WebSub §5.3 -- hubs MUST enforce lease
    // expirations. The sweeper removes expired subscriptions (this
    // replica's shard) from memory and persistence.
    {
        let hub = state.hub.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(30));
            ticker.tick().await; // first tick fires immediately; skip it
            loop {
                ticker.tick().await;
                hub.expire_sweep().await;
            }
        });
    }

    // Background task 2b: multi-replica subscription refresh. Subscriptions
    // persist in the shared store; replicas poll to learn about new
    // subscriptions and lease renewals verified on other replicas.
    {
        let hub = state.hub.clone();
        tokio::spawn(async move {
            let secs: u64 = std::env::var("SEMWEB_SUBS_REFRESH_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10);
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(secs.max(2)));
            ticker.tick().await; // skip the immediate tick (load_persisted covers it)
            loop {
                ticker.tick().await;
                hub.refresh_from_persistence().await;
            }
        });
    }

    // Background task 3: periodic counter refresh. Counters are exact for
    // writes through this service; this bounds drift from out-of-band
    // writers (direct SPARQL UPDATE against the store).
    {
        let state = state.clone();
        tokio::spawn(async move {
            let secs: u64 = std::env::var("SEMWEB_COUNTER_REFRESH_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(300);
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(secs.max(30)));
            ticker.tick().await; // skip the immediate tick
            loop {
                ticker.tick().await;
                if let Err(e) = state.cardinality.load_initial(&state.store).await {
                    tracing::warn!("cardinality refresh failed: {e}");
                } else {
                    tracing::debug!("cardinality counters refreshed");
                }
            }
        });
    }

    let app = router(state);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .expect("bind port");
    tracing::info!("listening on 0.0.0.0:{port}");
    axum::serve(listener, app).await.expect("server error");
}

/// Warm in-memory state once the store is reachable: cardinality counters
/// first, then persisted subscriptions and the durable delivery log.
async fn warm_state(state: &AppState) {
    match state.cardinality.load_initial(&state.store).await {
        Ok(()) => tracing::info!("cardinality counters initialized"),
        Err(e) => tracing::error!("cardinality init failed: {e}"),
    }
    state.hub.load_persisted().await;
}

/// Startup file-load retry policy. Compose starts containers
/// concurrently and the store may take a while to boot; load_seed's
/// internal budget (10 x 2s) covers a normal boot, this outer loop adds
/// ~5 more minutes before giving up. If the store never appears, seed
/// and shapes are skipped (counters and subscriptions recover on their
/// periodic refresh schedules; the store stays empty until data is
/// written).
const STARTUP_LOAD_TRIES: usize = 60;
const STARTUP_LOAD_BACKOFF: Duration = Duration::from_secs(5);

async fn load_with_retry(state: &AppState, what: &str, path: &std::path::Path, graph: Option<&str>) {
    for attempt in 0..STARTUP_LOAD_TRIES {
        if let Some(g) = graph {
            // Clear-then-load: shape files contain blank nodes, which
            // get fresh labels on every load (unlike URI triples, RDF
            // set semantics does not deduplicate those).
            let _ = state.store.clear_graph(g).await;
        }
        match state.store.load_seed(path, graph).await {
            Ok(()) => {
                tracing::info!("{what} loaded from {}", path.display());
                return;
            }
            Err(e) if attempt + 1 < STARTUP_LOAD_TRIES => {
                tracing::warn!(
                    "{what} load attempt {} failed ({e}); retrying in {STARTUP_LOAD_BACKOFF:?}",
                    attempt + 1
                );
                tokio::time::sleep(STARTUP_LOAD_BACKOFF).await;
            }
            Err(e) => {
                tracing::error!("{what} load failed after {STARTUP_LOAD_TRIES} attempts: {e}");
                return;
            }
        }
    }
}

async fn load_startup_files(state: &AppState) {
    if let Ok(seed) = std::env::var("SEMWEB_SEED_PATH") {
        load_with_retry(state, "seed", std::path::Path::new(&seed), None).await;
    }
    if let Ok(shapes) = std::env::var("SEMWEB_SHACL_PATH") {
        load_with_retry(
            state,
            "SHACL shapes",
            std::path::Path::new(&shapes),
            Some(SHAPES_GRAPH),
        )
        .await;
    }
}

fn router(state: Arc<AppState>) -> axum::Router {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/", get(api::root))
        .route("/context.jsonld", get(api::context_jsonld))
        .route("/fragments", get(api::fragments))
        .route("/sparql", get(api::sparql).post(api::sparql_post))
        .route("/manifest", get(api::manifest))
        .route("/ui", get(api::explorer))
        .route("/events", get(api::events))
        .route("/mcp", post(api::mcp))
        .route("/health", get(api::health))
        .route("/metrics", get(api::metrics))
        .route("/hub", post(api::hub_post).get(api::hub_get))
        .route("/topics/{name}", get(api::topic))
        .route("/admin/insert", post(api::admin_insert))
        .layer(cors_layer())
        .with_state(state)
}

/// CORS for apps that live on other origins (the demo microblog,
/// dashboards, third-party frontends). `SEMWEB_CORS_ORIGINS` is a
/// comma-separated allowlist; unset/empty = allow any origin (demo/dev
/// posture — tighten it for a public deployment).
fn cors_layer() -> tower_http::cors::CorsLayer {
    use axum::http::HeaderValue;
    use tower_http::cors::{Any, CorsLayer};

    let origins = std::env::var("SEMWEB_CORS_ORIGINS").unwrap_or_default();
    let list: Vec<String> = origins
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    let layer = CorsLayer::new().allow_methods(Any).allow_headers(Any);
    if list.is_empty() {
        layer.allow_origin(Any)
    } else {
        let origins = list
            .iter()
            .map(|s| HeaderValue::from_str(s).expect("valid CORS origin"))
            .collect::<Vec<_>>();
        layer.allow_origin(origins)
    }
}
