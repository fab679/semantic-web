# Trust layer: claims that carry their own evidence

Most information on the web arrives naked: a number, a statement, an
export — with no verifiable attachment saying *who vouched for this,
when, and until when*. This page describes the trust layer: the part of
the infrastructure that lets a publisher sign what they serve and a
reader check it before believing it.

## What gets signed

When a deployment is given a signing key (one environment variable),
its self-description — the manifest — carries a **cryptographic
proof**: an Ed25519 signature over the document, made under the
publisher's web identity.

The identity is a `did:web` DID — derived from the web location the
same way an HTTPS certificate is. The public key is served at a
conventional URL, `/.well-known/did.json`, so a reader verifies with
nothing but HTTP and their own decision about whom to trust.

A deployment can also issue **attestations**: signed statements about
claims (this is true, valid until X, replaces Y). Attestations have ids
and can expire, be revoked, or be superseded — stale facts can retire
old evidence instead of silently contradicting it.

## What a reader checks — four questions

Verification is not a yes/no badge; it answers four separate questions,
each independently:

1. **Is there a proof at all?** An unsigned document is not an error —
   it is simply unverifiable, and a careful reader declines to treat it
   as more than a claim.
2. **Does the content match the signature?** Every tampered byte breaks
   this — the signature covers the whole document.
3. **Is the signer trusted?** The signer's identity is resolved against
   the reader's own trust registry (or is the publisher itself). Keys
   are resolved locally; the verifier never fetches keys from parties
   it hasn't chosen to trust.
4. **Is it fresh?** Was it issued inside its validity window? Is it on
   a revocation list? Has it been superseded by a newer attestation?

A reader that requires all four has moved verification *before use* —
instead of fact-checking copies after the fact.

## The economics, stated plainly

Signing costs the publisher one key and one config line. The benefit
is asymmetric: verifiable data is more *usable* data — more citable,
more integrable, more defensible — and a growing set of regulations
(transparency, provenance, reporting duties) increasingly demands
exactly this property. Like HTTPS, the expectation is that verifiable
starts as a differentiator and ends as an assumption.

## Configuration

| Variable | Meaning |
|---|---|
| `SEMWEB_SIGNING_KEY` | 32-byte Ed25519 seed, 64 hex chars (`openssl rand -hex 32`). Enables the layer. Unset = unsigned surface, everything unchanged. |
| `SEMWEB_DID` | Override the publisher identity. Default: `did:web:<host>` from the public URL. |
| `SEMWEB_TRUSTED_ISSUERS` | Inline registry of trusted issuers: `did=zMk...` pairs. |
| `SEMWEB_TRUSTED_ISSUERS_PATH` | Registry as a JSON file: `{"issuers": {"did": "z..."}, "revoked": ["urn:..."]}`. |
| `SEMWEB_REVOKED_CREDENTIAL_IDS` | Inline revocation list (comma-separated ids). |

## Documented simplifications

The trust layer follows the W3C Verifiable Credentials 2.0 and Data
Integrity shapes with three deliberate, audited simplifications — each
scoped to what this service actually signs:

- **Canonicalization**: sorted-key JSON instead of URDNA2015 (the
  signed documents contain no blank nodes, where the two differ).
- **Revocation**: a flat identifier list instead of the Bitstring
  Status List.
- **Registry**: operator-provisioned (env/file) instead of a
  network-fetched, self-signed registry — trust configuration is a
  decision, and here it is an explicit one.