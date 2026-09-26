# Operations

How to run the infrastructure, what it costs, how it fails, and how it
is observed. This is the operator-facing page; the concepts are in
[how-it-works.md](how-it-works.md).

## Running

### Docker (the full topology)

```sh
docker compose up --build
```

Starts two services: the SPARQL 1.1 store (Oxigraph, :7878) and this
service (:8484). Configuration is entirely environment variables —
see [reference.md](reference.md) for the complete table. The compose
file also carries two profiles:

- `demo` — adds the reference subscriber container, used by the
  conformance battery
- `scale` — a second service replica forming one 2-shard hub cluster
  over the shared store

### Bare metal / containerless

```sh
cargo run -p semweb        # against any SPARQL 1.1 endpoint
```

The store is addressed by URL only (`SEMWEB_SPARQL_ENDPOINT`,
`SEMWEB_SPARQL_UPDATE`); any conforming store works, and read replicas
can be placed behind a load balancer by pointing at the balancer.

## Failure modes and their answers

| Concern | Mechanism | Knobs |
|---|---|---|
| Subscriptions lost on restart | Subscriptions and the delivery log persist as named graphs in the store; recovered at boot | — |
| Crash between event and delivery | Durable delivery log; workers ack only after delivery; overflow stays queued | `SEMWEB_QUEUE_CAPACITY` |
| Slow subscribers | Bounded queue + worker pool; per-callback rate limiting | `SEMWEB_DELIVERY_WORKERS` |
| Counter drift (writers that bypass the service) | Periodic recount from the store | `SEMWEB_COUNTER_REFRESH_SECS` (default 300) |
| Hub traffic beyond one box | Consistent-hash sharding of the subscription space across replicas; cross-replica refresh | `SEMWEB_REPLICA_COUNT`, `SEMWEB_REPLICA_INDEX`, `SEMWEB_SUBS_REFRESH_SECS` |
| Store boots slowly (compose races) | Startup loads retry with backoff; the service comes up degraded, not down | — |

## Security posture

| Surface | Control |
|---|---|
| Writes | Bearer token (`SEMWEB_WRITE_TOKEN`), constant-time comparison; unset token = open writes |
| Who may subscribe | Optional bearer token (`SEMWEB_HUB_TOKEN`) |
| Subscriber secrets | AES-256-GCM encrypted at rest when `SEMWEB_SECRET_KEY` is set |
| Delivery authenticity | HMAC-SHA256 with the per-subscription secret |
| Subscriber callbacks | Optional HTTPS enforcement (`SEMWEB_REQUIRE_HTTPS_CALLBACKS`) and host allowlist (`SEMWEB_CALLBACK_ALLOWLIST`); the same allowlist governs open-hub fetches |
| Signed manifest | Ed25519 under the publisher's DID when `SEMWEB_SIGNING_KEY` is set — see [trust.md](trust.md) |
| CORS | `SEMWEB_CORS_ORIGINS` allowlist; empty = any origin (tighten for public deployments) |

Deployment hardening beyond the service (TLS termination, secret
management, network policy) is standard-reverse-proxy work and out of
scope here by design: the service speaks plain HTTP and expects to sit
behind TLS at the edge.

## Observability

- `GET /health` — liveness plus store reachability (for orchestrators)
- `GET /metrics` — Prometheus text format: hub counters (subscriptions,
  deliveries, retries, shard behavior) and service counters
  (fragment requests, inserts)
- Logs — structured, level-filtered via `RUST_LOG` (default `info`)

## Verifying a deployment

The end-to-end conformance battery is the executable specification:

```sh
./scripts/e2e.sh
```

It boots the docker topology and checks every mechanism: discovery
headers, cursor pagination walking without overlap, manifest fields,
MCP initialize/list/read/call, write auth and idempotent inserts, the
full WebSub loop (intent verification → signed delivery → unsubscribe),
the trust layer (DID document, signed manifest, verification gates
rejecting tampered and unsigned input), SSE, CORS, and metrics.
CI runs this battery on every change.