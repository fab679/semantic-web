# Streaming Semantic Fragments

**A standard way to put live data on the web.**

This is infrastructure: a small Rust service that turns a triple store
(RDF) into a web surface that any person, browser, or program can read —
over nothing but plain HTTP.

Four things make it different from "another API":

1. **Data comes in pages, streamed.** Triple Pattern Fragments: you ask
   for a pattern (any subject / predicate / object combination) and the
   answers stream back as NDJSON-LD, one line per fact, with cursor
   pagination. No query engine required on your side.
2. **It describes itself.** The schema, cardinalities, meanings, shapes
   and runnable examples are regenerated from the live store on every
   request. There is no build step and nothing to go stale — the
   interface and the data can never drift apart.
3. **It pushes.** A spec-compliant W3C WebSub hub ([the
   Recommendation](docs/WebSub.md) is included in this repo) notifies
   subscribers the moment data changes. No polling, no scraping.
4. **Claims can be proven.** The manifest can be cryptographically
   signed (Ed25519, Data Integrity, `did:web`) and any signed document
   can be verified against a local trust registry — who vouched, when,
   until when, revoked or superseded.

Everything rides on existing web standards. No new protocol, no
translation layer in front of the store: SPARQL 1.1 remains the
execution plane, and the service is stateless in front of it.

## Read the documentation

The docs explain the infrastructure for people — what it is, how it
works, what it makes possible:

- [What this is](docs/what-it-is.md) — the problem and the idea
- [How it works](docs/how-it-works.md) — the four mechanisms, in plain language
- [What it enables](docs/what-it-enables.md) — what becomes possible when data works like the web
- [Standards implemented](docs/standards.md) — every spec this service rides on
- [Trust layer](docs/trust.md) — verifiable provenance, in plain language
- [Reference](docs/reference.md) — the HTTP surface and configuration
- [Operations](docs/operations.md) — running, scaling, security, observability

## Run it

```sh
docker compose up --build
```

That starts a SPARQL 1.1 store (Oxigraph) on :7878 and the service on
:8484. Then:

```sh
curl http://localhost:8484/                   # the service describing itself
curl http://localhost:8484/manifest           # the self-description for programs
curl "http://localhost:8484/fragments?predicate=http://schema.org/worksFor"
open http://localhost:8484/ui                 # the built-in graph explorer
```

Local development without docker:

```sh
cargo run -p semweb
```

Configuration is environment-only (see
[docs/reference.md](docs/reference.md)); `cargo test` runs the unit
suite and `scripts/e2e.sh` runs the end-to-end conformance battery in
docker — the same battery CI gates on.

## Layout

Coding convention: one concern per small file (nothing over ~300 lines).

```
crates/semweb/             the service
  src/main.rs              wiring + background tasks
  src/state.rs             environment configuration
  src/jsonld.rs            triples -> compacted JSON-LD lines
  src/context.rs           live @context + prefix compaction
  src/trust/               signing + verification + trust registry
  src/api/                 the HTTP surface (one file per endpoint group)
  src/hub/                 the WebSub hub (verification, delivery,
                           persistence, sharding, crypto, rate limits)
  src/store/               SPARQL 1.1 Protocol client, fragments,
                           cardinalities, durability
  assets/ui/index.html     the embedded Graph Explorer page
  src/bin/demo-subscriber.rs   reference WebSub subscriber (used by the
                               e2e battery as the conformant consumer)
  sample_data.ttl          example seed data for SEMWEB_SEED_PATH
  shapes.ttl               example SHACL shapes for SEMWEB_SHACL_PATH
scripts/e2e.sh             end-to-end conformance battery
bench/                     latency harness
docs/                      documentation
```