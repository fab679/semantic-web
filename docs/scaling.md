# Hardening & Scale-Out Roadmap

The service implements the full design — fragment streaming with cursor
pagination and cardinality counters, live self-description, the agent
manifest, and a spec-compliant WebSub hub with durable subscriptions,
a bounded delivery queue and worker pool, rate limiting, write auth and
Prometheus metrics — on a store contract that is the SPARQL 1.1
Protocol itself. This document states what is implemented and carries
the concrete designs for what comes next, in order of leverage.

## 0. Measured numbers

Reproduce: `uv run --project bench bench.py load --triples 100000`,
then `read`, `fanout --fanout 50`, `crash` (harness in `bench/`,
subscriber instrument in `demo-subscriber`; runs against the compose
stack on one machine).

| Measurement | Result (single container, 100k triples) |
|---|---|
| Bulk load | 100k triples in 3.2–3.9s (~26k–32k triples/s via Graph Store) |
| Fragments paged full scan (100) | p50 5.0 ms · p95 6.8 ms |
| Fragments predicate-bound | p50 4.0 ms · p95 5.4 ms |
| Fragments subject-bound | p50 4.1 ms · p95 6.0 ms |
| Live self-description (`GET /`) | p50 4.6 ms · p95 6.0 ms |
| Agent manifest (`GET /manifest`, GROUP BYs) | p50 7.7 ms · p95 9.7 ms |
| SPARQL COUNT over 100k | p50 4.4 ms · p95 5.1 ms |
| WebSub fan-out, 50 subscribers, one insert | 50/50 delivered · p50 31 ms · p95 45 ms |
| Crash redelivery (planted pending entry) | **1.1 s** (restart + log scan + signed delivery) |

Notes: latency measured from the host against the containerized stack
(loopback HTTP); fan-out on small content — full-content delivery scales
with topic size, so measure topics at their real size. The crash test
exercises the durable-log path end to end: stop → plant → restart →
signed delivery. It also caught a real production bug during
development: a usize underflow in the retry loop killed delivery workers
silently on their first failure (now fixed + panic-contained per
delivery).

## 1. What is implemented

| Concern | Implementation |
|---|---|
| Fragment reads | Cursor pagination (`after=`, O(1) seek) + legacy offset; results ordered by string forms of (s,p,o); hypermedia control line carries both mechanisms |
| Cardinalities | Maintained per-class/per-predicate counters (loaded at startup, updated on every write) serve `count_estimate` without a COUNT round-trip; counters are exact for writes through this service |
| Self-description + `@context` | Live from store state; prefix-compacted; no build step |
| Agent manifest | `/manifest`: schema fingerprint (sha256 of the shape graph), live GROUP BY cardinalities, `rdfs:comment`/`skos:definition` descriptions, executable example SPARQL per class; fingerprint also carried on `/topics/schema` |
| Execution plane | Read-only `/sparql` passthrough (read-only by construction — the store's query endpoint cannot execute updates) |
| WebSub conformance | Subscribe/unsubscribe/publish, intent verification (§5.3), leases enforced with clamping + expiry sweep (§5.3), query-string preservation (§5.1.1), denied notifications (§5.2), full-content distribution with Link headers + matching Content-Type (§7), HMAC sha256 signing (§7.1), retry schedule, 410 termination (§7) |
| Hub durability | Subscriptions persist to a dedicated named graph in the store; restart reloads them, expired leases are dropped on load |
| Delivery pipeline | Bounded queue (`SEMWEB_QUEUE_CAPACITY`) drained by `SEMWEB_DELIVERY_WORKERS`; explicit backpressure (full queue keeps the entry in the durable log); retries 1s/5s/15s per §7 |
| Delivery durability | Durable delivery log: logged before enqueue, acked on success/410/exhaustion; crash mid-flight redelivers on startup; other replicas pick up their shard's entries on a periodic scan (age-guarded) |
| Multi-replica | `SEMWEB_REPLICA_COUNT`/`SEMWEB_REPLICA_INDEX`: consistent-hash shard per (topic, callback); replicas share the store and poll it (`SEMWEB_SUBS_REFRESH_SECS`) for new subscriptions and pending deliveries; `hub.mode=publish` is idempotent |
| Secret protection | AES-256-GCM encryption at rest (`SEMWEB_SECRET_KEY`); legacy plaintext values decrypt transparently; AEAD rejects tampering |
| Live sessions | SSE `/events` (header-only events + keepalives + Lagged resync signal), complementary to WebSub |
| Callback policies | Optional HTTPS-callback requirement for secret'd subscriptions; optional callback-host allowlist (§5.1) |
| Hub auth | `SEMWEB_HUB_TOKEN` bearer on POST /hub; `SEMWEB_WRITE_TOKEN` bearer on the write path |
| MCP delivery | POST /mcp: full MCP server (initialize / resources / tools / prompts — 6 graph tools, the manifest resource, 2 guided prompts) |
| SHACL in manifest | Shapes loaded from `SEMWEB_SHACL_PATH` into a shapes graph; per-class property shapes (path/min/max/datatype) in the manifest |
| Abuse control | Per-callback token bucket on subscription requests (429 + metric); §8.2 challenge restrictions; `hub.secret` length enforcement |
| Auth | `SEMWEB_WRITE_TOKEN` bearer auth on mutating endpoints; read endpoints open |
| Observability | `/metrics` (Prometheus text), `/health` (liveness + store readiness), per-mutation `event_id` threading write → publish → fan-out |
| Load harness | `bench/`: bulk load, fragment latency percentiles, fan-out latency, crash redelivery — numbers in §0 |
| Store contract | SPARQL 1.1 Protocol + Graph Store Protocol — any conforming endpoint is a drop-in; query and update URLs are separately configurable (read replicas = config) |

## 2. Write path at scale

`/admin/insert` is the reference write path. Production deployments
bring their own ingestion (SPARQL UPDATE endpoint, ETL) and must
satisfy one invariant: **whatever writes the store must publish the
affected topics**, because WebSub distribution is pull-on-publish, not
change-data-capture.

1. One write API owns store mutations and, in one transaction boundary,
   publishes the affected topics (the service's counter updates and
   new-term detection show the shape).
2. Cardinality counters are exact for writes through the service; for
   out-of-band writers, re-run the GROUP BY load periodically or move
   counting into the write path of every writer.
3. Change-data-capture for many independent writers: oxigraph exposes
   no CDC stream, so either funnel all writes through the service's
   write API, or adopt a patch log (RDF Delta-style) / WAL tail that
   turns every writer into a publisher without code coupling.

## 3. What remains (deployment-level)

The service-side scaling work is complete. What remains is operational,
not code:

1. **TLS termination** — terminate TLS at the edge (Caddy/nginx/envoy
   sidecar); oxigraph itself has no TLS server, so service↔store mTLS
   also rides the mesh/sidecar. The service's store URLs already accept
   https.
2. **KMS-backed key management** — `SEMWEB_SECRET_KEY` comes from the
   environment; production deployments should inject it from a KMS/secret
   manager and rotate it (rotation = re-subscription, which the spec's
   override-after-verification semantics support without downtime).
3. **Per-tenant productization** — authz on topics per tenant identity,
   per-tenant rate limits and metrics labels (the named-graph tenancy
   primitive is in place).
4. **CDC for out-of-band writers** — counters refresh periodically
   (`SEMWEB_COUNTER_REFRESH_SECS`); deployments with many independent
   writers should either funnel writes through the service's write API or
   adopt a patch log (RDF Delta-style) that turns every writer into a
   publisher.

## 7. Non-goals

- A pre-compiled query contract: the whole point is that the
  self-description is *live*; freezing it into a compiled artifact would
  reintroduce the rebuild/versioning machinery the design exists to
  avoid (architecture.md §2).
- A bespoke transport: everything rides on REST/HTTP/WebSub/NDJSON/MCP.