# Configuration reference

Everything is configured through environment variables — no config
files, no recompiles. Defaults in parentheses.

## Store

| Variable | Default | Meaning |
|---|---|---|
| `SEMWEB_SPARQL_ENDPOINT` | `http://localhost:7878/query` | read endpoint (SPARQL 1.1 Protocol). Point at a load-balanced replica set for read scaling |
| `SEMWEB_SPARQL_UPDATE` | `http://localhost:7878/update` | write endpoint (separate on purpose — the `/sparql` passthrough can never write) |
| `SEMWEB_SEED_PATH` | unset | Turtle file loaded into the default graph at startup (retried while the store boots; idempotent) |
| `SEMWEB_SHACL_PATH` | unset | SHACL shapes file loaded into the shapes graph, surfaced by `GET /manifest` and MCP |

## Service

| Variable | Default | Meaning |
|---|---|---|
| `SEMWEB_PORT` | `8484` | listen port (local runs). The project uses dedicated ports so it never conflicts with anything else on a machine: service **8484**, Oxigraph **7878**, scale replica 2 **8485** |
| `SEMWEB_PUBLIC_URL` | `http://localhost:$PORT` | absolute base used for WebSub discovery (`rel=self` / `rel=hub`) Link headers |
| `RUST_LOG` | `info` | tracing filter (`debug` for delivery/verification detail) |

## WebSub hub

| Variable | Default | Meaning |
|---|---|---|
| `SEMWEB_QUEUE_CAPACITY` | `4096` | bounded delivery queue; overflow entries stay in the durable log |
| `SEMWEB_DELIVERY_WORKERS` | `8` | concurrent delivery workers |
| `SEMWEB_SECRET_KEY` | unset (plaintext at rest) | 64-hex-char (32-byte) AES-256-GCM key; encrypts subscriber secrets in the store. Generate: `openssl rand -hex 32` |
| `SEMWEB_REQUIRE_HTTPS_CALLBACKS` | `false` | reject `http://` callbacks that register a `hub.secret` (spec §8.2 recommends HTTPS) |
| `SEMWEB_CALLBACK_ALLOWLIST` | empty (allow all) | comma-separated callback host suffixes, e.g. `mysvc.example.com,api.example.org` |
| `SEMWEB_OPEN_HUB` | `false` | accept third-party topics (any publisher's URL) — the hub serves external publishers (§5.1 policy); content fetched at publish time |
| `SEMWEB_HUB_URLS` | empty (only ours) | comma-separated external hubs advertised in Link headers and notified on every mutation (§4 fault tolerance / §6) |

Lease policy is hub-fixed per spec §5.3: requested leases clamped to
[60s, 10 days], default 1 day; expiry enforced by a 30s sweeper.

## Authentication

| Variable | Effect |
|---|---|
| `SEMWEB_WRITE_TOKEN` | when set, mutating endpoints (`POST /admin/insert`, MCP `insert_triple`) require `Authorization: Bearer <token>` |
| `SEMWEB_HUB_TOKEN` | when set, `POST /hub` (subscribe/unsubscribe/publish) requires the same header |

Read endpoints stay open in both cases.

## Scale-out

| Variable | Default | Effect |
|---|---|---|
| `SEMWEB_REPLICA_COUNT` | `1` | total replicas sharing one store |
| `SEMWEB_REPLICA_INDEX` | `0` | this replica's shard index (0..COUNT-1) |
| `SEMWEB_SUBS_REFRESH_SECS` | `10` | how often replicas poll the shared store for new subscriptions/renewals and pending deliveries in their shard |
| `SEMWEB_COUNTER_REFRESH_SECS` | `300` | cardinality-counter refresh (bounds drift from out-of-band writers) |

Sharding: a (topic, callback) subscription is owned by the replica
`hash(subscription id) % COUNT == INDEX`. Publishers on any replica log
deliveries for all shards; owners pick up their shard's entries on the
periodic scan.

## Vocabulary

| Variable | Effect |
|---|---|
| `SEMWEB_EXTRA_PREFIXES` | comma-separated `name=namespace` pairs — friendly prefixes for private namespaces, e.g. `ex=http://example.org/vocab/,acme=http://acme.example/ns#` |
| `SEMWEB_TERM_ALIASES` | comma-separated `name=URI` pairs — bare friendly names for exact URIs, emitted in `/context.jsonld` as JSON-LD term definitions (round-trippable). Demo default: `name`, `knows`, `employer`, `employs`, `founded` |

Standard vocabularies (rdf, rdfs, owl, xsd, sh, foaf, schema, dcterms,
skos) always compact; extras never shadow them.

## Demo compose defaults

`docker-compose.yml` sets: `SEMWEB_WRITE_TOKEN=demo-write-token`,
`SEMWEB_SECRET_KEY=<demo key>`, `SEMWEB_SHACL_PATH=/app/shapes.ttl`.
Change the secret key for anything real (`openssl rand -hex 32`) — it is
the subscription-secret encryption root.