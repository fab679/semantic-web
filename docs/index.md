# What this is, and why it matters

## The problem in one page

A **knowledge graph** stores facts as triples — *thing — relationship —
thing*:

> Alice **works for** Acme Corp · Alice **knows** Bob · Acme **founded**
> 2001-04-03

Graphs power rich search results, large companies' internal data models,
and increasingly the grounding that keeps AI agents honest. But raw
graph infrastructure is hostile to work with:

1. **Everything is a raw URI.** A person's name is
   `http://xmlns.com/foaf/0.1/name`. Humans squint; LLM agents
   hallucinate; application developers drown.
2. **Nobody answers "what's in here?"** Ask a store which classes and
   relationships it holds and you get a raw dump. No self-description,
   no "what fields can I use", no meanings.
3. **It's frozen in time.** Consumers build against a snapshot; when a
   new kind of relationship appears, everything built on yesterday's
   snapshot silently goes stale.

## What this project is

**Streaming Semantic Fragments** is a small Rust service that turns an
RDF store ([Oxigraph](https://oxigraph.org) by default, *any* SPARQL 1.1
endpoint in general) into a friendly, always-current, real-time web API.
No new protocol, no translation layer in front of the store — the store
keeps speaking SPARQL 1.1, and the service adds the layer that was
missing: discovery, streaming reads, and change push.

| Capability | In one line | Standard it rides on |
|---|---|---|
| Friendly reads | `{"@id": "…/alice", "name": "Alice"}` — streamed, one triple per line | HTTP streaming + JSON-LD |
| Live self-description | "what exists right now" — classes, predicates, meanings, constraints — computed fresh on every request | plain REST |
| Change push | the graph tells you when it changed; you never poll | WebSub (W3C) + SSE |
| Agent surface | an MCP server whose tools, resources and prompts are the graph | MCP |

Three ideas carry the whole design:

**1. Friendly names, zero compromise.** You never have to read a raw
URI — and nothing is hardcoded. Standard vocabularies compact
automatically (`http://xmlns.com/foaf/0.1/mbox` → `foaf:mbox`); private
namespaces get prefixes at runtime (`SEMWEB_EXTRA_PREFIXES`); and the
terms your consumers read most get **bare friendly names** at runtime
(`SEMWEB_TERM_ALIASES=name=http://xmlns.com/foaf/0.1/name` → `"name"` —
the demo stack ships `name`, `knows`, `employer`, `employs`, `founded`).
Unknown things stay honest full URIs. Everything round-trips through
`/context.jsonld`, and typed values keep their types (`"2001-04-03"`
stays a date). New to this? [Concepts](concepts.md) explains all the
moving parts in plain language.

**2. The description IS the data.** There is no schema file to compile,
no build step, no cache to bust. Every request for "what does this graph
contain?" is answered from the live store. Insert a triple with a
brand-new kind of relationship and the very next request reflects it.
A content fingerprint tells you exactly when your understanding went
stale.

**3. The graph pushes.** Subscribe once (a URL plus an optional
secret); when data changes, the service delivers the full current
content of the topic you follow — HMAC-signed so you can verify it,
retried until you acknowledge it. Backends that sleep between events
and live dashboards that never close are both first-class (WebSub for
the former, SSE for the latter).

## Why it matters

**For people and AI assistants:** the graph becomes *queryable in
natural terms*. An agent asks "what does this graph know about people?"
and gets back a current, grounded vocabulary — with human-language
descriptions — then fetches facts using terms it has *actually seen*,
never invented. That grounding is what separates data-backed AI answers
from confident hallucination.

**For product teams:** the surface stays correct as the business
evolves. New attributes, new entity types, new tenants — they appear in
the self-description the moment the data does. Nobody schedules a
"regenerate the API" sprint; there is nothing to regenerate.

**For engineering teams:** every piece is a standard. Reads are REST
with NDJSON streaming and cursor pagination. Change notification is the
W3C WebSub Recommendation — verified challenges, expiring leases,
HMAC-signed deliveries, retries, at-least-once durability — plus SSE for
live sessions. Secrets are AES-256-GCM encrypted at rest. The store is
any SPARQL 1.1 endpoint (the service speaks the protocol, not a product
API), so capacity and replication are infrastructure decisions, not
rewrites. Scale-out is built in: replicas share one store and each
delivers only the subscriptions its consistent-hash shard owns. Metrics
are Prometheus. It ships as one static Rust binary.

**For data and platform engineers:** the ontology is not a code
artifact. No compiled schemas, no redeploy on vocabulary change, no
versioning machinery. The service adapts because it asks the store —
and the schema-change event + fingerprint exist so consumers can diff
their understanding safely instead of guessing.

## How people actually use it

| You are | What you do | Where to go |
|---|---|---|
| Non-technical / evaluating | skim this page, then run the [quickstart](getting-started.md) | [Quickstart](getting-started.md) |
| New to RDF / semantic web | read the plain-language tour first | [Concepts](concepts.md) |
| App developer | pattern reads, cursor paging, SPARQL fallback, token-gated writes | [Reading the graph](for-developers.md) |
| Backend service owner | subscribe your webhook, get signed pushes | [WebSub guide](websub-guide.md) |
| AI engineer / agent builder | manifest for grounding, MCP tools for execution, SSE for reactivity | [For agents](for-agents.md), [Agent tutorial](agent-tutorial.md) |
| Platform / SRE | topology, sharding, durability, security posture | [For engineering teams](engineering.md), [Feature list](features.md) |

## Usage in one breath

```sh
docker compose up --build -d            # store + service

open http://localhost:8484/ui           # explore the graph in a browser
curl http://localhost:8484/             # what exists? (live)
curl "http://localhost:8484/fragments?predicate=http://schema.org/worksFor"
curl http://localhost:8484/manifest     # the agent manifest
curl -X POST http://localhost:8484/hub -d "hub.mode=subscribe" ...  # real-time
```

---

The rest of this site: [Concepts](concepts.md) ·
[Quickstart](getting-started.md) · [For developers](for-developers.md) ·
[For agents](for-agents.md) · [Agent tutorial](agent-tutorial.md) ·
[Architecture](architecture.md) · [Scaling](scaling.md)