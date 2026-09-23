# Quickstart

## Run the stack

```sh
git clone https://github.com/fab679/semantic-web
cd semantic-web
docker compose up --build -d
```

Two containers come up:

| Container | What | Where |
|---|---|---|
| `oxigraph` | the triple store (SPARQL 1.1 Protocol) | <http://localhost:7878> |
| `semantic-web` | this service | <http://localhost:8484> |

## First look (30 seconds)

Open the **Graph Explorer** at <http://localhost:8484/ui> — a single
embedded page that builds itself from the live manifest: browse classes
and descriptions, search patterns, preview topics, watch the live
change feed, and see health/metrics. It is a pure client over the API
(everything it shows is plain HTTP you can call yourself).

```sh
# What does the graph contain? (live, always current)
curl http://localhost:8484/

# Stream triples matching a pattern
curl "http://localhost:8484/fragments?predicate=http://schema.org/worksFor"

# The agent manifest (classes, descriptions, SHACL shapes, examples)
curl http://localhost:8484/manifest
```

## Put data in

Writes are token-gated (compose sets `demo-write-token`):

```sh
curl -X POST http://localhost:8484/admin/insert \
  -H "Authorization: Bearer demo-write-token" \
  -H "Content-Type: application/json" \
  -d '{"subject":"http://example.org/dave","predicate":"http://xmlns.com/foaf/0.1/name","object":"Dave"}'
```

Read it back immediately — there is no rebuild:

```sh
curl "http://localhost:8484/fragments?subject=http://example.org/dave"
```

## Watch it change in real time

Terminal 1 — run the demo subscriber:

```sh
docker compose --profile demo up -d demo-subscriber
```

Terminal 2 — subscribe and mutate:

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
# content distribution received; signature verified (sha256)
```

## Run it locally (no docker)

```sh
# Oxigraph separately (or any SPARQL 1.1 endpoint)
docker run -p 7878:7878 ghcr.io/oxigraph/oxigraph:0.5.10 serve --bind 0.0.0.0:7878

cargo run -p semweb
```

## What's inside

- **Rust service** (axum + tokio) — the whole HTTP surface
- **Oxigraph** — the store; any SPARQL 1.1 endpoint is a drop-in
- Environment-configured: `SEMWEB_WRITE_TOKEN`, `SEMWEB_SECRET_KEY`
  (encrypts subscriber secrets at rest), `SEMWEB_REPLICA_COUNT/INDEX`
  (sharding), `SEMWEB_SHACL_PATH`, and more — see
  [README](https://github.com/fab679/semantic-web#readme).