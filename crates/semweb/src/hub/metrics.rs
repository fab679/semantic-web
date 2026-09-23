//! Hub observability counters, rendered as Prometheus text at /metrics
//! (see api::observability).

use std::sync::atomic::AtomicU64;

#[derive(Default)]
pub struct HubMetrics {
    pub verifications_ok: AtomicU64,
    pub verifications_failed: AtomicU64,
    pub deliveries_ok: AtomicU64,
    pub deliveries_exhausted: AtomicU64,
    pub delivery_retries: AtomicU64,
    pub terminated_410: AtomicU64,
    pub denied_sent: AtomicU64,
    pub queue_dropped: AtomicU64,
    pub rate_limited: AtomicU64,
    /// Deliveries re-enqueued from the durable log after a restart.
    pub redelivered_on_restart: AtomicU64,
    /// Deliveries skipped because another replica owns the subscription.
    pub routed_to_other_replica: AtomicU64,
}