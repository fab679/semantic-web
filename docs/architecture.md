# Architecture

> Streaming Semantic Fragments — a live, agent-readable surface over RDF,
> built on standard web technologies only.

## 1. What this is

A Rust service that puts RDF data "in simple terms for users and agents
together", and keeps that surface **real-time as the ontology changes**.
It is built from three primitives, all carried over standards that
already exist (REST, HTTP chunked streaming, NDJSON, JSON-LD, WebSub):

1. **Triple Pattern Fragments** — the read primitive (§3.1)
2. **Live self-description** — the discovery/catalog primitive (§3.2)
3. **WebSub push** — the real-time change primitive (§3.3)

No new protocol, no new query language, no translation layer in front
of the store. SPARQL remains the execution plane; the service's
contribution is the live discovery and notification layer around it.

## 2. Design decisions

The service is the missing layer between an RDF store and its consumers —
not a replacement for any of them. Each decision below follows from one
constraint: **the surface must stay correct while the data evolves**.

| Decision | Rationale |
|---|---|
| SPARQL is the execution plane | The store speaks SPARQL 1.1 already. The service adds discovery and notification around it, never a translation layer in front of it. |
| Self-description, not compiled schema | "What exists?" is answered from the live store on every request. No build step, no staleness window: the moment the ontology changes, the next request reflects it. |
| Push over poll | WebSub for durable subscribers (they can be offline between events), SSE for live sessions. Both come from one in-process publish bus. |
| Plain HTTP only | REST verbs, query params, standard media types. Works with curl, proxies, caches, any HTTP client on day one — and every mechanism (streaming, auth headers, retries) composes with the rest of the web. |
| Namespace-prefix compaction, never per-term tables | Domain terms are *never* hardcoded (see §4). Standard namespaces get conventional prefixes; anything else stays a full URI or gets a runtime-registered prefix. Ontology growth requires zero code changes. |
| Hypermedia over convention | Responses embed the controls a client needs to keep going (`after` cursors, topic lists, discovery Link headers) — no out-of-band schema required. |
| At-least-once delivery | WebSub's contract plus a durable delivery log: correctness over exactly-once convenience; consumers treat notifications as idempotent. |

## 3. The three primitives

### 3.1 Triple Pattern Fragments, streamed (`GET /fragments`)

A client supplies zero to three of `subject`/`predicate`/`object`;
omitted positions are wildcards — exactly the `{?s ?p ?o}` pattern
semantics of the Triple Pattern Fragments specification.

- The pattern is expressed as a SPARQL `SELECT` against the store.
  Concrete positions are `BIND`-ed so every result row carries all
  three terms regardless of which positions were wildcards (SPARQL
  engines omit variables that do not occur in the pattern — see
  store/fragments.rs `pattern_fragment`).
- **Cursor pagination** (`?after=<opaque cursor>`): the cursor is the
  string form of the last-seen triple (base64 JSON), and results are
  ordered by the string forms of (s, p, o) so pages are deterministic
  and seeking is O(1) — no linear OFFSET scans. `?offset` remains for
  legacy clients; the control line carries both (`after` and `next`).
- **Cardinality counters**: per-class/per-predicate triple counts are
  maintained in-process (loaded with GROUP BY queries at startup,
  updated on every write through this service). Patterns the counters
  cover get their `count_estimate` without a COUNT round-trip; the
  rest fall back to an exact server COUNT.
- The response is **NDJSON: one compacted JSON-LD statement per line**,
  streamed as each line is ready. A client can act on line 1 without
  waiting for the page.
- The final line is the hypermedia control:

```json
{"@control": "metadata", "count_estimate": 5,
 "after": "WyJodHRw...", "next": "/fragments?...&offset=100"}
```

Why one *triple* per line and not grouped *node* objects: grouping
requires buffering every triple for a subject before emitting, which
breaks streaming for patterns matching millions of subjects. Grouping
is a cheap client-side operation; ungrouping a large buffered node
server-side is not.

### 3.2 Live self-description (`GET /`, `GET /context.jsonld`, `GET /manifest`)

The "semantic catalog" piece of the design. Two cheap SPARQL queries
against the **current** store state on every call ("which classes are
in use", "which predicates are in use") produce a description that can
never go stale — no build step, no rebuild window:

```json
{
  "@context": "/context.jsonld",
  "generatedFrom": "live store state (not a cached build)",
  "storeMode": "sparql-rust",
  "classes": ["foaf:Organization", "foaf:Person"],
  "predicates": ["schema:employee", "schema:worksFor", "rdf:type", "foaf:knows", "foaf:name"],
  "controls": {
    "fragments": "/fragments{?subject,predicate,object,after,limit,offset}",
    "sparql": "/sparql{?query}",
    "manifest": "/manifest",
    "hub": "/hub",
    "topics": ["/topics/data", "/topics/schema"]
  }
}
```

`/context.jsonld` is generated live the same way (namespaces in use →
prefixes). `GET /manifest` is the agent-facing evolution of this
catalog (§4 below). SPARQL itself is reachable through the same origin
and auth model via the read-only `GET /sparql?query=...` passthrough —
the execution plane, one hop from the discovery plane.

### 3.3 Real-time push via WebSub (`POST /hub`, `GET /topics/{name}`)

The change-notification primitive implements the W3C WebSub
Recommendation (`docs/WebSub.md`). Publisher and hub are this service
(the simplest valid topology); subscribers are any HTTP-callable app or
agent-side service. Two topics:

- `/topics/data` — instance data changed
- `/topics/schema` — the ontology *surface* changed (a class or
  predicate appeared that wasn't in use before)

The full conformance map, section numbers referring to docs/WebSub.md:

| Spec requirement | Implementation |
|---|---|
| §3.1.1.3 hub accepts hub.callback/mode/topic (+hub.secret, hub.lease_seconds) | api/hub_endpoints.rs `hub_post`, hub/mod.rs `subscribe`/`unsubscribe` |
| §5.1.2 `202` immediately; MUST NOT depend on verification outcome | verification runs in a spawned task after the response |
| §5.3 intent verification: GET callback with hub.mode, hub.topic, hub.challenge, hub.lease_seconds | `verify_and_commit` |
| §5.3 challenge charset (`+ - . / 0-9 = A-Z _ a-z`) and §8.2 no-binary | `generate_challenge` (URL-safe subset, 43 chars) |
| §5.3.1 2xx + body == challenge commits; wrong body / 3xx / 4xx leaves state unchanged | `verify_and_commit` (unreachable + 5xx retried 3x; final subscribe failure sends an explicit §5.2 denied notification) |
| §5.1 re-requests of active subscriptions allowed; state overridden only after verification | commit-on-success in `verify_and_commit` |
| §5.1 hub.secret MUST be < 200 bytes | validated in hub/mod.rs, 4xx otherwise |
| §5.3 hubs MUST enforce lease expirations; MUST NOT issue perpetual leases | default 1 day, clamped to [60s, 10 days], 30s sweeper task |
| §5.1.1 callback query string preserved, never overwritten; params travel in the URL | `append_query` |
| §5.2 denied notifications (hub.mode=denied + hub.reason) | `send_denied` |
| §6 publisher→hub notification (hub.mode=publish & hub.url) | api/hub_endpoints.rs publish branch |
| §7 distribution = POST of the **full** topic contents, Content-Type matching the topic, Link headers rel=self + rel=hub | `TopicContent` built at publish time; queued, delivered by workers |
| §7.1 HMAC `X-Hub-Signature` when hub.secret supplied | sha256 (§8.3: SHA-1 ruled out, sha256 is the minimum) |
| §7 retries up to self-imposed limits; subscription stays active until lease end; 410 Gone terminates | worker retry loop (1s/5s/15s schedule) |
| §4 discovery: topic resources advertise rel=self + rel=hub | `GET /topics/{name}` Link headers, single representation per §4.1 |

Scale-out mechanisms layered onto the hub (each verified by the test
battery):

- **Subscription persistence** — subscriptions persist to a dedicated
  named graph in the store; a restart reloads them, expired leases are
  dropped on load (§5.3). No extra infrastructure.
- **Secret encryption at rest** — `hub.secret` values are AES-256-GCM
  encrypted (`SEMWEB_SECRET_KEY`) before persistence; legacy plaintext
  values keep working and tampering is rejected by the AEAD.
- **Durable delivery log** — every delivery is logged in the store
  before enqueue and acked on completion; a crash mid-flight redelivers
  on startup (at-least-once). Publishes from any replica are picked up
  by the owning replica's periodic log scan (age-guarded so in-flight
  retries are not duplicated).
- **Multi-replica sharding** — with `SEMWEB_REPLICA_COUNT > 1` a
  (topic, callback) is owned by exactly one replica (consistent hash of
  its subscription id); replicas share the store, poll it for new
  subscriptions and pending deliveries, and deliver only their own
  shard. Verified with two replicas against one store.
- **Bounded delivery queue + worker pool** — `SEMWEB_QUEUE_CAPACITY` +
  `SEMWEB_DELIVERY_WORKERS`; explicit backpressure, publish latency
  decoupled from delivery.
- **Rate limiting** — token bucket per callback (burst 10, refill
  10/min) on subscription requests; exhausted → `429` + metric.
- **Callback policies** (§5.1) — optional HTTPS-callback requirement
  when secrets are used (`SEMWEB_REQUIRE_HTTPS_CALLBACKS`) and an
  optional callback-host allowlist (`SEMWEB_CALLBACK_ALLOWLIST`); under
  the open-hub policy the same allowlist gates third-party topic URLs
  (both are fetch/POST targets — one policy covers both SSRF surfaces).
- **Hub auth** — `SEMWEB_HUB_TOKEN` optionally gates who may
  subscribe/unsubscribe/publish; `SEMWEB_WRITE_TOKEN` gates the write
  path; read endpoints stay open.
- **Observability** — `/metrics` (Prometheus text, incl. redelivery and
  sharding-routing counters), `/health` (liveness + store readiness),
  per-mutation `event_id`, and `GET /events` (SSE) for live sessions.

## 3.4 The agent manifest (`GET /manifest`)

The live-catalog evolution: everything an agent needs to *plan* against
the graph, regenerated live on every request:

- **Schema fingerprint** — sha256 over the sorted class+predicate URI
  set. Carried on the manifest and on `/topics/schema` content, so
  consumers detect missed schema-change events and diff safely.
- **Live cardinalities** — instances per class, triples per predicate
  (GROUP BY on every request; never inherited counter drift).
- **Human descriptions** — pulled live from `rdfs:comment` /
  `skos:definition`: the grounding text agents plan against — what
  terms *mean*, not just which exist.
- **Worked example SPARQL per class** — generated, executable at
  `/sparql`.

Delivery is content, not transport: the same manifest travels as HTTP
GET, as the `/topics/schema` WebSub payload, and as an MCP resource
(`manifest://semantic-web/current`).

## 3.5 Federation (decentralized WebSub)

The hub participates in all three decentralized topologies, per
spec §4/§5.1 policy knobs:

- **Self-hosted hub** (default): publisher and hub are one service;
  scale-out via replicas + durable log.
- **Open hub** (`SEMWEB_OPEN_HUB`): accept third-party topics — the hub
  fetches any publisher's URL at publish time (§7 full-content rule,
  16 MiB cap) and distributes it exactly as served.
- **Federated instances**: any instance can subscribe to any other's
  topics via standard Link-header discovery; `SEMWEB_HUB_URLS`
  advertises/notifies external hubs for fault tolerance (§4).

Full walkthrough: [federation.md](federation.md).

## 4. Namespace-prefix compaction (no domain terms in code)

Consumers should never read raw URIs. Compaction is prefix-based:

- **Standards namespaces** (rdf, rdfs, owl, xsd, sh, foaf, schema,
  dcterms, skos) are registered with their conventional prefixes — the
  same table every Turtle file, SPARQL query and JSON-LD context
  assumes. This is ecosystem vocabulary, not application data: any term
  from these vocabularies compacts automatically (`foaf:mbox`,
  `schema:address`, `rdfs:seeAlso`, …) with zero code changes.
- **Unknown namespaces stay full URIs** — honest and stable, no
  invented names. A private namespace can be given a friendly prefix at
  runtime via `SEMWEB_EXTRA_PREFIXES`
  (`name=namespace`, comma-separated) without recompiling.
- **Term aliases** (`SEMWEB_TERM_ALIASES=name=URI,...`) give the terms
  consumers read most a *bare friendly name* (`foaf:name` → `name`).
  Aliases are emitted into `/context.jsonld` as JSON-LD term
  definitions so JSON-LD processors round-trip them; an alias wins over
  the prefix form for its exact URI, and aliases never shadow a prefix
  name.
- `rdf:type` compacts to `@type` (standard JSON-LD convention).
- Typed literals keep their datatype (`xsd:date`), language-tagged
  literals keep their tag — RDF fidelity over the wire is a
  requirement, not an optimization.

Compaction lives in `context.rs` (`PrefixMap`); the same map both
compacts output and generates the live `/context.jsonld`.

## 5. System overview

```
                       ┌─────────────────────────────────────────┐
                       │            crates/semweb (Rust)         │
   HTTP GET /          │  ┌──────────────┐   ┌───────────────┐   │
  ────────────────────►│  │    api/      │──►│    store/     │   │
    GET /fragments     │  │ (axum HTTP)  │   │  SPARQL 1.1   │   │
  ────────────────────►│  └──────┬───────┘   │   Protocol    │   │
   NDJSON-LD stream    │         │           └───────┬───────┘   │
  ────────────────────►│  ┌──────▼───────┐           │           │
    POST /hub          │  │    hub/      │           ▼           │
  ────────────────────►│  │  WebSub hub  │   ┌───────────────┐   │
    POST /mcp          │  └──────┬───────┘   │ Oxigraph or   │   │
  ────────────────────►│         │           │ any SPARQL 1.1│   │
                       │         │           │ endpoint      │   │
                       └─────────┼───────────┴───────────────┘
                                 │ POST (full topic content,
                                 │ Link headers, HMAC-signed)
                                 ▼
                       ┌─────────────────────┐
                       │ subscribers (any    │
                       │ callback URL, any   │
                       │ app/agent)          │
                       └─────────────────────┘
```

## 6. Components (crates/semweb/src)

Coding convention: one concern per small file — no module over ~300
lines, everything is editable in isolation.

| Module | Responsibility |
|---|---|
| `main.rs` | Wiring: config, router, background tasks (seed + shapes load, counter warm-up/refresh, persistence load, lease sweeper, replica subscription refresh) |
| `state.rs` | Environment-based configuration, shared `AppState` |
| `util.rs` | Shared helpers (schema fingerprint, hex) |
| `jsonld.rs` | SPARQL JSON bindings → compacted JSON-LD lines |
| `context.rs` | Namespace-prefix compaction and live `@context` generation |
| **api/** | |
| `api/mod.rs` | Router, shared helpers (discovery headers, NDJSON streaming, bearer auth) |
| `api/self_description.rs` | GET / and /context.jsonld |
| `api/fragments.rs` | GET /fragments (TPF, cursor pagination, named graphs) |
| `api/sparql.rs` | GET /sparql (read-only execution plane) |
| `api/manifest.rs` | GET /manifest (agent manifest + SHACL shapes) |
| `api/events.rs` | GET /events (SSE change feed for live sessions) |
| `api/hub_endpoints.rs` | POST|GET /hub, GET /topics/{name} |
| `api/write.rs` | POST /admin/insert (token-gated write path) |
| `api/ui.rs` | GET /ui (embedded Graph Explorer page) |
| `api/mcp/mod.rs` | POST /mcp (full MCP server: initialize, ping, tools, resources, prompts) |
| `api/mcp/tools.rs` | The six graph tools (search_graph, sparql_query, get_manifest, get_topic, insert_triple, subscribe) |
| `api/mcp/resources.rs` | The manifest resource (manifest://semantic-web/current) |
| `api/mcp/prompts.rs` | Guided prompt templates (explore_graph, answer_from_graph) |
| `api/health_metrics.rs` | GET /health, GET /metrics (Prometheus text) |
| **hub/** | |
| `hub/mod.rs` | Hub core: subscriptions, fan-out orchestration, policies, metrics, SSE bus |
| `hub/publish.rs` | Publish fan-out: durable-log + queue + SSE, external-hub notify, open-hub fetch |
| `hub/verification.rs` | §5.3 intent verification, challenge, lease clamping, callback query preservation |
| `hub/delivery.rs` | Worker pool + content distribution with retries, 410, §7.1 HMAC signing |
| `hub/reload.rs` | Persistence reload: startup recovery + multi-replica refresh loop |
| `hub/crypto.rs` | AES-256-GCM secret encryption at rest (SEMWEB_SECRET_KEY) |
| `hub/rate_limit.rs` | Per-callback token bucket |
| `hub/metrics.rs` | Hub counters |
| **store/** | |
| `store/mod.rs` | SPARQL 1.1 Protocol client core (query/update/Graph Store Protocol/shapes) |
| `store/fragments.rs` | TPF lookup + pagination cursor |
| `store/cardinality.rs` | Maintained counters (classes, predicates, subjects, total) |
| `store/persistence.rs` | Hub-graph durability: subscriptions, delivery log, SHACL shapes |
| `bin/demo-subscriber.rs` | Spec-conformant demo WebSub subscriber (challenge echo, signature validation) |

## 7. Deployment topology

`docker compose up --build` runs two containers:

- **oxigraph** standalone (`/query`, `/update`, `/store`), data in a
  named volume. Any SPARQL 1.1 Protocol endpoint can substitute for it —
  the service's store contract is the protocol, not a product.
- **semantic-web** (this service), configured by environment:
  `SEMWEB_SPARQL_ENDPOINT`, `SEMWEB_SPARQL_UPDATE`,
  `SEMWEB_PUBLIC_URL` (absolute base for rel=self/rel=hub discovery
  URLs), `SEMWEB_SEED_PATH` (optional Turtle seed, loaded via the Graph
  Store Protocol with `?default` — load-bearing: a bare GSP POST to
  oxigraph 0.5.10 lands in a server-generated named graph),
  `SEMWEB_SHACL_PATH` (SHACL shapes for the manifest),
  `SEMWEB_TERM_ALIASES` (bare friendly names) and
  `SEMWEB_EXTRA_PREFIXES` for runtime prefix registration.

The WebSub demo profile adds **demo-subscriber** so the full
publish/subscribe loop runs inside the network with no host networking.
A `scale` profile adds a second replica (`SEMWEB_REPLICA_COUNT=2`,
`SEMWEB_REPLICA_INDEX=1`, host port 8485) sharing the same store —
two replicas forming one 2-shard cluster.

## 8. Roadmap

Everything designed for the service itself is implemented: durability
(subscriptions + delivery log), multi-replica sharding, encryption at
rest, auth hooks, observability, the agent manifest with SHACL shapes,
MCP delivery, SSE, named graphs and cursor pagination (§3–4). What
remains is deployment-level (TLS termination, KMS-backed key
management, per-tenant productization) — tracked in docs/scaling.md.