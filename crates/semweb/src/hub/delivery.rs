//! Content distribution execution (spec §7) — the worker pool's core.
//!
//! The distribution request POSTs the full topic contents to the callback
//! URL exactly as submitted (query string preserved, parameters in the
//! URL). It MUST carry a Content-Type matching the topic and Link headers
//! rel=self (canonical topic URL) + rel=hub, combined into one header.
//! Subscriber semantics: 2xx = acknowledged (response body ignored);
//! 410 = subscription deleted, hub MAY terminate; everything else is a
//! failure and is retried per the self-imposed schedule. When retries are
//! exhausted the hub gives up on this notification but keeps the
//! subscription active until lease end (§7).
//!
//! Durability: every delivery is logged (persist_delivery) before being
//! queued and acked (delete_delivery) when finished — success, 410
//! termination, or retry exhaustion — so a crash mid-flight redelivers
//! from the log on startup instead of silently dropping the notification.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use hmac::{Hmac, Mac};
use sha2::Sha256;

use super::{Hub, Delivery};

/// Delivery retry schedule (spec §7: hubs SHOULD retry failed content
/// distribution up to self-imposed limits on count and time). After the
/// schedule is exhausted the hub stops attempting THIS notification but
/// MUST keep the subscription active until lease end; the next published
/// update triggers fresh delivery attempts.
pub const RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(5),
    Duration::from_secs(15),
];

/// The worker loop: exactly one worker takes each queued delivery
/// (workers share the receiver under a mutex, see Hub::new).
pub(crate) async fn worker_loop(hub: Arc<Hub>, rx: Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<Delivery>>>, worker: usize) {
    loop {
        let delivery = { rx.lock().await.recv().await };
        match delivery {
            Some(d) => deliver_one(hub.clone(), d, worker).await,
            None => return, // channel closed
        }
    }
}

pub(crate) async fn deliver_one(hub: Arc<Hub>, delivery: Delivery, worker: usize) {
    let Delivery {
        id,
        topic,
        callback,
        secret,
        content,
        self_url,
        hub_url,
        event_id,
        from_log,
    } = delivery;

    // §7: at least one Link header rel=hub and one rel=self, combined into
    // a single header. These are topic metadata; subscribers MUST NOT use
    // them to identify the subscription.
    let link = format!("<{self_url}>; rel=\"self\", <{hub_url}>; rel=\"hub\"");
    if from_log {
        tracing::info!(event = event_id, callback, "redelivering from durable log after restart");
    }

    for attempt in 0..=RETRY_DELAYS.len() {
        let mut req = hub
            .http
            .post(&callback)
            .header("Link", &link)
            .header("Content-Type", content.content_type);
        if let Some(secret) = &secret {
            if let Some(header) = signature_header(secret, &content.body) {
                req = req.header("X-Hub-Signature", header);
            }
        }
        match req.body(content.body.clone()).send().await {
            Ok(resp) if resp.status().is_success() => {
                hub.metrics.deliveries_ok.fetch_add(1, Ordering::Relaxed);
                ack(&hub, &id, &event_id, &callback, worker).await;
                if attempt > 0 {
                    tracing::info!(
                        event = event_id, worker, callback,
                        "delivery succeeded on retry {attempt}"
                    );
                } else {
                    tracing::debug!(event = event_id, worker, callback, "delivery ok");
                }
                return;
            }
            Ok(resp) if resp.status().as_u16() == 410 => {
                hub.metrics.terminated_410.fetch_add(1, Ordering::Relaxed);
                tracing::info!(event = event_id, callback, "410 Gone; terminating subscription");
                hub.remove_sub(topic, &callback).await;
                ack(&hub, &id, &event_id, &callback, worker).await;
                return;
            }
            other => {
                let status = match &other {
                    Ok(r) => format!("{}", r.status()),
                    Err(e) => format!("error: {e}"),
                };
                if attempt < RETRY_DELAYS.len() {
                    hub.metrics.delivery_retries.fetch_add(1, Ordering::Relaxed);
                    tracing::warn!(
                        event = event_id, worker, callback,
                        "delivery failed ({status}); retrying in {:?}",
                        RETRY_DELAYS[attempt]
                    );
                    tokio::time::sleep(RETRY_DELAYS[attempt]).await;
                } else {
                    // §7: keep the subscription active until lease end even
                    // after exhausting retries for this notification; only
                    // the notification itself is given up on (and acked in
                    // the durable log so a restart does not resurrect it).
                    hub.metrics
                        .deliveries_exhausted
                        .fetch_add(1, Ordering::Relaxed);
                    ack(&hub, &id, &event_id, &callback, worker).await;
                    tracing::error!(
                        event = event_id, callback,
                        "delivery failed after {} attempts ({status}); keeping subscription \
                         until lease expiry (spec §7)",
                        attempt + 1
                    );
                }
            }
        }
    }
}

async fn ack(hub: &Arc<Hub>, id: &str, event_id: &str, callback: &str, worker: usize) {
    if let Err(e) = hub.store.delete_delivery(id).await {
        tracing::warn!(event = event_id, worker, callback, "delivery log ack failed: {e}");
    }
}

/// Compute the §7.1 authenticated distribution header.
///
/// `X-Hub-Signature: method=signature`, HMAC over the request body keyed
/// by the subscriber's hub.secret, signature in lowercase hex. Recognized
/// algorithm names (§7.1.1) are sha1, sha256, sha384, sha512; we emit
/// sha256 because §8.3 rules out SHA-1 and calls sha256 the minimum.
fn signature_header(secret: &str, body: &[u8]) -> Option<String> {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes()).ok()?;
    mac.update(body);
    let bytes = mac.finalize().into_bytes();
    Some(format!(
        "sha256={}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ))
}

#[allow(dead_code)] // used by unit tests
/// exposed for tests
pub fn retry_schedule() -> Vec<Duration> {
    RETRY_DELAYS.to_vec()
}