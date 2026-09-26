# Reference

The complete HTTP surface and configuration. Everything below is
factual: one row per capability.

## HTTP surface

| Method | Path | Purpose | Auth |
|---|---|---|---|
| GET | `/` | Live self-description: classes, predicates, controls, discovery Link headers | none |
| GET | `/context.jsonld` | The JSON-LD `@context`, generated from namespaces in use | none |
| GET | `/fragments` | Triple Pattern Fragments: `?subject`, `?predicate`, `?object`, `?graph`, `?after` (cursor), `?limit`, `?offset` | none |
| GET | `/sparql` | Read-only SPARQL SELECT/ASK/DESCRIBE/CONSTRUCT (`?query`), transparent passthrough to the store | none |
| POST | `/sparql` | Same, with the query in the body | none |
| GET | `/manifest` | The self-description for programs: schema fingerprint, live cardinalities, descriptions, SHACL shapes, example queries; signed when the trust layer is enabled | none |
| GET | `/topics/{name}` | Topic content (data as NDJSON, schema as JSON) with WebSub discovery headers | none |
| POST | `/hub` | WebSub subscribe / unsubscribe / publish (form-encoded) | optional `SEMWEB_HUB_TOKEN` |
| GET | `/hub` | Hub info | none |
| GET | `/events` | Server-sent events feed (`?topic`) | none |
| POST | `/mcp` | Machine tool surface: graph search, SPARQL, manifest, topics, verify/issue, insert, subscribe | insert honours write token |
| POST | `/admin/insert` | Write one triple (duplicate-safe; fires schema-change events) | `SEMWEB_WRITE_TOKEN` |
| GET | `/.well-known/did.json` | The publisher's DID document (public key) when the trust layer is enabled; 404 otherwise | none |
| GET | `/ui` | The embedded Graph Explorer page | none |
| GET | `/health` | Liveness + store reachability | none |
| GET | `/metrics` | Prometheus counters | none |

Response shapes: reads are JSON, JSON-LD, NDJSON (streamed), or SSE.
Writes accept JSON. Hub operations accept form encoding (WebSub §5.1).

## Configuration (environment only)

### Where data lives

| Variable | Default | Meaning |
|---|---|---|
| `SEMWEB_SPARQL_ENDPOINT` | `http://localhost:7878/query` | SPARQL 1.1 query endpoint |
| `SEMWEB_SPARQL_UPDATE` | `http://localhost:7878/update` | SPARQL 1.1 update endpoint |
| `SEMWEB_PUBLIC_URL` | `http://localhost:<port>` | Base URL minted into discovery and subscription URLs |
| `SEMWEB_PORT` | `8000` (compose binds host 8484) | Listener port |
| `SEMWEB_SEED_PATH` | — | TTL file loaded at startup (idempotent seed) |
| `SEMWEB_SHACL_PATH` | — | TTL shapes file loaded into a reserved graph |

### Vocabulary surface

| Variable | Meaning |
|---|---|
| `SEMWEB_EXTRA_PREFIXES` | Extra prefix declarations, `p=namespace` comma-separated |
| `SEMWEB_TERM_ALIASES` | Bare friendly names mapped to URIs, `name=URI` comma-separated |

### Push (WebSub hub)

| Variable | Meaning |
|---|---|
| `SEMWEB_QUEUE_CAPACITY` | Bounded delivery queue size (overflow waits in the durable log) |
| `SEMWEB_DELIVERY_WORKERS` | Delivery worker pool size |
| `SEMWEB_SECRET_KEY` | 32-byte hex; AES-256-GCM encryption of subscriber secrets at rest |
| `SEMWEB_OPEN_HUB` | `1` allows third-party topic URLs on this hub |
| `SEMWEB_HUB_URLS` | External hubs notified on publish (fan-out federation) |
| `SEMWEB_HUB_TOKEN` | Bearer token gating who may subscribe |
| `SEMWEB_REQUIRE_HTTPS_CALLBACKS` | Require HTTPS subscriber callbacks |
| `SEMWEB_CALLBACK_ALLOWLIST` | Host allowlist for callbacks and open-hub fetches |
| `SEMWEB_REPLICA_COUNT` / `SEMWEB_REPLICA_INDEX` | Consistent-hash shard of the subscription space per replica |
| `SEMWEB_SUBS_REFRESH_SECS` | Multi-replica subscription refresh cadence |

### Access and trust

| Variable | Meaning |
|---|---|
| `SEMWEB_WRITE_TOKEN` | Bearer token for mutating endpoints; unset = open writes (not recommended) |
| `SEMWEB_SIGNING_KEY` | 32-byte Ed25519 seed (64 hex) — enables signing; unset = unsigned surface |
| `SEMWEB_DID` | Override the publisher identity; default derives `did:web:<host>` from `SEMWEB_PUBLIC_URL` |
| `SEMWEB_TRUSTED_ISSUERS` | Inline trusted issuers: `did=zMk...` comma-separated |
| `SEMWEB_TRUSTED_ISSUERS_PATH` | JSON registry file: `{"issuers": {...}, "revoked": [...]}` |
| `SEMWEB_REVOKED_CREDENTIAL_IDS` | Inline revocation list (comma-separated credential ids) |

### Background maintenance

| Variable | Meaning |
|---|---|
| `SEMWEB_COUNTER_REFRESH_SECS` | Periodic cardinality recount (default 300) |
| `SEMWEB_CORS_ORIGINS` | Comma-separated CORS allowlist; empty = allow all origins |

## Status and behavior guarantees

- Every read endpoint regenerates from the live store per request;
  nothing served is a cached build.
- Inserts are idempotent (duplicates change nothing and emit no event);
  inserts that introduce new vocabulary fire a schema-change event on
  `/topics/schema`.
- Subscriptions, leases, and the delivery log persist in the store's
  own named graphs and survive restarts and replica failover.
- Tokens are compared in constant time; subscriber secrets are
  encrypted at rest when `SEMWEB_SECRET_KEY` is set.
- The WebSub hub implements the W3C Recommendation: intent verification
  (§5.3), lease expiry enforcement, full-content distribution with
  HMAC signature (§7), retry contract, `410 Gone` termination, and
  discovery Link headers (§4). See [WebSub.md](WebSub.md).