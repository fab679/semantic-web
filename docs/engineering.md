# For engineering teams

The operational view: what runs where, how it fails, how it scales, what
to watch, and what to secure. Everything here is implemented and
exercised by the automated battery (`scripts/e2e.sh`).

## Topology

```
        clients (apps, agents, dashboards)
              │  plain HTTP
              ▼
┌───────────────────────────┐      ┌──────────────────────────┐
│  semantic-web (Rust)      │ HTTP │  oxigraph / any SPARQL   │
│  axum + tokio, 1 binary   ├─────►│  1.1 Protocol endpoint   │
│  stateless except cache   │      │  (query /update /store)  │
└────────────┬──────────────┘      └──────────────────────────┘
             │  WebSub POST + SSE
             ▼
     subscribers (your services)
```

- **The service is stateless except in-process caches** (cardinality
  counters, in-memory subscription index). All durable state lives in
  the store, in named graphs: subscriptions, the delivery log, SHACL
  shapes. Kill the container and nothing is lost.
- **The store is a protocol, not a product.** The service speaks SPARQL
  1.1 (`/query`, `/update`) and the Graph Store Protocol (`/store`).
  Oxigraph today; a Jena/Fuseki or Virtuoso endpoint is a config change
  (`SEMWEB_SPARQL_ENDPOINT`, `SEMWEB_SPARQL_UPDATE`).

## Capacity and scale path

| Dimension | Mechanism | Config |
|---|---|---|
| Read throughput | store-bound; put read replicas behind a LB (query and update URLs are separate env vars) | `SEMWEB_SPARQL_ENDPOINT` |
| Fragment paging | cursor (`after=`) seek, not OFFSET scans; exact counts from maintained counters | — |
| Counters drift (out-of-band writers) | periodic GROUP BY refresh | `SEMWEB_COUNTER_REFRESH_SECS` (default 300s) |
| Delivery concurrency | fixed worker pool | `SEMWEB_DELIVERY_WORKERS` (default 8) |
| Backpressure | bounded queue; overflow entries stay in the durable delivery log and are picked up later | `SEMWEB_QUEUE_CAPACITY` |
| Horizontal scale-out | replicas share the store; consistent-hash shard per (topic, callback); each replica publishes/delivers only its shard | `SEMWEB_REPLICA_COUNT`, `SEMWEB_REPLICA_INDEX`, `SEMWEB_SUBS_REFRESH_SECS` |
| Multi-tenancy | named graphs via `?graph=` (reads) and `"graph"` (writes) | — |

**Delivery semantics: at-least-once.** Every delivery is written to the
durable log with a claim before it is queued, and acked (deleted) when
finished. The claim (~3 minutes) covers the worst-case retry schedule,
so an in-flight delivery is never re-enqueued by another worker or
replica. A worker crash leaves the claim to expire, after which another
replica redelivers. Duplicates are possible only across
crash/retry windows — make consumer handling idempotent.

## Failure behavior (what to expect in incidents)

| Failure | Behavior |
|---|---|
| Store down | `/health` reports `degraded`, reads 5xx; seed + shapes loads retry for ~5 minutes at startup before giving up |
| Subscriber down (callback unreachable at subscription time) | verification retried 3x (2s/5s apart), then an explicit §5.2 denied notification is sent; no phantom subscription |
| Subscriber down (during delivery) | retries 1s/5s/15s, then the notification is dropped from the log; **subscription survives until lease end**; the next update retries delivery (spec §7) |
| Service crashed mid-delivery | delivery-log entry's claim expires (~3 min) → any replica redelivers on its scan (at-least-once) |
| Queue full | entry stays in the durable log (redelivered later), loud log + `semweb_queue_dropped_total` |
| Callback returns 410 | subscription terminated |
| Subscriber secret undecryptable (key changed/missing) | subscription survives, delivery unsigned until renewal; set `SEMWEB_SECRET_KEY` (64 hex chars) to encrypt at rest |
| Out-of-band SPARQL writes | counters drift until refresh; the agent manifest never inherits drift (live GROUP BY) |
| Delivery worker panics | panic is contained per delivery; the worker survives (spec §7) and the retry counter increments |

## Security posture

| Area | Mechanism | Config |
|---|---|---|
| Write path | bearer token, constant-time compare | `SEMWEB_WRITE_TOKEN` |
| Hub surface (who may subscribe) | bearer token (optional) | `SEMWEB_HUB_TOKEN` |
| Subscriber secrets at rest | AES-256-GCM (AEAD, tamper-rejecting) | `SEMWEB_SECRET_KEY` |
| Hub-state integrity | service-reserved named graphs are rejected on the write path and hidden from `?graph=` reads — hub state cannot be forged through the public API | built in |
| Fetch policy (open hub) | third-party topic URLs respect the same host allowlist as callbacks | `SEMWEB_CALLBACK_ALLOWLIST` |
| Notification integrity | HMAC-SHA256 `X-Hub-Signature` per §7.1 (sha256 minimum per §8.3) | per-subscription secret |
| Intent verification | single-use random challenge, spec charset, no-binary rule (§8.2) | — |
| Abuse control | per-callback token bucket | burst 10, refill 10/min |
| Callback policies | HTTPS requirement for secret'd callbacks; host allowlist | `SEMWEB_REQUIRE_HTTPS_CALLBACKS`, `SEMWEB_CALLBACK_ALLOWLIST` |

Not yet implemented (deployment-level): TLS termination (edge proxy —
oxigraph itself has no TLS), KMS-managed key injection/rotation, mTLS
between service and store (rides a mesh/sidecar).

## Observability

- `/metrics` — Prometheus text: verifications (ok/failed), deliveries
  (success/exhausted), retries, 410 terminations, denied notifications,
  queue drops, rate-limit hits, **redeliveries after restart**,
  **shard-routing counters**, active subscriptions, fragment requests,
  inserts.
- `/health` — liveness + store readiness (the compose healthcheck target).
- Every mutation carries an `event_id`: one insert → write log → publish
  → delivery log → subscriber acks, all traceable by that id.
- Structured logs via `tracing` (`RUST_LOG=info`); the WebSub hub logs
  every lifecycle transition with topic + callback + event id.

## Runs like this

```sh
docker compose up --build -d                       # single replica
docker compose --profile demo up -d                # + demo subscriber
SEMWEB_REPLICA_COUNT=2 docker compose --profile scale up -d   # 2-shard cluster
./scripts/e2e.sh                                   # full battery (18 checks), exit code = verdict
cargo test                                         # 24 unit tests
uv run --project bench bench.py read               # latency harness (bench/)
```

One binary, one store, no queue infrastructure, no schema registry.
Everything a platform team needs to reason about the system is in the
environment, the metrics, and the named graphs.