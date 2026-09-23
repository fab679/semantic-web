# Reading the graph (for developers)

Everything is plain HTTP. No SDK needed — curl works. The two read
primitives and one write path cover 95% of app use cases.

## 1. Look up patterns (`GET /fragments`)

Give up to three positions; missing ones are wildcards. This is the
Triple Pattern Fragment lookup:

```sh
# Who works at Acme?
curl "http://localhost:8484/fragments?subject=http://example.org/acme&predicate=http://schema.org/employee"

# Find by literal value
curl "http://localhost:8484/fragments?predicate=http://xmlns.com/foaf/0.1/name&object=Alice"
```

You get NDJSON — one compacted JSON-LD statement per line, streamed:

```json
{"@id": "http://example.org/acme", "employer": {"@id": "http://example.org/alice"}}
```

Friendly names come from the shared JSON-LD context
(`/context.jsonld`): standard vocabularies compact to CURIEs
(`foaf:name`, `xsd:date`), configured terms get bare names (`name`,
`employer` — the demo stack ships five via `SEMWEB_TERM_ALIASES`), and
unknown namespaces stay as full URIs — no invented names, ever.

Typed literals keep their type:

```json
{"@id": "http://example.org/acme", "founded": {"@value": "2001-04-03", "@type": "xsd:date"}}
```

## 2. Page deterministically

The last line is a control line (identified by `"@control"`). Follow the
`after` cursor — it's an O(1) seek, not an OFFSET scan:

```json
{"@control": "metadata", "count_estimate": 25,
 "after": "WyJodHRw...", "next": "/fragments?...&offset=2"}
```

```sh
curl "http://localhost:8484/fragments?limit=20"
# then
curl "http://localhost:8484/fragments?limit=20&after=<the after value>"
```

## 3. When fragments aren't enough: SPARQL

The execution plane is real SPARQL, through the same origin:

```sh
curl "http://localhost:8484/sparql?query=SELECT%20%3Fs%20%3Fname%20WHERE%20%7B%20%3Fs%20%3Chttp%3A%2F%2Fxmlns.com%2Ffoaf%2F0.1%2Fname%3E%20%3Fname%20%7D"
```

Read-only by construction (it hits the store's query endpoint; updates
live on a separate URL this handler never touches).

## 4. Write one triple

```sh
curl -X POST http://localhost:8484/admin/insert \
  -H "Authorization: Bearer demo-write-token" \
  -H "Content-Type: application/json" \
  -d '{"subject":"http://example.org/dave","predicate":"http://schema.org/worksFor","object":"http://example.org/acme"}'
```

Response includes an `event_id` (traceable end-to-end through logs and
notifications) and delivery counts. Optional `"graph"` isolates tenants
into named graphs.

In production your ingestion pipeline (SPARQL UPDATE endpoint, ETL)
replaces this demo endpoint — the invariant is: **whatever writes must
publish** the affected topics, which the write path does.

## 5. Multi-tenancy: named graphs

```sh
curl "http://localhost:8484/fragments?graph=http://example.org/graphs/tenant-a"
```

Default-graph reads never see tenant data.

## 6. Authentication

The service separates **reads** (open by default — discovery and
grounding shouldn't need credentials) from **writes** (token-gated):

| What | How |
|---|---|
| Reads (fragments, `/sparql`, `/manifest`, `/events`) | open |
| Writes (`POST /admin/insert`) | `Authorization: Bearer <SEMWEB_WRITE_TOKEN>` — 401 without it |
| Who may subscribe (`POST /hub`) | optionally gated by `Authorization: Bearer <SEMWEB_HUB_TOKEN>` |
| Notification integrity | every push is HMAC-SHA256-signed with your subscription `hub.secret`; verify before trusting the payload |
| Subscriber secrets at rest | AES-256-GCM encrypted in the store when `SEMWEB_SECRET_KEY` is set |
| Tenant isolation | named graphs (`?graph=` / `"graph"`) — no credentials needed, structural isolation |

Callback policies for subscribers: `SEMWEB_REQUIRE_HTTPS_CALLBACKS=1`
rejects secret'd subscriptions over `http://`, and
`SEMWEB_CALLBACK_ALLOWLIST=host1,host2` restricts callback hosts. The
full matrix is in the [configuration reference](configuration.md).

For WebSub specifically — the verification handshake, signature
validation and subscriber checklist — see the
[WebSub guide](websub-guide.md#subscriber-checklist-spec-82).

## 7. Extending the vocabulary

New terms from **standard vocabularies** (FOAF, schema.org, SKOS, ...)
compact automatically — `foaf:mbox`, `schema:address` work with zero
code changes. A private namespace gets a friendly prefix at runtime:

```sh
SEMWEB_EXTRA_PREFIXES="ex=http://example.org/vocab/" cargo run -p semweb
```

## 8. What's always current

`GET /` (self-description) and `GET /manifest` (agent manifest) are
computed live on every request. Insert a triple with a brand-new
predicate and the very next call reflects it — no rebuild, no cache to
bust. That is the core architectural promise.