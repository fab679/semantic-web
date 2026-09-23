# What this is, and why it matters

## The problem in plain terms

A **knowledge graph** stores facts as connections:

> Alice **works for** Acme Corp · Alice **knows** Bob · Acme Corp **was founded** on 2001-04-03

Each fact is one **triple**: *thing — relationship — thing*. Graphs are
how the web itself is described (it's what powers rich search results),
how large companies model their entire organizations, products and
devices, and increasingly how AI agents ground their knowledge in
something real instead of a model's memory.

But raw graphs are hostile to work with:

1. **Everything is a raw URI.** A person's name is
   `http://xmlns.com/foaf/0.1/name`. Humans squint; LLM agents
   hallucinate; application developers drown.
2. **Nobody answers "what's in here?"** Ask a graph what classes and
   relationships it holds and you get… a technical dump. There is no
   self-description, no docs, no list of "fields you can use".
3. **It's frozen in time.** When the data changes — a new kind of
   relationship appears — every consumer built on yesterday's snapshot
   silently goes stale.

## What this project is

**Streaming Semantic Fragments** is a small Rust service that turns an
RDF knowledge graph (the W3C standard triple format, stored in
[Oxigraph](https://oxigraph.org)) into something that talks like a modern
web API:

| Capability | In one line | Standard it rides on |
|---|---|---|
| Friendly reads | `{"@id": "…/alice", "employer": {"@id": "…/acme"}}` — streamed, one triple per line | HTTP streaming + JSON-LD |
| Live self-description | "what exists right now" — classes, predicates, meanings, constraints — computed fresh on every request | plain REST |
| Change push | the graph tells you when it changed; you never poll | WebSub (W3C) + SSE |
| Agent surface | an MCP server whose tools, resources and prompts are the graph | MCP (Model Context Protocol) |

Three ideas carry the whole design — everything else follows from them:

**1. Friendly names, zero compromise.** You never read a raw URI. A
shared vocabulary context maps `http://xmlns.com/foaf/0.1/name` to
`name`, `foaf:knows` to `knows`. Standard vocabularies compact
automatically; private namespaces get names at runtime; unknown things
stay honest full URIs. Typed values keep their types (`"2001-04-03"` stays
a date).

**2. The description IS the data.** There is no schema file to compile,
no build step, no cache to bust. Every request for "what does this graph
contain?" is answered from the live store. Insert a triple with a
brand-new kind of relationship and the very next request reflects it.
A content fingerprint tells you exactly when your understanding went
stale.

**3. The graph pushes.** Subscribe once (a URL plus an optional secret);
when data changes, the service delivers the full current content of the
topic you follow — signed so you can verify it, retried until you
acknowledge it. Backends that sleep between events and live dashboards
that never close are both first-class (WebSub for the former, SSE for the
latter).

## Why it matters

**For people and AI assistants:** the graph becomes *queryable in
natural terms*. An agent can ask "what does this graph know about
people?" and get back not just rows of data but a current, grounded
vocabulary — with descriptions written in human language — and then
fetch facts using terms it has *actually seen*, never invented. That
grounding is what separates reliable data-backed AI answers from
confident hallucination.

**For product teams:** the surface stays correct as the business evolves.
New attributes, new entity types, new tenants — they appear in the
self-description the moment the data does. Nobody schedules a
"regenerate the API" sprint; there is nothing to regenerate.

**For engineering teams:** every piece is a standard. Reads are REST with
NDJSON streaming and cursor pagination. Change notification is the W3C
WebSub Recommendation — verified challenges, expiring leases, HMAC-signed
deliveries, retries, at-least-once durability — plus SSE for live
sessions. Secrets are AES-256-GCM encrypted at rest. The store is any
SPARQL 1.1 endpoint (the service speaks the protocol, not a product API),
so capacity and replication are infrastructure decisions, not rewrites.
Scale-out is built in: replicas share one store and each delivers only
the subscriptions its consistent-hash shard owns. Metrics are Prometheus.
It ships as one static Rust binary.

**For data and platform engineers:** the ontology is not a code artifact.
No compiled schemas, no redeploy on vocabulary change, no versioning
machinery. The service adapts because it asks the store — which is also
why a schema change event and its fingerprint exist: consumers can diff
their understanding safely instead of guessing.

## How people actually use it

| You are | What you do | Where to go |
|---|---|---|
| Non-technical / evaluating | skim this page, then watch the [30-second quickstart](getting-started.md) | [Quickstart](getting-started.md) |
| App developer | pattern reads, cursor paging, SPARQL fallback, token-gated writes | [Reading the graph](for-developers.md) |
| Backend service owner | subscribe your webhook, get signed pushes | [WebSub guide](websub-guide.md) |
| AI engineer / agent builder | manifest for grounding, MCP tools for execution, SSE for reactivity | [For agents](for-agents.md), [Agent tutorial](agent-tutorial.md) |
| Platform / SRE | topology, sharding, durability, security posture | [For engineering teams](engineering.md), [Feature list](features.md) |

## Usage in one breath

```sh
docker compose up --build -d          # store + service

curl http://localhost:8484/            # what exists? (live)
curl "http://localhost:8484/fragments?predicate=http://schema.org/worksFor"
curl http://localhost:8484/manifest    # the agent manifest
curl -X POST http://localhost:8484/hub -d "hub.mode=subscribe" ...   # real-time
```

---

The rest of this site: [Quickstart](getting-started.md) ·
[For developers](for-developers.md) · [For agents](for-agents.md) ·
[Agent tutorial](agent-tutorial.md) · [Architecture](architecture.md) ·
[Scaling](scaling.md)