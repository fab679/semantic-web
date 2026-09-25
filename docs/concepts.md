# Concepts, in plain language

This project sits at the crossing of two worlds — the **Semantic Web**
(W3C standards for machine-readable data) and **plain web development**
(HTTP, JSON, REST). If you have never worked with RDF before, this page
gives you everything you need. Ten minutes here saves you an hour
elsewhere.

## The data side

### A knowledge graph is rows of three columns

A knowledge graph stores facts. Each fact is a **triple** of three
values:

```
subject              predicate                 object
──────────────────   ───────────────────────   ─────────────────────────
http://…/alice       http://xmlns.com/…/name   "Alice"
http://…/alice       http://schema.org/…      http://…/acme
http://…/acme        http://…/foundingDate     "2001-04-03" (a date)
```

Read a row aloud as a sentence: *"Alice's name is Alice"*, *"Alice
works-for Acme"*, *"Acme's founding date is 2001-04-03"*. That is all a
graph is — millions of these little sentences, stored so they can be
joined in any direction. This format is the W3C standard called **RDF**.

### URIs are the identifiers

Everything that has identity gets a **URI** (a web address) instead of a
row id. `http://xmlns.com/foaf/0.1/name` is not a web page you must
visit — it *is* the name of the "name" relationship, globally unique and
clickable. This is why data from different systems can merge: two
datasets that both use the FOAF `name` term mean the same thing.

**Prefixes** are just abbreviations. Instead of writing
`http://xmlns.com/foaf/0.1/name` everywhere, you register `foaf:` as a
shorthand and write `foaf:name`. The prefixes this service ships with:
`rdf`, `rdfs`, `owl`, `xsd`, `sh` (SHACL), `foaf`, `schema` (schema.org),
`dcterms`, `skos`.

### SPARQL is the query language

**SPARQL** is to graphs what SQL is to tables. You describe the shape of
the facts you want, with variables in the positions you don't care
about:

```sparql
# "Who works for Acme, and what are their names?"
SELECT ?who ?name WHERE {
  ?who <http://schema.org/worksFor> <http://example.org/acme> .
  ?who <http://xmlns.com/foaf/0.1/name> ?name .
}
```

The store behind this service (Oxigraph by default) is a **SPARQL 1.1
endpoint**: you send it queries over HTTP, it sends back JSON. This
service talks that same standard protocol — the store is swappable.

### JSON-LD is the wire format

**JSON-LD** is JSON that carries its own vocabulary. Two tricks make it
readable:

- a **`@context`** maps short names to full URIs
  (`"foaf": "http://xmlns.com/foaf/0.1/"`),
- **`@id`** marks the identifier of the thing a JSON object describes.

This service emits *compacted* JSON-LD: one triple per line, short
names, full URIs still available via `GET /context.jsonld`. Typed
values keep their types: `{"@value": "2001-04-03", "@type": "xsd:date"}`
is a date, not a string.

### Named graphs are the tenancy boundary

One store can hold many graphs of triples, each under its own URI
("named graph"). This service maps **one tenant to one named graph**:
writes accept `"graph": "…"`, reads accept `?graph=…`. Reads without
`?graph=` see only the default graph — tenants can't see each other.

## The interaction side

### Triple Pattern Fragments (TPF) — the read primitive

Instead of learning SPARQL to read, you ask a pattern: give me triples
where the **subject** is X and the **predicate** is Y — any position you
leave out is a wildcard. `GET /fragments?predicate=…` is "everything
with this relationship". Results stream as JSON lines with cursor
pagination. It is the 80% read case; full SPARQL is still there for the
other 20%.

### WebSub — push notifications, standardized

**WebSub** is a W3C Recommendation for *push*: you register a callback
URL with a **hub** (that's this service), it verifies you exist by
sending a random challenge you must echo back, and from then on it POSTs
the full topic content to your callback whenever the data changes —
signed with an HMAC you can verify. Subscribers can be offline between
events; delivery retries until you acknowledge.

### SSE — push for live sessions

**Server-Sent Events** is the browser-native way to hold one HTTP
connection open and receive events. Same change bus as WebSub, different
transport: WebSub for durable backends, SSE for live sessions (a
dashboard, an agent run).

### SHACL — declared constraints

**SHACL** shapes describe what a valid record looks like (a person has
exactly one name; at most five `knows` links). This service loads a
shapes file and surfaces the constraints in `GET /manifest` so agents
can plan against them.

### MCP — the LLM-agent protocol

**MCP** (Model Context Protocol) is the standard way LLM agents discover
and call tools. This service is an MCP server: an agent asks
`tools/list`, gets graph tools (`search_graph`, `sparql_query`, …), and
calls them like any other tool. Same JSON-RPC 2.0 shape, over plain
HTTP POST.

## The three primitives of this service

Everything the service does reduces to three plain-HTTP primitives:

| # | Primitive | Plain meaning | Endpoint |
|---|---|---|---|
| 1 | **Read** | "give me the facts matching this pattern" | `GET /fragments` (+ `GET /sparql` for the rest) |
| 2 | **Describe** | "what exists in this graph right now?" | `GET /`, `/context.jsonld`, `/manifest` |
| 3 | **Push** | "tell me when it changes" | `POST /hub` (WebSub) + `GET /events` (SSE) |

Plus a write endpoint (`POST /admin/insert`), an MCP server (`POST
/mcp`), health/metrics, and a browser UI. That is the whole surface —
there is no SDK, and every response is JSON you can read with your
eyes.

## How the demo data makes sense of it

`docker compose up` loads a tiny seed graph of people and companies:

- **Alice**, **Bob** and **Carol** are `foaf:Person`s with `foaf:name`s;
  Alice knows Bob, Bob knows Carol.
- Alice and Bob **work for** Acme Corp; Carol works for Globex.
- Acme has a `foundingDate` (a typed `xsd:date`).

Every guide on this site uses this graph, and the docs show the output
you actually get — including the **friendly names** the demo stack
configures (`name`, `knows`, `employer`, `employs`, `founded`) so you
rarely see a raw URI.

## Where to next

- Watch it run in 5 commands: [Quickstart](getting-started.md)
- Read the graph from an app: [For developers](for-developers.md)
- Every endpoint, request and response: [API reference](api.md)
- Get pushes when the graph changes: [WebSub guide](websub-guide.md)