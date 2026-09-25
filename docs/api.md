# API reference

Every endpoint of the service, with methods, parameters, auth rules and
real request/response examples. All examples run against the demo stack
(`docker compose up`, service at `http://localhost:8484`, seed graph
loaded). Values that vary per run (event ids, cursors, fingerprints,
latencies) are shown truncated.

Plain HTTP only: REST verbs, query params, form-encoded WebSub requests,
NDJSON streaming, JSON-LD, SSE. No SDK, no custom media types.

**Authentication at a glance**

| Endpoint(s) | Auth |
|---|---|
| All reads (`/`, `/context.jsonld`, `/fragments`, `/sparql`, `/manifest`, `/events`, `/health`, `/metrics`, `/ui`, `GET /hub`, `GET /topics/*`) | open |
| `POST /admin/insert` | `Authorization: Bearer <SEMWEB_WRITE_TOKEN>` when that env var is set (demo compose sets `demo-write-token`) |
| `POST /hub` | `Authorization: Bearer <SEMWEB_HUB_TOKEN>` when that env var is set |
| `POST /mcp` | open; MCP `insert_triple` validates the write token inside the tool call |

## Endpoints

| Method | Path | Purpose |
|---|---|---|
| GET | `/` | Live self-description (classes, predicates, controls) + discovery Link headers |
| GET | `/context.jsonld` | JSON-LD `@context`, generated live from namespaces in use |
| GET | `/fragments` | Triple Pattern Fragment, streamed as NDJSON-LD, cursor or offset pagination |
| GET | `/sparql` | Read-only SPARQL 1.1 Protocol passthrough (the execution plane); POST supported for long queries |
| GET | `/manifest` | Agent manifest: schema fingerprint, cardinalities, descriptions, shapes, examples |
| GET | `/ui` | Graph Explorer (single-page browser UI over the same endpoints) |
| GET | `/events` | SSE change feed for live sessions (complement to WebSub) |
| POST | `/mcp` | MCP server: tools, resources and prompts over JSON-RPC 2.0 |
| GET | `/health` | Liveness + store reachability |
| GET | `/metrics` | Prometheus text exposition of hub/service counters |
| POST | `/hub` | WebSub subscribe / unsubscribe / publish |
| GET | `/hub` | Topic list + active subscription counts (convenience, not in the spec) |
| GET | `/topics/{name}` | Topic content + discovery Link headers |
| POST | `/admin/insert` | Write path (token-gated): one triple, counter updates, topic publishes |

---

## `GET /`

Live self-description, computed from current store state on every call.
No cache, no build step — always exactly as current as the data.
Carries WebSub discovery Link headers (`rel=self`, `rel=hub`).

```sh
curl -i http://localhost:8484/
```

```json
{
  "@context": "/context.jsonld",
  "generatedFrom": "live store state (not a cached build)",
  "storeMode": "sparql-rust",
  "classes": ["foaf:Organization", "foaf:Person"],
  "predicates": ["employs", "founded", "employer", "rdf:type",
                 "rdfs:comment", "skos:definition", "knows", "name"],
  "controls": {
    "fragments": "/fragments{?subject,predicate,object,after,limit,offset,graph}",
    "sparql": "/sparql{?query}",
    "manifest": "/manifest",
    "mcp": "/mcp",
    "events": "/events{?topic}",
    "hub": "/hub",
    "topics": ["/topics/data", "/topics/schema"]
  }
}
```

Class/predicate names are compacted through the namespace prefix table
(standards vocabularies → `prefix:local` CURIEs; configured term aliases
→ bare names; unknown namespaces stay full URIs). With the demo stack's
`SEMWEB_TERM_ALIASES`, `schema:worksFor` appears as `employer` etc. —
adding a term from a known vocabulary requires no code change; a new
namespace gets a prefix via `SEMWEB_EXTRA_PREFIXES` at runtime.

## `GET /context.jsonld`

The JSON-LD `@context`, generated live from the namespaces currently in
use in the store, plus term aliases as JSON-LD term definitions:

```json
{
  "@context": {
    "type": "@type",
    "xsd": "http://www.w3.org/2001/XMLSchema#",
    "rdf": "http://www.w3.org/1999/02/22-rdf-syntax-ns#",
    "rdfs": "http://www.w3.org/2000/01/rdf-schema#",
    "schema": "http://schema.org/",
    "skos": "http://www.w3.org/2004/02/skos/core#",
    "foaf": "http://xmlns.com/foaf/0.1/",
    "name": {"@id": "http://xmlns.com/foaf/0.1/name"},
    "knows": {"@id": "http://xmlns.com/foaf/0.1/knows"},
    "employer": {"@id": "http://schema.org/worksFor"},
    "employs": {"@id": "http://schema.org/employee"},
    "founded": {"@id": "http://schema.org/foundingDate"}
  }
}
```

`xsd` is always present (datatypes in the data render as `xsd:date` etc.,
so a JSON-LD processor can round-trip them). Prefixes/aliases come from
the standard table plus `SEMWEB_EXTRA_PREFIXES` / `SEMWEB_TERM_ALIASES`.
Only registered prefixes are emitted; namespaces without a registered
prefix appear as full URIs in the data — the context need not (and
cannot honestly) name them.

## `GET /fragments`

Triple Pattern Fragment lookup (spec: `{?s ?p ?o}` pattern semantics;
omitted positions are wildcards).

| Param | Meaning |
|---|---|
| `subject` | URI to match on subject |
| `predicate` | URI to match on predicate |
| `object` | URI (auto-detected via `http(s)://` prefix) or plain literal |
| `graph` | Optional named graph (multi-tenancy: tenants map to graphs); absent = default graph. Service-reserved graph URIs (`http://semweb.dev/graph/…`) are rejected with `400` |
| `after` | Opaque cursor from a previous page's control line — O(1) seek pagination |
| `limit` | Page size, default 100, max 1000 |
| `offset` | Legacy skip count (cursor pagination is preferred; results are deterministic either way — ordered by the string forms of s, p, o) |

```sh
curl "http://localhost:8484/fragments?predicate=http://schema.org/worksFor"
curl "http://localhost:8484/fragments?subject=http://example.org/acme"
curl "http://localhost:8484/fragments?predicate=http://xmlns.com/foaf/0.1/name&object=Alice"
```

Response: `application/x-ndjson`, streamed, one compacted JSON-LD
statement per line:

```json
{"@id": "http://example.org/alice", "employer": {"@id": "http://example.org/acme"}}
{"@id": "http://example.org/bob", "employer": {"@id": "http://example.org/acme"}}
```

- `rdf:type` compacts to `@type`:
  `{"@id": "http://example.org/alice", "@type": "foaf:Person"}`
- Typed literals keep their datatype; language tags survive:
  `{"@id": "http://example.org/acme", "founded": {"@value": "2001-04-03", "@type": "xsd:date"}}`

When the page is truncated, the final line is the hypermedia control
carrying both continuation mechanisms:

```json
{"@control": "metadata", "count_estimate": 20,
 "after": "WyJodHRwOi8vZXhhbXBsZS5vcmcvYWNtZSIs...",
 "next": "/fragments?subject=&predicate=&object=&limit=2&offset=2"}
```

Follow `after` for cursor pagination (O(1) seek, production path);
`next` remains for offset-based clients. `count_estimate` comes from the
maintained cardinality counters when they cover the pattern (whole
graph, single-predicate, or single-subject patterns), else from an exact
server COUNT; with `after` or `graph` present it is always an exact
COUNT.

## `GET /sparql?query=...` and `POST /sparql`

Read-only SPARQL 1.1 Protocol passthrough — the execution plane, one
hop from the discovery plane. Queries are forwarded to the store's
`/query` endpoint, which only executes SPARQL Query forms
(SELECT/ASK/DESCRIBE/CONSTRUCT); updates live on a separate URL this
handler never touches, so read-only is guaranteed by construction.

```sh
# GET (short queries)
curl "http://localhost:8484/sparql?query=SELECT%20%3Fs%20%3Fname%20WHERE%20%7B%20%3Fs%20%3Chttp%3A%2F%2Fxmlns.com%2Ffoaf%2F0.1%2Fname%3E%20%3Fname%20%7D"

# POST, form-encoded (long queries)
curl -X POST http://localhost:8484/sparql \
  -H "Content-Type: application/x-www-form-urlencoded" \
  --data-urlencode "query=SELECT ?s ?name WHERE { ?s <http://xmlns.com/foaf/0.1/name> ?name }"

# POST, raw query body (SPARQL 1.1 Protocol §2.2)
curl -X POST http://localhost:8484/sparql \
  -H "Content-Type: application/sparql-query" \
  -d "ASK WHERE { ?s ?p ?o }"
```

Response: the store's SPARQL JSON results, content-type preserved.
`Content-Type: application/sparql-update` is rejected with `400` —
read-only by construction (updates live on a separate store URL this
handler never touches). Missing/empty `query` → `400`.

## `GET /manifest`

The agent manifest — the planning surface for agents and developers,
regenerated live on every request:

```json
{
  "@context": "/context.jsonld",
  "kind": "agent-manifest",
  "generatedFrom": "live store state (not a cached build)",
  "schemaFingerprint": "sha256:9beb...",
  "prefixes": [["foaf", "http://xmlns.com/foaf/0.1/"], ["schema", "http://schema.org/"]],
  "classes": [
    {
      "uri": "http://xmlns.com/foaf/0.1/Person",
      "compact": "foaf:Person",
      "instances": 3,
      "description": "A person as described by the FOAF vocabulary.",
      "shapes": [
        {"path": "http://xmlns.com/foaf/0.1/name", "minCount": 1,
         "maxCount": 1, "datatype": "http://www.w3.org/2001/XMLSchema#string"},
        {"path": "http://xmlns.com/foaf/0.1/knows", "maxCount": 5}
      ],
      "exampleQuery": "SELECT ?s WHERE { ?s a <http://xmlns.com/foaf/0.1/Person> } LIMIT 10"
    }
  ],
  "predicates": [
    {"uri": "http://xmlns.com/foaf/0.1/name", "compact": "name", "triples": 5,
     "description": null}
  ],
  "topics": ["/topics/data", "/topics/schema"],
  "controls": {"fragments": "...", "sparql": "/sparql{?query}", "hub": "/hub"}
}
```

Field by field:

- `schemaFingerprint` — sha256 over the sorted class+predicate URI set;
  also carried on `/topics/schema` content, so consumers detect missed
  schema-change events and diff safely.
- `prefixes` — the PREFIX declarations agents should copy to the top of
  SPARQL queries, derived from the namespaces currently in use.
- `instances` / `triples` — live GROUP BY counts, never counter drift.
- `description` — from `rdfs:comment` / `skos:definition` in the graph
  (null when absent). The grounding text agents plan against.
- `shapes` — SHACL property shapes per class (path, minCount, maxCount,
  datatype) read live from the shapes graph when `SEMWEB_SHACL_PATH` is
  configured. Declared constraints agents can plan against.
- `exampleQuery` — executable at `/sparql`.

## `GET /ui`

The Semantic Graph Explorer: one embedded HTML page (no build tooling,
no extra backend) that builds itself from `/manifest`, searches via
`/fragments`, previews topics, subscribes to `/events` via EventSource,
and shows `/health` + `/metrics`. A lens on the graph, not an admin
panel.

## `GET /events` (SSE)

Live change feed for open agent sessions — the complementary channel to
WebSub (durable subscribers can be offline; SSE sessions are live
streams). Server-Sent Events over a long-lived GET; `?topic=/topics/data`
filters (an unknown topic → `400`).

Each event carries the notification header only; clients refetch
`GET /topics/{name}` for content, keeping the stream small regardless
of topic size. Keepalive comments flow every 15s; a `Lagged` event tells
the client it missed events and should resync.

```sh
curl -N "http://localhost:8484/events?topic=/topics/data"
```

```text
data: {"topic":"/topics/data","event_id":"9c7c27ac8ef141dc"}

: keepalive

data: {"error":"lagged","missed":"3"}
```

## `POST /mcp`

A full [MCP](https://modelcontextprotocol.io) server (Model Context
Protocol): JSON-RPC 2.0 over POST, single-JSON responses. Protocol
version `2025-06-18`. Three capability groups:

**Tools** (`tools/list`, `tools/call`):

| Tool | Input | What it does |
|---|---|---|
| `search_graph` | `subject`, `predicate`, `object`, `graph`, `limit` (default 20, results capped at 200 lines) | TPF fragment lookup; omitted positions are wildcards. Returns compacted JSON-LD lines + `[count_estimate, has_more]` |
| `sparql_query` | `query` (required) | Read-only SPARQL; appends a "0 results → check URI casing" hint on empty results |
| `get_manifest` | — | The live agent manifest (same document as `GET /manifest`) |
| `get_topic` | `topic` (`/topics/data` or `/topics/schema`) | Full topic content |
| `insert_triple` | `subject`, `predicate`, `object`, `graph`, `token` | Write path; honours `SEMWEB_WRITE_TOKEN` (pass it as the `token` argument when the deployment gates writes). Shares the pipeline with `POST /admin/insert`: duplicate inserts are reported as "no change" (no counter bump, no push) and a brand-new class/predicate fires `/topics/schema` |
| `subscribe` | `topic`, `callback`, `secret`, `lease_seconds` | WebSub subscription on behalf of a callback URL (the callback must implement the subscriber contract) |

Tool text is capped at 8,000 characters (`_truncated: true` marks a cut)
so an LLM caller never receives unbounded payloads. `sparql_query`
teaches instead of returning bare empty results — an empty result is
usually a mistyped (case-sensitive) URI, not absence of data.

**Resources** (`resources/list`, `resources/read`): the live agent
manifest at `manifest://semantic-web/current`.

**Prompts** (`prompts/list`, `prompts/get`): `explore_graph`
(optional `focus`) and `answer_from_graph` (required `question`) —
guided method templates.

```sh
curl -X POST http://localhost:8484/mcp -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}'
curl -X POST http://localhost:8484/mcp -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'
curl -X POST http://localhost:8484/mcp -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call",
       "params":{"name":"search_graph",
                 "arguments":{"predicate":"http://schema.org/worksFor"}}}'
curl -X POST http://localhost:8484/mcp -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":4,"method":"resources/read",
       "params":{"uri":"manifest://semantic-web/current"}}'
```

Also handled: `ping` → `{}`, `prompts/get`, and notifications
(`notifications/initialized`, `notifications/cancelled`) → `202` with no
body.

## `GET /health`

Liveness + store readiness (the docker healthcheck target):

```json
{"status": "ok", "store": "reachable"}
```

`degraded` / `unreachable` when the store does not answer an `ASK {}`.

## `GET /metrics`

Prometheus text exposition. Counters:

```text
semweb_verifications_total{result="ok"|"failed"}   # §5.3 intent verifications
semweb_deliveries_total{result="success"|"exhausted"}
semweb_delivery_retries_total
semweb_terminations_410_total
semweb_denied_total
semweb_queue_dropped_total
semweb_rate_limited_total
semweb_redelivered_on_restart_total
semweb_routed_to_other_replica_total
semweb_subscriptions_active                        # gauge
semweb_fragments_requests_total
semweb_inserts_total
```

## `POST /hub`

WebSub subscription request — form-encoded per the WebSub spec §5.1.
Unknown additional parameters are ignored, as the spec requires.

| Param | Meaning |
|---|---|
| `hub.mode` | `subscribe`, `unsubscribe`, or `publish` |
| `hub.topic` | Topic URL; MUST be the rel=self URL from discovery (the bare `/topics/data` path also works) |
| `hub.callback` | Subscriber callback URL (query string is preserved, §5.1.1) |
| `hub.lease_seconds` | Optional requested lease; hub clamps to [60s, 10 days], default 1 day (§5.3: expirations are mandatory, never perpetual) |
| `hub.secret` | Optional HMAC secret, MUST be < 200 bytes (§5.1) |

Subscription requests **persist to the store in a dedicated named
graph** (a restart reloads them; expired leases are dropped on load per
§5.3), and secrets are **AES-256-GCM encrypted at rest** when
`SEMWEB_SECRET_KEY` is configured.

Hub policies (§5.1 allows hubs to set their own): `SEMWEB_HUB_TOKEN`
requires a bearer token on POST /hub; `SEMWEB_REQUIRE_HTTPS_CALLBACKS`
rejects http:// callbacks that registered a secret; `SEMWEB_CALLBACK_ALLOWLIST`
restricts callback hosts (suffix match) — and, when the open-hub policy
is on, third-party topic URLs as well. Both subscribe and unsubscribe
requests share one rate-limit bucket per callback (burst 10, refill
10/min; exceeded → `429`).

Deliveries are **at-least-once**: every delivery is logged in the
store before enqueue and acked on completion, so a crash mid-flight
redelivers on startup (a duplicate notification is possible if a
delivery is retried across a scan window; subscribers should treat
duplicates as idempotent).

```sh
curl -X POST http://localhost:8484/hub \
  -d "hub.mode=subscribe" \
  -d "hub.topic=http://localhost:8484/topics/data" \
  -d "hub.callback=http://localhost:9000/callback" \
  -d "hub.secret=my-secret"
```

Responses:

- `202 Accepted` — received; verification starts asynchronously and the
  response MUST NOT depend on its outcome (§5.1.2).
- `4xx` with a plain-text body — validation errors (unknown topic,
  missing parameters, secret ≥ 200 bytes, policy rejection).

### Verification handshake (§5.3)

The hub GETs the callback with `hub.mode`, `hub.topic`, `hub.challenge`
and (for subscribe) `hub.lease_seconds` appended to the callback's
existing query string. The callback must respond 2xx with the challenge
as the body. Unreachable callbacks and transient 5xx answers are
retried (3 attempts, 2s/5s apart); a wrong challenge echo, a 404, or
another 3xx/4xx fails immediately. If verification finally fails on a
subscribe, the hub sends an explicit §5.2 denied notification
(`hub.mode=denied` GET) so the subscriber is not left waiting on a
subscription that will never activate. The example `demo-subscriber`
binary shows the correct subscriber behavior, including the §8.2
safe-media-type + nosniff mitigations. Unsubscribe works identically
with `hub.mode=unsubscribe` (no lease semantics).

### Topics

**`/topics/data`** — fired on every instance-data change.
**`/topics/schema`** — fired when an insert introduces a class or
predicate not previously in use (the ontology surface changed).

Content distribution (§7) POSTs the **full current contents of the
topic** — the topic's Content-Type (`application/x-ndjson` for data,
`application/json` for schema), Link headers
`rel="self"` (canonical topic URL) + `rel="hub"`, and — when the
subscription registered a `hub.secret` — an `X-Hub-Signature`
(`sha256=...`, HMAC over the body).

Delivery semantics (§7): subscriber must answer 2xx (bodies are
ignored; the ack means "received", not "processed"). `410 Gone`
terminates the subscription. Any other failure is retried on a
1s/5s/15s schedule; after the retry budget the hub gives up on that
notification but keeps the subscription active until lease end, and
the next published update retries delivery again.

### Publisher notification (§6)

The spec leaves the publisher→hub mechanism open; the conventional
`hub.mode=publish` + `hub.url` form is supported:

```sh
curl -X POST http://localhost:8484/hub \
  -d "hub.mode=publish" \
  -d "hub.url=http://localhost:8484/topics/data"
```

The hub rebuilds the topic content at publish time and fans out.
Unknown `hub.url`s are rejected with 400 — except third-party topics
when `SEMWEB_OPEN_HUB=1` (the [federation](federation.md) feature).

## `GET /hub`

Convenience for humans/agents poking at the API before subscribing
(not part of the spec): the topic list and active subscription counts.

```sh
curl http://localhost:8484/hub
```

```json
{"topics": ["/topics/data", "/topics/schema"],
 "subscriptions": {"/topics/data": 1, "/topics/schema": 0}}
```

## `GET /topics/{name}`

Topic resources — the WebSub discovery surface (§4). Returns the full
topic content with a single `rel=self` Link header (the canonical topic
URL) and a `rel=hub` link, in a Content-Type that content distribution
will preserve (§4.1: one representation per rel=self, so no
content-negotiation ambiguity):

```sh
curl -i http://localhost:8484/topics/data
```

```
HTTP/1.1 200 OK
content-type: application/x-ndjson
link: <http://localhost:8484/topics/data>; rel="self", <http://localhost:8484/hub>; rel="hub"

{"@id":"http://example.org/alice","@type":"foaf:Person"}
{"@id":"http://example.org/alice","name":"Alice"}
...
```

Only canonical topics are served here; third-party topics (open-hub
policy) live at the publisher's own URL and are fetched at publish
time. `/topics/schema` content carries the same `schemaFingerprint` as
the manifest.

## `POST /admin/insert`

Writes one triple (SPARQL UPDATE), updates the cardinality counters,
and publishes to subscribed topics. When `SEMWEB_WRITE_TOKEN` is set
(the compose stack sets it), the endpoint requires
`Authorization: Bearer <token>`; read endpoints stay open. Production
deployments bring their own write path (SPARQL UPDATE endpoint, ETL)
that publishes the same way — see scaling.md §2.

```sh
curl -X POST http://localhost:8484/admin/insert \
  -H "Authorization: Bearer demo-write-token" \
  -H "Content-Type: application/json" \
  -d '{"subject":"http://example.org/carol","predicate":"http://xmlns.com/foaf/0.1/knows","object":"http://example.org/alice"}'
```

Body: `subject`, `predicate`, `object` (required; `object` becomes a
URI when it starts with `http(s)://`, else a plain literal) and an
optional `"graph"` for tenant isolation (absent = default graph;
service-reserved graph URIs `http://semweb.dev/graph/…` are rejected
with `400`).

```json
{
  "event_id": "440b3df8889a033b",
  "inserted": {"@id": "http://example.org/carol", "knows": {"@id": "http://example.org/alice"}},
  "duplicate": false,
  "data_subscribers_notified": 1,
  "schema_changed": false,
  "schema_subscribers_notified": 0
}
```

- `event_id` — one id for this mutation; ties write → publish → fan-out
  together across logs, metrics and (via the Link metadata) deliveries.
- `duplicate` — `true` when the exact triple already existed: no store
  change, no counter bump, no topic publish, no notifications (a
  re-POSTed triple is a no-op end to end).
- `schema_changed` — `true` when the insert introduced a class or
  predicate not previously in use (detected O(1) from the counters, not
  by re-diffing the whole schema) — `/topics/schema` fires in addition
  to `/topics/data`.
- `*_subscribers_notified` — deliveries enqueued to the bounded queue
  for active (unexpired) subscribers; workers perform the actual POSTs
  asynchronously with the spec's retry contract.

## Error responses

Errors are plain text (per §5.1.2, "to assist the client developer in
understanding the error"):

- `400` — missing/invalid hub parameters, unknown publish target,
  missing `?query=` on `/sparql`, unknown `?topic=` filter on `/events`
- `401` — write path (or `/hub`, when SEMWEB_HUB_TOKEN is set) called without the configured bearer token
- `404` — unknown topic or `hub.topic` not resolvable to a topic
- `429` — subscription rate limit exceeded for this callback
- `500` — store error (also logged with `tracing`)
- `502` — store unreachable (on `/sparql`)