# How agents use this

An agent needs three things from a data source: **discovery** (what
exists), **grounding** (what it means), **execution** (get the facts),
and ideally **reactivity** (know when it changed). This service provides
all four as plain HTTP — no SDK, no training.

## The four capabilities

### 1. Discovery — `GET /` and `GET /manifest`

The manifest is the planning surface: every class and predicate, live
cardinalities, **human descriptions** pulled from the graph
(`rdfs:comment` / `skos:definition`), SHACL shapes (what's required,
what's bounded), and an executable example SPARQL query per class. The
`schemaFingerprint` identifies the exact ontology version so an agent can
detect that its understanding went stale.

```json
{
  "schemaFingerprint": "sha256:9beb...",
  "classes": [{
    "compact": "foaf:Person",
    "instances": 3,
    "description": "A person as described by the FOAF vocabulary.",
    "shapes": [{"path": "http://xmlns.com/foaf/0.1/name", "minCount": 1}],
    "exampleQuery": "SELECT ?s WHERE { ?s a <http://xmlns.com/foaf/0.1/Person> } LIMIT 10"
  }]
}
```

### 2. Execution — two ways

- **Fragments** (`GET /fragments`) — pattern lookup, streaming NDJSON,
  for the "look up who works at X" cases.
- **SPARQL** (`GET /sparql?query=...`) — read-only, for multi-hop or
  aggregate questions. SPARQL is *fine* for LLMs to write; the hard part
  was never the query language — it was knowing the vocabulary, which the
  manifest solves.

### 3. Real-time — WebSub and SSE

Durable agent-side services subscribe via WebSub (signed full-content
pushes). Live agent runs watch `GET /events` (SSE) — the notification
tells them *the graph changed*, and they refetch. No polling, no stale
caches.

### 4. MCP — the native LLM-agent surface

`POST /mcp` is a full [MCP](https://modelcontextprotocol.io) server:
JSON-RPC 2.0, three capability groups:

| Group | What an agent gets |
|---|---|
| **tools** | `search_graph`, `sparql_query`, `get_manifest`, `get_topic`, `insert_triple`, `subscribe` — with JSON-Schema inputs and bounded outputs |
| **resources** | the live agent manifest at `manifest://semantic-web/current` |
| **prompts** | `explore_graph`, `answer_from_graph` — guided method templates |

MCP-aware agents (Claude, Cursor, custom harnesses) list these the same
way they list their other tools. Adding a tool server-side extends every
agent without touching it.

```sh
curl -X POST http://localhost:8000/mcp -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call",
       "params":{"name":"search_graph",
                 "arguments":{"predicate":"http://schema.org/worksFor"}}}'
```

## The design pattern (the part worth stealing)

This is the **semantic plane / execution plane** split, made concrete:

- the LLM **plans** against the manifest (small, human-readable, current),
- deterministic code (fragments/SPARQL) executes the plan,
- the answer cites URIs the tools actually returned.

Never let the model free-form SPARQL against a graph it hasn't seen the
manifest of — that's the failure mode this design exists to prevent.

## Authentication: what agents need to know

The trust model has three layers, each answering a different question.

**1. Reads are open by design.** The manifest, fragments, `/sparql`,
`/events` and MCP reads need no credentials. Discovery and grounding
contain no data (only the vocabulary), so an agent can bootstrap without
secrets.

**2. Writes: the agent carries its own credential.** When the deployment
sets `SEMWEB_WRITE_TOKEN`, the MCP `insert_triple` tool accepts a
`token` argument and the server validates it. The agent operator controls
whether writes exist at all — issue the token or don't. The
[tutorial agent](agent-tutorial.md) reads `SEMWEB_WRITE_TOKEN` from its
environment and attaches it to `insert_triple` calls automatically; a
write attempt without a valid token comes back as a tool error, which
the model treats as "writes are not available".

**3. Push: authenticated to the subscriber, not by it.** This is the
layer agents get the most value from, and it is inverted from normal
auth:

- On `subscribe`, the agent's callback service supplies a `hub.secret`
  it chooses itself — a shared secret with the hub.
- Every delivered notification carries
  `X-Hub-Signature: sha256=HMAC(body, secret)`. The callback verifies it
  and **discards mismatches locally** (spec §7.1.2) while still 2xx-acking
  (acknowledging receipt prevents brute force; verification happens after).
- So "is this really the hub?" is answered per-payload by HMAC — even if
  the callback runs plain HTTP. For stronger guarantees the deployment
  can set `SEMWEB_REQUIRE_HTTPS_CALLBACKS=1` (the hub then rejects
  secret'd subscriptions over plain `http://`).
- The callback URL should be an **unguessable capability URL** — before
  the signature check, it is the only thing protecting the endpoint.

Server-side hardening the hub applies to agent subscriptions: secret
length (< 200 bytes, §5.1), challenge charset/no-binary rules (§8.2),
per-callback rate limiting (429), AES-256-GCM encryption of stored
secrets, and the optional host allowlist.

**Tenancy:** named graphs via the `graph` argument on reads/writes give
structural isolation. Scoping *which graphs* per agent identity (per-agent
credentials and topic-level authz) is the documented next step — see
[scaling.md §3](scaling.md).

### The practical recipe

```
public read by default
→ insert_triple only with your deployment's token
→ every pushed notification HMAC-verified with the subscription secret
→ capability-URL callbacks (unguessable, HTTPS when secrets are used)
→ named graphs when you need isolation
```

## Worked example

See [the Together AI agent tutorial](agent-tutorial.md) — a complete,
runnable ~200-line agent whose tools are *discovered from the server*
via MCP, grounded on the manifest, and executed against the graph with
Together AI's Llama 3.3 70B.

## What an agent should NOT do

- Don't memorize the vocabulary — read the manifest each session (it's
  live, and the fingerprint tells you when it changed).
- Don't fabricate predicates not in the manifest.
- Don't poll — subscribe (`POST /hub` with a callback, or watch
  `/events`).
- Don't trust unverified pushes — validate the HMAC with your
  subscription secret.