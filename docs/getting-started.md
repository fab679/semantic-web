# Quickstart

From zero to a running graph API with real-time push, in about five
minutes. Nothing here needs any SDK — curl and a browser are enough.

## 1. Run the stack

```sh
git clone https://github.com/fab679/semantic-web
cd semantic-web
docker compose up --build -d
```

Two containers come up:

| Container | What it is | Where |
|---|---|---|
| `oxigraph` | the triple store (SPARQL 1.1 Protocol); seed data loaded automatically | <http://localhost:7878> |
| `semantic-web` | this service | <http://localhost:8484> |

Wait ~10 seconds for the seed to load (the service retries while the
store boots), then continue.

## 2. First look (30 seconds)

Open the **Graph Explorer** at <http://localhost:8484/ui> — a browser
page that builds itself from the live manifest: browse classes and
descriptions, search patterns, preview topics, watch the live change
feed, see health and metrics. It is a pure client over the API;
everything it shows is plain HTTP you can call yourself.

Then, from a terminal:

```sh
# What does the graph contain? (live, always current)
curl http://localhost:8484/
```

With the demo seed loaded you get the two classes and every predicate in
use — note the friendly names (`name`, `knows`, `employer`…) configured
by the demo stack:

```json
{
  "@context": "/context.jsonld",
  "generatedFrom": "live store state (not a cached build)",
  "storeMode": "sparql-rust",
  "classes": ["foaf:Organization", "foaf:Person"],
  "predicates": ["employs", "founded", "employer", "rdf:type",
                 "rdfs:comment", "skos:definition", "knows", "name"],
  "controls": { "...": "see API reference" }
}
```

```sh
# Stream triples matching a pattern: who works for whom?
curl "http://localhost:8484/fragments?predicate=http://schema.org/worksFor"
```

```json
{"@id":"http://example.org/alice","employer":{"@id":"http://example.org/acme"}}
{"@id":"http://example.org/bob","employer":{"@id":"http://example.org/acme"}}
{"@id":"http://example.org/carol","employer":{"@id":"http://example.org/globex"}}
{"@control":"metadata","count_estimate":3}
```

```sh
# The agent manifest (classes, descriptions, SHACL shapes, examples)
curl http://localhost:8484/manifest
```

## 3. Put data in

Writes are token-gated (the compose stack sets `demo-write-token`). The
object is treated as a URI when it starts with `http(s)://`, otherwise
as a plain literal:

```sh
curl -X POST http://localhost:8484/admin/insert \
  -H "Authorization: Bearer demo-write-token" \
  -H "Content-Type: application/json" \
  -d '{"subject":"http://example.org/dave","predicate":"http://xmlns.com/foaf/0.1/name","object":"Dave"}'
```

```json
{"event_id":"440b3df8889a033b",
 "inserted":{"@id":"http://example.org/dave","name":"Dave"},
 "data_subscribers_notified":0,
 "schema_changed":false,
 "schema_subscribers_notified":0}
```

Read it back immediately — there is no rebuild step:

```sh
curl "http://localhost:8484/fragments?subject=http://example.org/dave"
# {"@id":"http://example.org/dave","name":"Dave"}
# {"@control":"metadata","count_estimate":1}
```

## 4. Watch it change in real time

Terminal 1 — run the demo subscriber (a small WebSub consumer that ships
with the project):

```sh
docker compose --profile demo up -d demo-subscriber
```

Terminal 2 — subscribe it to the data topic, then insert something:

```sh
curl -X POST http://localhost:8484/hub \
  -d "hub.mode=subscribe" \
  -d "hub.topic=http://localhost:8484/topics/data" \
  -d "hub.callback=http://demo-subscriber:9000/callback" \
  -d "hub.secret=demo"

curl -X POST http://localhost:8484/admin/insert \
  -H "Authorization: Bearer demo-write-token" \
  -H "Content-Type: application/json" \
  -d '{"subject":"http://example.org/erin","predicate":"http://xmlns.com/foaf/0.1/knows","object":"http://example.org/alice"}'
```

The subscriber logs a signature-verified, full-content notification:

```sh
docker compose logs -f demo-subscriber
# intent verification: mode=subscribe, topic=…
# content distribution received; signature verified (sha256)
# content-type="application/x-ndjson" link=<…/topics/data>; rel="self", …
#   {"@id":"http://example.org/erin","knows":{"@id":"http://example.org/alice"}}
```

That is the whole real-time loop: **subscribe once, get pushes with the
full topic content, verify the signature**. The mechanics (challenge
handshake, leases, retries) are in the [WebSub guide](websub-guide.md).

## 5. Run it locally (no docker)

```sh
# Oxigraph separately (or any SPARQL 1.1 endpoint)
docker run -p 7878:7878 ghcr.io/oxigraph/oxigraph:0.5.10 serve --bind 0.0.0.0:7878

cargo run -p semweb        # listens on :8484 by default
```

## What's inside

- **Rust service** (axum + tokio) — the whole HTTP surface
- **Oxigraph** — the store; any SPARQL 1.1 endpoint is a drop-in
- Everything is environment-configured (`SEMWEB_WRITE_TOKEN`,
  `SEMWEB_SECRET_KEY`, `SEMWEB_TERM_ALIASES`, sharding vars, …) — the
  full table is in the [configuration reference](configuration.md)

## Where to next

- Understand the building blocks: [Concepts](concepts.md)
- Read the graph from an app: [For developers](for-developers.md)
- Every endpoint in detail: [API reference](api.md)
- Build an agent on top: [For agents](for-agents.md)