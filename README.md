# Streaming Semantic Fragments

A live, agent-readable surface over RDF, written in Rust: Triple Pattern
Fragments streamed as NDJSON-LD over a SPARQL 1.1 Protocol store, live
self-description with no build step, and real-time change push via a
WebSub hub implementing the W3C Recommendation
([docs/WebSub.md](docs/WebSub.md)). Plain HTTP only — REST, chunked
streaming, JSON-LD. No new protocol, no translation layer in front of the store.

- [Architecture](docs/architecture.md) — the design, the three
  primitives, the WebSub conformance map
- [API reference](docs/api.md) — every endpoint with curl examples
- [Hardening & scale-out](docs/scaling.md) — production roadmap
- [WebSub specification](docs/WebSub.md) — the W3C REC this hub implements

## Project layout

Coding convention: one concern per small file (nothing over ~300 lines)
so every piece is editable in isolation.

```
Cargo.toml                 workspace
crates/semweb/             the service
  src/main.rs              wiring + background tasks
  src/state.rs             env-based configuration
  src/util.rs              shared helpers (schema fingerprint)
  src/jsonld.rs            bindings -> compacted JSON-LD lines
  src/context.rs           namespace-prefix compaction + live @context
  src/api/                 HTTP surface, one endpoint group per file
    mod.rs                 router + shared helpers
    self_description.rs    GET / and /context.jsonld
    fragments.rs           GET /fragments (TPF, cursor pagination, named graphs)
    sparql.rs              GET /sparql (read-only execution plane)
    manifest.rs            GET /manifest (agent manifest + SHACL shapes)
    events.rs              GET /events (SSE change feed)
    hub_endpoints.rs       POST|GET /hub, GET /topics/{name}
    write.rs               POST /admin/insert (token-gated)
    mcp.rs                 POST /mcp (minimal MCP resource server)
    health_metrics.rs      GET /health, GET /metrics
  src/hub/
    mod.rs                 hub core: subscriptions, publish fan-out (durable
                           log + replica sharding), SSE broadcast, policies
    verification.rs        spec §5.3 intent verification + leases
    delivery.rs            worker pool + §7 distribution + HMAC signing
    crypto.rs              AES-256-GCM secret encryption at rest
    rate_limit.rs          per-callback token bucket
    metrics.rs             hub counters
  src/store/
    mod.rs                 SPARQL 1.1 Protocol client core
    fragments.rs           TPF lookup + pagination cursor
    cardinality.rs         maintained counters
    persistence.rs         hub-graph durability (subscriptions, delivery
                           log, SHACL shapes)
  src/bin/demo-subscriber.rs   spec-conformant demo WebSub subscriber
  sample_data.ttl          demo seed
  shapes.ttl               demo SHACL shapes (agent manifest)
Dockerfile                 multi-stage build (service + demo subscriber)
docker-compose.yml         oxigraph + service (+ demo profile)
.github/workflows/ci.yml   build + unit tests
docs/                      architecture, api, scaling, WebSub spec
```

## Quickstart

### Full topology (docker)

```sh
docker compose up --build
```

Oxigraph serves the SPARQL 1.1 Protocol at <http://localhost:7878>
(web UI at <http://localhost:7878>), the service at
<http://localhost:8484>.

```sh
open http://localhost:8484/ui                                                 # graph explorer (browser)
curl http://localhost:8484/manifest                                           # agent manifest
curl http://localhost:8484/                                                   # live self-description
curl "http://localhost:8484/fragments?predicate=http://schema.org/worksFor"   # streamed fragment
```

### Locally (against a running Oxigraph)

```sh
cargo run -p semweb
# env: SEMWEB_SPARQL_ENDPOINT (default http://localhost:7878/query),
#      SEMWEB_SPARQL_UPDATE   (default http://localhost:7878/update),
#      SEMWEB_PUBLIC_URL, SEMWEB_SEED_PATH, SEMWEB_SHACL_PATH,
#      SEMWEB_QUEUE_CAPACITY, SEMWEB_DELIVERY_WORKERS,
#      SEMWEB_WRITE_TOKEN, SEMWEB_HUB_TOKEN, SEMWEB_SECRET_KEY,
#      SEMWEB_REPLICA_COUNT/INDEX, SEMWEB_SUBS_REFRESH_SECS,
#      SEMWEB_COUNTER_REFRESH_SECS, SEMWEB_REQUIRE_HTTPS_CALLBACKS,
#      SEMWEB_CALLBACK_ALLOWLIST, SEMWEB_EXTRA_PREFIXES
```

### Watch the real-time loop

```sh
docker compose --profile demo up -d        # adds demo-subscriber to the network

curl -X POST http://localhost:8484/hub \
  -d "hub.mode=subscribe" \
  -d "hub.topic=http://localhost:8484/topics/data" \
  -d "hub.callback=http://demo-subscriber:9000/callback" \
  -d "hub.secret=demo"

curl -X POST http://localhost:8484/admin/insert \
  -H "Content-Type: application/json" \
  -d '{"subject":"http://example.org/erin","predicate":"http://xmlns.com/foaf/0.1/knows","object":"http://example.org/alice"}'

docker compose logs -f demo-subscriber     # verified sha256-signed delivery arrives
```

The demo-subscriber binary is a spec-conformant WebSub subscriber
(intent-verification echo with safe media type + nosniff, HMAC signature
validation) — use it as the reference consumer for your own agents and
services.

Inserts need the configured write token (compose sets
`SEMWEB_WRITE_TOKEN: demo-write-token`):

```sh
curl -X POST http://localhost:8484/admin/insert \
  -H "Authorization: Bearer demo-write-token" \
  -H "Content-Type: application/json" \
  -d '{"subject":"http://example.org/erin","predicate":"http://xmlns.com/foaf/0.1/knows","object":"http://example.org/alice"}'
```

## Status

Production implementation in Rust (axum + tokio, SPARQL 1.1 Protocol
store contract). The WebSub hub follows the W3C Recommendation
(leases, verification handshake, retries, 410 termination, discovery
headers, authenticated distribution) with durable subscriptions,
AES-GCM-encrypted secrets, a crash-safe delivery log, multi-replica
sharding, a bounded delivery queue with worker pool, rate limiting,
bearer auth, Prometheus metrics, the agent manifest (with SHACL shapes
and MCP delivery), SSE for live sessions, and named-graph tenancy.
Remaining operational concerns: [docs/scaling.md](docs/scaling.md).