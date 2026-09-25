# Verified claims

Every claim on this site is checkable against a running stack. This page
is the evidence log: each claim, the exact command, and the observed
output (captured live; commands are reproducible — run them yourself).

Stack under test: `docker compose --profile demo up -d` (service :8484,
Oxigraph :7878), demo write token, demo term aliases
(`name`, `knows`, `employer`, `employs`, `founded`), `SEMWEB_OPEN_HUB=1`.

## 1. "Friendly names, zero compromise"

| Sub-claim | Evidence (live) |
|---|---|
| Bare friendly names (runtime-configured, nothing hardcoded) | `{"@id":"http://example.org/alice","name":"Alice"}` — via `SEMWEB_TERM_ALIASES` |
| Standard vocabularies compact automatically — a term never seen in the demo | inserted `http://schema.org/address` → rendered `"schema:address":"42 Main St"` with zero code/config changes |
| Unknown namespaces stay honest full URIs | `http://example.org/private-ns/rating` rendered verbatim |
| Typed values keep their types | `{"@value":"2001-04-03","@type":"xsd:date"}` |
| Round-trip through `/context.jsonld` | context serves prefixes (`foaf` → namespace) **and** term definitions (`name` → `{"@id": …}`); `xsd` is always present so typed literals round-trip too; **proven with pyld**: the aliased document expands back to `http://xmlns.com/foaf/0.1/name` / `http://schema.org/worksFor` |

## 2. "Self-description is live" (no build step)

`GET /` before an insert listed a new predicate as absent → one insert →
the **same request** returned it, and `GET /manifest` showed it with live
cardinality (`triples: 1`) and a changed `schemaFingerprint`. No rebuild,
no cache invalidation — sequential curls.

## 3. "The graph pushes" (WebSub)

| Step | Evidence |
|---|---|
| Subscribe once | `202`, async challenge verification passed |
| Insert | `event_id: 0b929d5f…` returned from the write |
| Delivery (no polling anywhere) | subscriber log: `content distribution received; signature verified (sha256)` |
| Full content + topic metadata | `content-type=application/x-ndjson`, `link=<…/topics/data>; rel="self", <…/hub>; rel="hub"` |
| Push is signed | HMAC-SHA256 over the raw body, verified by the demo subscriber |

## 4. Security

| Claim | Evidence |
|---|---|
| Write auth | `POST /admin/insert` without token → `401` |
| Secrets encrypted at rest | store graph contains `enc:v1:fcU7…` (AES-GCM ciphertext) — never plaintext |
| Rate limiting | 13 rapid subscribes, same callback → `202×10` then `429×3`, counter incremented |
| Duplicate insert is a no-op | re-POST of the same triple → `"duplicate": true`, zero notifications, counters unchanged (e2e check 13) |
| Reserved graphs are protected | write with `"graph": "http://semweb.dev/graph/hub/subscriptions"` → `400`; `?graph=` read → `400` |
| SPARQL read-only **by construction** | `INSERT DATA` through `/sparql` → `400` (it forwards to the store's query endpoint, which cannot execute updates) |

## 5. Agents (MCP)

| Claim | Evidence |
|---|---|
| Tools discovered dynamically | `tools/list` → `['search_graph','sparql_query','get_manifest','get_topic','insert_triple','subscribe']` |
| Execution via MCP | `tools/call sparql_query` returned live results (`COUNT = 29`) |
| Resources | `resources/read manifest://semantic-web/current` → the full manifest document |
| Prompts | `prompts/get explore_graph` → guided method text |
| Real agent | `examples/agent/agent.py` ran end-to-end with Together AI (Llama 3.3 70B): multi-tool runs, grounded answers, correctly `PREFIX`-ed SPARQL |

## 6. Scale

| Claim | Evidence |
|---|---|
| Cursor pagination | control line carries `after` (O(1) seek) + exact `count_estimate` from counters; pages advance without overlap (e2e) |
| Multi-replica sharding | 2 replicas, one store: insert via replica-0 → delivered its owned shard (`notified: 7` from `data_subscribers_notified`) and reported `routed_to_other_replica_total: 6` — the rest belongs to the other shard |
| Named-graph tenancy | `?graph=` reads and `"graph"` writes isolate tenants (e2e) |
| Durable delivery log | planted an unacked log entry with the service down → redelivered on restart (at-least-once) |
| Open-hub federation | third-party topic (`http://semantic-web:8000/manifest`) → hub fetched it at publish (§7), delivered `application/json`, `rel=self` = publisher's URL, `rel=hub` = ours, signed |

## 7. Interface

| Claim | Evidence |
|---|---|
| Graph Explorer | `GET /ui` → 200, self-building page over the same endpoints (17th e2e check) |
| SSE | `data: {"topic":"/topics/data","event_id":"…"}` observed within ~1s of the insert |
| Metrics | Prometheus text with hub lifecycle counters (deliveries, verifications, 410s, rate-limits, shard routing) |

## How to re-verify

- One command: `./scripts/e2e.sh` (18 checks, exit code = verdict, runs in CI)
- Unit tests: `cargo test` (26)
- The commands above are plain curl against a running stack.

If any output ever contradicts a claim here, that's a bug — open an
issue.