# API Reference

All endpoints are plain HTTP over the standard interfaces the
architecture is built on (REST verbs, form-encoded WebSub requests,
NDJSON streaming, JSON-LD). No authentication yet — see scaling.md for
the hardening roadmap.

Base URL: `http://localhost:8484` (docker compose) unless
`SEMWEB_PUBLIC_URL` changes it.

## Endpoints

| Method | Path | Purpose |
|---|---|---|
| GET | `/` | Live self-description (classes, predicates, controls) + discovery Link headers |
| GET | `/context.jsonld` | JSON-LD `@context`, generated live from namespaces in use |
| GET | `/fragments` | Triple Pattern Fragment, streamed as NDJSON-LD, cursor or offset pagination |
| GET | `/sparql` | Read-only SPARQL 1.1 Protocol passthrough (the execution plane) |
| GET | `/manifest` | Agent manifest: schema fingerprint, cardinalities, descriptions, example SPARQL |
| GET | `/events` | SSE change feed for live agent sessions (complement to WebSub) |
| POST | `/mcp` | Minimal MCP resource server (initialize, resources/list, resources/read) |
| GET | `/health` | Liveness + store reachability |
| GET | `/metrics` | Prometheus text exposition of hub/service counters |
| POST | `/hub` | WebSub subscribe / unsubscribe / publish (docs/WebSub.md §5, §6) |
| GET | `/hub` | Topic list + active subscription counts (convenience, not in the spec) |
| GET | `/topics/{data\|schema}` | Topic content + discovery Link headers (§4) |
| POST | `/admin/insert` | Write path (optionally token-gated): one triple, counter updates, topic publishes |

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
  "predicates": ["schema:employee", "schema:foundingDate", "schema:worksFor",
                 "rdf:type", "foaf:knows", "foaf:name"],
  "controls": {
    "fragments": "/fragments{?subject,predicate,object}",
    "hub": "/hub",
    "topics": ["/topics/data", "/topics/schema"]
  }
}
```

Class/predicate URIs are compacted through the namespace prefix table
(standards vocabularies → `prefix:local` CURIEs; unknown namespaces
stay full URIs). Adding a term from a known vocabulary requires no code
change; a new namespace gets a prefix via `SEMWEB_EXTRA_PREFIXES`/
`SEMWEB_EXTRA_PREFIXES` at runtime.

---

## `GET /context.jsonld`

The JSON-LD `@context`, generated live from the namespaces currently
in use in the store:

```json
{
  "@context": {
    "type": "@type",
    "schema": "http://schema.org/",
    "rdf": "http://www.w3.org/1999/02/22-rdf-syntax-ns#",
    "foaf": "http://xmlns.com/foaf/0.1/"
  }
}
```

Only registered prefixes are emitted. Namespaces without a registered
prefix appear as full URIs in the data — the context need not (and
cannot honestly) name them.

---

## `GET /fragments`

Triple Pattern Fragment lookup (spec: `{?s ?p ?o}` pattern semantics;
omitted positions are wildcards).

| Param | Meaning |
|---|---|
| `subject` | URI to match on subject |
| `predicate` | URI to match on predicate |
| `object` | URI (auto-detected via `http(s)://` prefix) or plain literal |
| `graph` | Optional named graph (multi-tenancy: tenants map to graphs); absent = default graph |
| `after` | Opaque cursor from a previous page's control line — production pagination, O(1) seek |
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
{"@id": "http://example.org/alice", "schema:worksFor": {"@id": "http://example.org/acme"}}
{"@id": "http://example.org/bob", "schema:worksFor": {"@id": "http://example.org/acme"}}
```

- `rdf:type` compacts to `@type`:
  `{"@id": "http://example.org/alice", "@type": "foaf:Person"}`
- Typed literals keep their datatype; language tags survive:
  `{"@id": "http://example.org/acme", "schema:foundingDate": {"@value": "2001-04-03", "@type": "xsd:date"}}`
with, when the page is truncated, both continuation mechanisms:

```json
{"@control": "metadata", "count_estimate": 20,
 "after": "WyJodHRwOi8vZXhhbXBsZS5vcmcvYWNtZSIs...",
 "next": "/fragments?subject=&predicate=&object=&limit=2&offset=2"}
```

Follow `after` for cursor pagination (O(1) seek, production path);
`next` remains for offset-based clients. `count_estimate` comes from
the maintained cardinality counters when they cover the pattern, else
an exact server COUNT.

---

## `GET /sparql?query=...`

Read-only SPARQL 1.1 Protocol passthrough — the execution plane, one
hop from the discovery plane. The query is forwarded to the store's
`/query` endpoint, which only executes SPARQL Query forms
(SELECT/ASK/DESCRIBE/CONSTRUCT); updates live on a separate URL this
handler never touches, so read-only is guaranteed by construction.

```sh
curl "http://localhost:8484/sparql?query=SELECT%20%3Fs%20%3Fname%20WHERE%20%7B%20%3Fs%20%3Chttp%3A%2F%2Fxmlns.com%2Ffoaf%2F0.1%2Fname%3E%20%3Fname%20%7D"
```

Response: the store's SPARQL JSON results, content-type preserved.

---

## `GET /manifest`

The agent manifest — the planning surface for agents and developers,
regenerated live on every request:

```json
{
  "@context": "/context.jsonld",
  "kind": "agent-manifest",
  "schemaFingerprint": "sha256:9beb...",
  "prefixes": [["foaf", "http://xmlns.com/foaf/0.1/"], ["schema", "http://schema.org/"]],
  "classes": [
    {
      "uri": "http://xmlns.com/foaf/0.1/Person",
      "compact": "foaf:Person",
      "instances": 3,
      "description": "A person as described by the FOAF vocabulary.",
      "exampleQuery": "SELECT ?s WHERE { ?s a <http://xmlns.com/foaf/0.1/Person> } LIMIT 10"
    }
  ],
  "predicates": [
    {"uri": "http://xmlns.com/foaf/0.1/name", "compact": "foaf:name",
     "triples": 6, "description": null}
  ],
  "topics": ["/topics/data", "/topics/schema"],
  "controls": {"fragments": "...", "sparql": "/sparql{?query}", "hub": "/hub"}
}
```

- `schemaFingerprint` — sha256 over the sorted class+predicate URI set;
  also carried on `/topics/schema` content, so consumers detect missed
  schema-change events and diff safely.
- `instances` / `triples` — live GROUP BY counts, never counter drift.
- `description` — from `rdfs:comment` / `skos:definition` in the graph
  (null when absent). The grounding text agents plan against.
- `shapes` — SHACL property shapes per class (path, minCount, maxCount,
  datatype) read live from the shapes graph when `SEMWEB_SHACL_PATH` is
  configured. Declared constraints agents can plan against.
- `exampleQuery` — executable at `/sparql`.

---

## `GET /events` (SSE)

Live change feed for open agent sessions — the complementary channel to
WebSub (durable subscribers can be offline; SSE sessions are live
streams). Server-Sent Events over a long-lived GET; `?topic=/topics/data`
filters.

Each event carries the notification header only; clients refetch
`GET /topics/{name}` for content, keeping the stream small regardless
of topic size. Keepalive comments flow every 15s; a `Lagged` event
tells the client it missed events and should resync.

```sh
curl -N "http://localhost:8484/events?topic=/topics/data"
```

```text
data: {"topic":"/topics/data","event_id":"9c7c27ac8ef141dc"}

```

---

## `POST /mcp`

A minimal MCP (Model Context Protocol) resource server so MCP-aware
agents discover and read the manifest exactly the way they list tools:
JSON-RPC 2.0 over POST, single-JSON responses.

```sh
curl -X POST http://localhost:8484/mcp -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}'
curl -X POST http://localhost:8484/mcp -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"resources/list","params":{}}'
curl -X POST http://localhost:8484/mcp -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":3,"method":"resources/read","params":{"uri":"manifest://semantic-web/current"}}'
```

`resources/read` returns the live agent manifest (the same document as
`GET /manifest`) as an MCP content item. Resources-only scope: no
prompts/tools, no SSE transport.

---

## `GET /health`

Liveness + store readiness (the docker healthcheck target):

```json
{"status": "ok", "store": "reachable"}
```

---

## `GET /metrics`

Prometheus text exposition: WebSub verification outcomes, deliveries
(success/exhausted), retries, 410 terminations, denied notifications,
queue drops, rate-limit hits, active subscriptions, fragment requests
and inserts.

```sh
curl http://localhost:8484/metrics
```

```text
# TYPE semweb_deliveries_total counter
semweb_deliveries_total{result="success"} 3
...
```

---

## `POST /hub`

WebSub subscription request — form-encoded per docs/WebSub.md §5.1.
Unknown additional parameters are ignored, as the spec requires.

| Param | Meaning |
|---|---|
| `hub.mode` | `subscribe`, `unsubscribe`, or `publish` |
| `hub.topic` | Topic URL; MUST be the rel=self URL from discovery |
| `hub.callback` | Subscriber callback URL (query string is preserved, §5.1.1) |
| `hub.lease_seconds` | Optional requested lease; hub clamps to [60s, 10 days], default 1 day (§5.3: expirations are mandatory, never perpetual) |
| `hub.secret` | Optional HMAC secret, MUST be < 200 bytes (§5.1) |

Subscriptions are rate-limited per callback (burst 10, refill 10/min;
exceeded → `429`), **persist to the store in a dedicated named graph**
(a restart reloads them; expired leases are dropped on load per §5.3),
and secrets are **AES-256-GCM encrypted at rest** when
`SEMWEB_SECRET_KEY` is configured.

Hub policies (§5.1 allows hubs to set their own): `SEMWEB_HUB_TOKEN`
requires a bearer token on POST /hub; `SEMWEB_REQUIRE_HTTPS_CALLBACKS`
rejects http:// callbacks that registered a secret; `SEMWEB_CALLBACK_ALLOWLIST`
restricts callback hosts (suffix match).

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
  missing parameters, secret ≥ 200 bytes).

### Verification handshake (§5.3)

The hub GETs the callback with `hub.mode`, `hub.topic`, `hub.challenge`
and (for subscribe) `hub.lease_seconds` appended to the callback's
existing query string. The callback must respond 2xx with the challenge
as the body. The example `demo-subscriber` binary shows the correct
subscriber behavior, including the §8.2 safe-media-type + nosniff
mitigations. Unsubscribe works identically with `hub.mode=unsubscribe`
(no lease semantics).

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
Unknown `hub.url`s are rejected with 400.

---

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
{"@id":"http://example.org/alice","foaf:name":"Alice"}
...
```

---

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

The body accepts an optional `"graph"` for tenant isolation (absent =
default graph).

```json
{
  "event_id": "440b3df8889a033b",
  "inserted": {"@id": "http://example.org/carol", "foaf:knows": {"@id": "http://example.org/alice"}},
  "data_subscribers_notified": 1,
  "schema_changed": false,
  "schema_subscribers_notified": 0
}
```

- `event_id` — one id for this mutation; ties write → publish → fan-out
  together across logs, metrics and (via the Link metadata) deliveries.
- `schema_changed` — `true` when the insert introduced a class or
  predicate not previously in use (detected O(1) from the counters, not
  by re-diffing the whole schema) — `/topics/schema` fires in addition
  to `/topics/data`.
- `*_subscribers_notified` — deliveries enqueued to the bounded queue
  for active (unexpired) subscribers; workers perform the actual POSTs
  asynchronously with the spec's retry contract.

---

## Error responses

Errors are plain text (per §5.1.2, "to assist the client developer in
understanding the error"):

- `400` — missing/invalid hub parameters, unknown publish target
- `401` — write path (or `/hub`, when SEMWEB_HUB_TOKEN is set) called without the configured bearer token
- `404` — unknown topic or `hub.topic` not resolvable to a topic
- `429` — subscription rate limit exceeded for this callback
- `500` — store error (also logged with `tracing`)
- `502` — store unreachable (on `/sparql`)