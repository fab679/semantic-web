//! GET /health (liveness + store readiness) and GET /metrics (Prometheus
//! text exposition of hub + service counters).

use std::sync::atomic::Ordering;

use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use serde_json::json;

use crate::api::{counters, SharedState};

pub async fn health(State(state): SharedState) -> impl IntoResponse {
    let reachable = state.store.ping().await.is_ok();
    axum::Json(json!({
        "status": if reachable { "ok" } else { "degraded" },
        "store": if reachable { "reachable" } else { "unreachable" },
    }))
}

pub async fn metrics(State(state): SharedState) -> impl IntoResponse {
    let m = &state.hub.metrics;
    let subs = state.hub.subscription_counts().await;
    let active_total: usize = subs.values().sum();
    let body = format!(
        "# TYPE semweb_verifications_total counter\n\
         semweb_verifications_total{{result=\"ok\"}} {}\n\
         semweb_verifications_total{{result=\"failed\"}} {}\n\
         # TYPE semweb_deliveries_total counter\n\
         semweb_deliveries_total{{result=\"success\"}} {}\n\
         semweb_deliveries_total{{result=\"exhausted\"}} {}\n\
         # TYPE semweb_delivery_retries_total counter\n\
         semweb_delivery_retries_total {}\n\
         # TYPE semweb_terminations_410_total counter\n\
         semweb_terminations_410_total {}\n\
         # TYPE semweb_denied_total counter\n\
         semweb_denied_total {}\n\
         # TYPE semweb_queue_dropped_total counter\n\
         semweb_queue_dropped_total {}\n\
         # TYPE semweb_rate_limited_total counter\n\
         semweb_rate_limited_total {}\n\
         # TYPE semweb_redelivered_on_restart_total counter\n\
         semweb_redelivered_on_restart_total {}\n\
         # TYPE semweb_routed_to_other_replica_total counter\n\
         semweb_routed_to_other_replica_total {}\n\
         # TYPE semweb_subscriptions_active gauge\n\
         semweb_subscriptions_active {active_total}\n\
         # TYPE semweb_fragments_requests_total counter\n\
         semweb_fragments_requests_total {}\n\
         # TYPE semweb_inserts_total counter\n\
         semweb_inserts_total {}\n",
        m.verifications_ok.load(Ordering::Relaxed),
        m.verifications_failed.load(Ordering::Relaxed),
        m.deliveries_ok.load(Ordering::Relaxed),
        m.deliveries_exhausted.load(Ordering::Relaxed),
        m.delivery_retries.load(Ordering::Relaxed),
        m.terminated_410.load(Ordering::Relaxed),
        m.denied_sent.load(Ordering::Relaxed),
        m.queue_dropped.load(Ordering::Relaxed),
        m.rate_limited.load(Ordering::Relaxed),
        m.redelivered_on_restart.load(Ordering::Relaxed),
        m.routed_to_other_replica.load(Ordering::Relaxed),
        counters::FRAGMENTS_REQUESTS.load(Ordering::Relaxed),
        counters::INSERTS_TOTAL.load(Ordering::Relaxed),
    );
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], body)
}