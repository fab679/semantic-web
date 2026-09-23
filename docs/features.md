# Complete feature list

Every capability of the service, grouped by who uses it. Each entry is
covered in depth on its guide page; this page is the full map.

## Reads (everyone)

| Feature | What it does | Endpoint |
|---|---|---|
| Live self-description | What classes/predicates exist *right now* — computed per request, never stale | `GET /` |
| Live `@context` | The JSON-LD context generated from namespaces in use | `GET /context.jsonld` |
| Triple Pattern Fragments | Pattern lookup (any of subject/predicate/object as wildcards), streamed NDJSON | `GET /fragments` |
| Cursor pagination | O(1) seek via `after=` (plus legacy `offset`) | `GET /fragments` |
| Named graphs (tenants) | Isolate tenants into graphs | `?graph=` on reads |
| SPARQL execution | Read-only SELECT/ASK/DESCRIBE/CONSTRUCT passthrough | `GET /sparql` |
| Typed-value fidelity | Dates, numbers, language tags survive the wire (`xsd:date`, `@language`) | everywhere |
| Prefix compaction | Standard vocabularies → `foaf:name`-style CURIEs; runtime extras via env | all outputs |

## Discovery & grounding (agents, developers)

| Feature | What it does | Endpoint |
|---|---|---|
| Agent manifest | Classes/predicates + live cardinalities + human descriptions + SHACL shapes + example SPARQL + schema fingerprint | `GET /manifest` |
| Schema fingerprint | sha256 of the ontology surface — detect that your understanding went stale | manifest, `/topics/schema` |
| SHACL shapes | Property constraints (minCount/maxCount/datatype) per class, loaded from a shapes file | `SEMWEB_SHACL_PATH` |
| Example queries | One executable SPARQL per class | manifest |

## Real-time push (services, agents)

| Feature | What it does | Where |
|---|---|---|
| WebSub hub | W3C Recommendation-compliant: subscribe/unsubscribe/publish, intent verification, leases, retries, 410 termination | `POST /hub` |
| Two topics | `/topics/data` (instance data) and `/topics/schema` (ontology surface changed) | `GET /topics/{name}` |
| Signed delivery | HMAC-SHA256 `X-Hub-Signature` over the full topic content | spec §7.1 |
| Lease enforcement | expiring subscriptions (60s–10 days, default 1 day), renewal by re-request | `hub.lease_seconds` |
| Durable delivery log | logged before enqueue, acked when finished; crash mid-flight → redelivery on restart (at-least-once) | store graph |
| Delivery claims | in-flight deliveries are claimed so no other worker/replica duplicates them | internal |
| Retry schedule | 1s / 5s / 15s, then stop for that notification; subscription survives until lease end (spec §7) | internal |
| Denied notifications | `hub.mode=denied` callback when a topic becomes unavailable (§5.2) | internal |
| Discovery headers | every topic advertises `rel=self` + `rel=hub` (§4) | `GET /topics/*` |
| Publisher notify | `hub.mode=publish&hub.url=...` (spec §6, mechanism unspecified — this is the common convention) | `POST /hub` |
| SSE change feed | live sessions: header-only events, keepalives, `Lagged` resync signal | `GET /events?topic=` |
| Two topics, one bus | WebSub (durable, offline-capable) and SSE (live sessions) from the same publish | — |

## Security

| Feature | What it does | Config |
|---|---|---|
| Write-path auth | bearer token on mutating endpoints | `SEMWEB_WRITE_TOKEN` |
| Hub auth (optional) | gate who may subscribe/unsubscribe/publish | `SEMWEB_HUB_TOKEN` |
| Secret encryption at rest | AES-256-GCM on stored subscriber secrets; legacy plaintext values keep working; tampering rejected | `SEMWEB_SECRET_KEY` |
| HMAC-signed pushes | verify deliveries with your `hub.secret` (sha256) | per subscription |
| Rate limiting | per-callback token bucket on subscription requests (429) | built in |
| HTTPS-callback policy | reject `http://` callbacks that register secrets | `SEMWEB_REQUIRE_HTTPS_CALLBACKS` |
| Callback allowlist | restrict callback hosts (suffix match) | `SEMWEB_CALLBACK_ALLOWLIST` |
| Challenge hardening | spec charset, length limit, no-binary (§8.2) | built in |

## Scale-out

| Feature | What it does | Config |
|---|---|---|
| Bounded delivery queue + worker pool | explicit backpressure; publish latency decoupled from delivery | `SEMWEB_QUEUE_CAPACITY`, `SEMWEB_DELIVERY_WORKERS` |
| Multi-replica sharding | consistent-hash shard per (topic, callback); replicas share the store, deliver only their shard | `SEMWEB_REPLICA_COUNT`, `SEMWEB_REPLICA_INDEX` |
| Subscription sync | replicas poll the shared store for new subscriptions and renewals | `SEMWEB_SUBS_REFRESH_SECS` |
| Read replicas | query and update URLs are separate — point reads at a LB | `SEMWEB_SPARQL_ENDPOINT` |
| Named-graph tenancy | tenants map to graphs; isolated reads/writes | `?graph=` / `"graph"` |
| Counter refresh | self-healing cardinality counters for out-of-band writers | `SEMWEB_COUNTER_REFRESH_SECS` |

## MCP (LLM agents)

| Capability | Tools / resources / prompts |
|---|---|
| **tools** | `search_graph`, `sparql_query`, `get_manifest`, `get_topic`, `insert_triple`, `subscribe` — JSON-Schema inputs, bounded outputs |
| **resources** | the live agent manifest at `manifest://semantic-web/current` |
| **prompts** | `explore_graph`, `answer_from_graph` — guided method templates |
| transport | JSON-RPC 2.0 over POST (streamable-HTTP style, single-JSON responses) |

See [For agents](for-agents.md) and the [agent tutorial](agent-tutorial.md).

## Operations

| Feature | What it does | Where |
|---|---|---|
| Health | liveness + store readiness | `GET /health` |
| Metrics | Prometheus text: verifications, deliveries, retries, 410s, denials, queue drops, rate limits, redeliveries, shard routing, subscriptions, requests | `GET /metrics` |
| Traceability | one `event_id` per mutation: write → publish → delivery log → acks | responses + logs |
| Structured logs | `tracing` with per-topic/per-callback lifecycle lines | `RUST_LOG` |
| e2e battery | full-stack verification script (CI-gated) | `scripts/e2e.sh` |

Full details per mechanism: [For engineering teams](engineering.md).