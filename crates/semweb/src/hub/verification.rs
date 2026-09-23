//! WebSub §5.3 intent verification and state mutation (§5.1).
//!
//! The hub GETs the callback with hub.mode, hub.topic, hub.challenge and
//! (for subscribe) hub.lease_seconds appended to the callback's existing
//! query string. Commit rules (§5.3.1):
//!   - 2xx + body == challenge  -> action is verified: commit it
//!   - 404                      -> subscriber rejects the action: no change
//!   - any other 3xx/4xx/5xx    -> verification failed: no change
//!   - 2xx + wrong body         -> verification failed: no change
//! Committing a subscribe (re)creates/extends the (topic, callback)
//! subscription with the hub-determined lease; committing an unsubscribe
//! removes it. A previously active subscription is only touched once the
//! NEW action is verified, per §5.1. Committed state is persisted to
//! HUB_GRAPH (encrypted secrets when a key is configured) so it survives
//! restarts.

use std::sync::Arc;
use std::time::Duration;

use rand::Rng;

use super::{Hub, Intent, Subscription};
use crate::store::persistence as persist;

/// Lease policy (spec §5.3: hubs MUST enforce lease expirations and MUST
/// NOT issue perpetual leases; §8.2 recommends short-lived leases, 10 days
/// as a good default).
pub const DEFAULT_LEASE_SECS: u64 = 86_400; // 1 day when hub.lease_seconds omitted
pub const MAX_LEASE_SECS: u64 = 864_000; // 10 days cap
pub const MIN_LEASE_SECS: u64 = 60;

pub(crate) fn clamp_lease(requested: Option<u64>) -> u64 {
    requested
        .unwrap_or(DEFAULT_LEASE_SECS)
        .clamp(MIN_LEASE_SECS, MAX_LEASE_SECS)
}

/// Generate a hub.challenge.
///
/// Spec §5.3: the challenge MUST only consist of characters in the set
/// [#x2B] | [#x2D-#x39] | [#x3D] | [#x41-#x5A] | [#x5F] | [#x61-#x7A]
/// (i.e. + - . / 0-9 = A-Z _ a-z). Security §8.2 adds: limit its length
/// and reject binary data. We generate from the URL-safe subset
/// (alnum, '-', '_') of exactly that set, satisfying both the set
/// constraint and the no-binary recommendation.
pub fn generate_challenge(len: usize) -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut rng = rand::rng();
    (0..len)
        .map(|_| CHARSET[rng.random_range(0..CHARSET.len())] as char)
        .collect()
}

/// The allowed hub.challenge charset from spec §5.3, exposed for tests:
/// + - . / 0-9 = A-Z _ a-z.
#[allow(dead_code)] // used by unit tests below
pub const CHALLENGE_CHARSET_ALLOWED: &[char] = &[
    '+', '-', '.', '/', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '=', 'A', 'B', 'C', 'D',
    'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W',
    'X', 'Y', 'Z', '_', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o',
    'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z',
];

/// Append a query parameter to a callback URL while preserving the
/// existing query string.
///
/// Spec §5.1.1: callbacks MAY carry arbitrary query parameters; hubs MUST
/// preserve them during verification "by appending new parameters to the
/// end of the list using the & character"; existing parameters with
/// overlapping names are not overwritten (appending never overwrites).
/// During content distribution the callback URL -- including its query
/// string -- is POSTed to, with parameters in the URL, not the body.
pub(crate) fn append_query(callback: &str, key: &str, value: &str) -> String {
    let pair = format!("{key}={}", urlencoding::encode(value));
    let sep = if callback.contains('?') { "&" } else { "?" };
    format!("{callback}{sep}{pair}")
}

pub(crate) async fn verify_and_commit(
    hub: Arc<Hub>,
    canonical_topic: &str,
    submitted_topic: &str,
    callback: &str,
    intent: Intent,
) {
    // §5.2: subscriptions MAY be denied by the hub at any point. If the
    // topic is no longer available at verification time, deny explicitly
    // instead of leaving the subscriber waiting.
    if super::resolve_topic(submitted_topic).is_none() {
        hub.send_denied(callback, submitted_topic, "topic is no longer available")
            .await;
        return;
    }

    let challenge = generate_challenge(43);
    let (mode, url) = match &intent {
        Intent::Subscribe { lease, .. } => {
            let mut url = append_query(callback, "hub.mode", "subscribe");
            url = append_query(&url, "hub.topic", submitted_topic);
            url = append_query(&url, "hub.challenge", &challenge);
            // §5.3: lease_seconds is REQUIRED when hub.mode is "subscribe".
            url = append_query(&url, "hub.lease_seconds", &lease.to_string());
            ("subscribe", url)
        }
        Intent::Unsubscribe => {
            let mut url = append_query(callback, "hub.mode", "unsubscribe");
            url = append_query(&url, "hub.topic", submitted_topic);
            url = append_query(&url, "hub.challenge", &challenge);
            // §5.3: lease_seconds MAY be present for unsubscribe and MUST
            // be ignored by subscribers; we omit it.
            ("unsubscribe", url)
        }
    };

    match hub.http.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            let body = resp.text().await.unwrap_or_default();
            if body.trim() == challenge {
                hub.metrics.verifications_ok.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                match intent {
                    Intent::Subscribe { secret, lease } => {
                        let expires = now_epoch() + lease;
                        tracing::info!(topic = canonical_topic, callback, "subscription verified");
                        hub.subs.write().await.insert(
                            (submitted_topic.to_string(), callback.to_string()),
                            Subscription {
                                secret: secret.clone(),
                                lease_expires: std::time::Instant::now()
                                    + Duration::from_secs(lease),
                                lease_expires_epoch: expires,
                            },
                        );
                        let stored_secret = secret.as_deref().map(|s| hub.crypto.encrypt(s));
                        if let Err(e) = hub
                            .store
                            .persist_hub_subscription(
                                &persist::subscription_persist_id(submitted_topic, callback),
                                submitted_topic,
                                callback,
                                stored_secret.as_deref(),
                                expires,
                            )
                            .await
                        {
                            tracing::warn!("subscription persistence failed: {e}");
                        }
                    }
                    Intent::Unsubscribe => {
                        tracing::info!(topic = canonical_topic, callback, "unsubscribed");
                        let id = persist::subscription_persist_id(submitted_topic, callback);
                        hub.subs
                            .write()
                            .await
                            .remove(&(submitted_topic.to_string(), callback.to_string()));
                        if let Err(e) = hub.store.delete_hub_subscription(&id).await {
                            tracing::warn!("persisted subscription {id} removal failed: {e}");
                        }
                    }
                }
            } else {
                hub.metrics
                    .verifications_failed
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                tracing::warn!(
                    "verification body mismatch for {callback} ({mode}); state unchanged"
                );
            }
        }
        Ok(resp) => {
            // §5.3.1: 3xx/4xx/5xx all mean verification failed. A 404 is
            // the subscriber explicitly rejecting the action.
            hub.metrics
                .verifications_failed
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            tracing::warn!(
                "verification of {callback} ({mode}) -> {}; state unchanged",
                resp.status()
            );
        }
        Err(e) => {
            hub.metrics
                .verifications_failed
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            tracing::warn!("verification of {callback} ({mode}) unreachable: {e}");
        }
    }
}

pub fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_uses_only_the_spec_charset() {
        // spec §5.3: only + - . / 0-9 = A-Z _ a-z; we generate from the
        // URL-safe subset (alnum, -, _)
        for len in [1, 8, 43, 100] {
            for ch in generate_challenge(len).chars() {
                assert!(
                    ch.is_ascii_alphanumeric() || ch == '-' || ch == '_',
                    "challenge char {ch:?} outside allowed set"
                );
            }
        }
    }

    #[test]
    fn challenge_is_long_enough_to_be_unguessable() {
        // §8.2: limit length + no binary; we generate 43 chars per request
        let c = generate_challenge(43);
        assert_eq!(c.len(), 43);
    }

    #[test]
    fn clamp_lease_enforces_explication_and_bounds() {
        // §5.3: MUST enforce expirations, MUST NOT issue perpetual leases
        assert_eq!(clamp_lease(None), 86_400);
        assert_eq!(clamp_lease(Some(5)), 60); // below minimum -> clamped
        assert_eq!(clamp_lease(Some(u64::MAX)), 864_000); // never perpetual
        assert_eq!(clamp_lease(Some(7200)), 7200); // respected in range
    }

    #[test]
    fn append_query_preserves_existing_params_without_overwrite() {
        // §5.1.1: preserve query string, append, never overwrite
        let cb = "http://x/cb?tag=demo&hub.mode=unsubscribe";
        let out = append_query(cb, "hub.mode", "subscribe");
        assert!(out.contains("?tag=demo"));
        assert!(out.contains("&hub.mode=subscribe"));
        assert!(out.contains("hub.mode=unsubscribe"));
        let out2 = append_query("http://x/cb", "hub.challenge", "a=b&c");
        assert_eq!(out2, "http://x/cb?hub.challenge=a%3Db%26c");
    }
}
