//! GET /.well-known/did.json — the deployment's DID document (did:web).
//!
//! This is how the trust layer stays a pure web-standard layer: the
//! public key that verifies the signed manifest is fetched over plain
//! HTTP from the conventional did:web location, exactly like any other
//! did:web identity. 404 with an explanatory body when signing is not
//! configured (the trust layer is opt-in).

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::api::SharedState;

pub async fn did_document(State(state): SharedState) -> Response {
    let Some(identity) = &state.signing else {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(json!({
                "error": "signing not configured",
                "hint": "set SEMWEB_SIGNING_KEY (64 hex chars) to enable the trust layer; optionally SEMWEB_DID to override the did:web identity"
            })),
        )
            .into_response();
    };
    let did = &identity.did;
    axum::Json(json!({
        "@context": ["https://www.w3.org/ns/did/v1"],
        "id": did,
        "verificationMethod": [{
            "id": format!("{did}#key-1"),
            "type": "Multikey",
            "controller": did,
            "publicKeyMultibase": identity.public_key_multibase,
        }],
        "assertionMethod": [format!("{did}#key-1")],
        "authentication": [format!("{did}#key-1")],
        "trust": {
            "trustedIssuers": state.trust.issuer_count(),
            "revokedCredentials": state.trust.revoked_count(),
        },
    }))
    .into_response()
}