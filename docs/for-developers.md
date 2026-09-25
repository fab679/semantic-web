# Reading the graph (for developers)

Everything is plain HTTP — no SDK needed, curl works. The two read
primitives and one write path cover 95% of app use cases. All examples
below run against the demo stack (`docker compose up`) with its seed
graph (Alice, Bob, Carol, Acme Corp, Globex) and its configured friendly
names (`name`, `knows`, `employer`, `employs`, `founded`).

## 1. Look up patterns (`GET /fragments`)

Give up to three positions (`subject`, `predicate`, `object`); any
position you leave out is a wildcard. This is the Triple Pattern
Fragment lookup:

```sh
# Who works for whom?
curl "http://localhost:8484/fragments?predicate=http://schema.org/worksFor"
# {"@id":"http://example.org/alice","employer":{"@id":"http://example.org/acme"}}
# {"@id":"http://example.org/bob","employer":{"@id":"http://example.org/acme"}}
# {"@id":"http://example.org/carol","employer":{"@id":"http://example.org/globex"}}
# {"@control":"metadata","count_estimate":3}

# Everything about Acme Corp
curl "http://localhost:8484/fragments?subject=http://example.org/acme"
# {"@id":"http://example.org/acme","employs":{"@id":"http://example.org/alice"}}
# {"@id":"http://example.org/acme","employs":{"@id":"http://example.org/bob"}}
# {"@id":"http://example.org/acme","founded":{"@value":"2001-04-03","@type":"xsd:date"}}
# {"@id":"http://example.org/acme","name":"Acme Corp"}
# {"@id":"http://example.org/acme","@type":"foaf:Organization"}
# {"@control":"metadata","count_estimate":5}

# Find by literal value
curl "http://localhost:8484/fragments?predicate=http://xmlns.com/foaf/0.1/name&object=Alice"
# {"@id":"http://example.org/alice","name":"Alice"}
# {"@control":"metadata","count_estimate":1}
```

Notes on the wire format:

- Response content type is `application/x-ndjson`: **one compacted
  JSON-LD statement per line**, streamed — you can act on line 1 before
  the page finishes.
- **Friendly names** come from the shared JSON-LD context
  (`/context.jsonld`): standard vocabularies compact to CURIEs
  (`foaf:name`, `xsd:date`), terms configured via `SEMWEB_TERM_ALIASES`
  get bare names (`name`, `employer`… — the demo stack ships five), and
  unknown namespaces stay full URIs — no invented names, ever.
- `rdf:type` compacts to `@type`.
- **Typed literals keep their type** and language tags survive:
  `{"@value":"2001-04-03","@type":"xsd:date"}` is a date, not a string.
- An object value starting with `http(s)://` matches as a URI;
  anything else matches as a plain literal.

## 2. Page deterministically

The last line is a control line (identified by `"@control"`). To get the
next page, follow the `after` cursor — it is an O(1) seek, not an OFFSET
scan:

```json
{"@control":"metadata","count_estimate":25,"after":"WyJodHRwOi8vZXhhbXBsZS5vcmcvYWNtZSIs…"}
```

```sh
curl "http://localhost:8484/fragments?limit=20"
# then
curl "http://localhost:8484/fragments?limit=20&after=<the after value>"
```

The cursor is opaque (base64 of the last-seen triple). Pages are
deterministic — ordered by the string forms of subject, predicate,
object — so paging never overlaps or repeats. `limit` defaults to 100
and caps at 1000. An `offset` parameter remains for legacy clients
(both continuation values travel in the control line).

## 3. When fragments aren't enough: SPARQL

The execution plane is real SPARQL, through the same origin:

```sh
curl "http://localhost:8484/sparql?query=SELECT%20%3Fs%20%3Fname%20WHERE%20%7B%20%3Fs%20%3Chttp%3A%2F%2Fxmlns.com%2Ffoaf%2F0.1%2Fname%3E%20%3Fname%20%7D"
```

Read-only **by construction**: queries are forwarded to the store's
query endpoint, which cannot execute updates (updates live on a separate
URL this handler never touches). Results come back in SPARQL JSON.

## 4. Write one triple

```sh
curl -X POST http://localhost:8484/admin/insert \
  -H "Authorization: Bearer demo-write-token" \
  -H "Content-Type: application/json" \
  -d '{"subject":"http://example.org/dave","predicate":"http://schema.org/worksFor","object":"http://example.org/acme"}'
```

```json
{"event_id":"c65e0d3ac1d09f42",
 "inserted":{"@id":"http://example.org/dave","employer":{"@id":"http://example.org/acme"}},
 "duplicate":false,
 "data_subscribers_notified":1,
 "schema_changed":false,
 "schema_subscribers_notified":0}
```

Behavior:

- `object` starting with `http://` / `https://` is stored as a **URI**;
  anything else as a **plain literal**.
- Re-inserting an existing triple is a **no-op end to end**: the response
  carries `"duplicate": true` with no counter update, no topic publish,
  and no notifications.
- `graph` URIs starting with `http://semweb.dev/graph/` are reserved for
  service state and rejected with `400`.
- The response's `event_id` is traceable end-to-end through logs,
  metrics and notifications.
- `data_subscribers_notified` counts deliveries enqueued to active
  WebSub subscribers (workers POST them asynchronously).
- `schema_changed` is `true` when the insert introduced a class or
  predicate not previously in use — `/topics/schema` then fires in
  addition to `/topics/data`.
- Optional `"graph": "…"` isolates tenants into named graphs.
- Without the bearer token (when `SEMWEB_WRITE_TOKEN` is set) the
  endpoint answers `401`.

In production your ingestion pipeline (SPARQL UPDATE endpoint, ETL)
replaces this demo endpoint — the invariant is: **whatever writes must
publish** the affected topics, which the write path does.

## 5. Multi-tenancy: named graphs

```sh
curl -X POST http://localhost:8484/admin/insert \
  -H "Authorization: Bearer demo-write-token" \
  -H "Content-Type: application/json" \
  -d '{"subject":"http://example.org/dave","predicate":"http://xmlns.com/foaf/0.1/name","object":"Dave (tenant A)","graph":"http://example.org/graphs/tenant-a"}'

curl "http://localhost:8484/fragments?graph=http://example.org/graphs/tenant-a"
```

Default-graph reads never see tenant data and vice versa. Isolation is
structural (separate graphs), so it needs no credentials.

## 6. Authentication

The service separates **reads** (open by default — discovery and
grounding shouldn't need credentials) from **writes** (token-gated):

| What | How |
|---|---|
| Reads (`/`, `/fragments`, `/sparql`, `/manifest`, `/events`, `/mcp`, `/ui`) | open |
| Writes (`POST /admin/insert`, MCP `insert_triple`) | `Authorization: Bearer <SEMWEB_WRITE_TOKEN>` — 401 without it |
| Who may subscribe (`POST /hub`) | optionally gated by `Authorization: Bearer <SEMWEB_HUB_TOKEN>` |
| Notification integrity | every push is HMAC-SHA256-signed with your subscription `hub.secret`; verify before trusting the payload |
| Subscriber secrets at rest | AES-256-GCM encrypted in the store when `SEMWEB_SECRET_KEY` is set |
| Tenant isolation | named graphs (`?graph=` / `"graph"`) — no credentials needed |

Callback policies for subscribers: `SEMWEB_REQUIRE_HTTPS_CALLBACKS=1`
rejects secret'd subscriptions over `http://`, and
`SEMWEB_CALLBACK_ALLOWLIST=host1,host2` restricts callback hosts. The
full matrix is in the [configuration reference](configuration.md).

For WebSub specifically — the verification handshake, signature
validation and subscriber checklist — see the
[WebSub guide](websub-guide.md#subscriber-checklist).

## 7. Extending the vocabulary

New terms from **standard vocabularies** (FOAF, schema.org, SKOS, …)
compact automatically — `foaf:mbox`, `schema:address` work with zero
code changes. A private namespace gets a friendly prefix at runtime:

```sh
SEMWEB_EXTRA_PREFIXES="ex=http://example.org/vocab/" cargo run -p semweb
```

And the terms your consumers read most can get **bare friendly names**
at runtime:

```sh
SEMWEB_TERM_ALIASES="age=http://example.org/vocab/age" cargo run -p semweb
```

Both appear in `/context.jsonld` as JSON-LD prefix / term definitions,
so JSON-LD processors round-trip them. The demo compose stack ships
five aliases (`name`, `knows`, `employer`, `employs`, `founded`).

## 8. What's always current

`GET /` (self-description) and `GET /manifest` (agent manifest) are
computed live on every request. Insert a triple with a brand-new
predicate and the very next call reflects it — no rebuild, no cache to
bust. That is the core architectural promise; [for agents](for-agents.md)
is the guide to exploiting it.