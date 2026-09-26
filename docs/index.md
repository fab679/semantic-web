# Streaming Semantic Fragments

**A standard way to put live data on the web.**

Databases hold the world's useful facts, but the web only knows how to
serve *documents* — so data gets trapped behind APIs that must be
built, documented, and kept alive, or exported as copies that go stale
the moment they are written.

This infrastructure closes that gap: it turns a triple store into a
web surface with four properties no API or export can offer.

1. **Data comes in pages, streamed** — any pattern of facts, over
   plain HTTP, with pagination. Nothing to install to read it.
2. **It describes itself** — the schema, the meanings, the counts, the
   shapes, and runnable examples are generated from the live store on
   every request. Nothing to build; nothing that can go stale.
3. **It pushes** — a W3C WebSub hub notifies subscribers when data
   changes. No polling; no scraping.
4. **Claims can be proven** — the self-description can be signed, and
   any signed claim can be verified against four gates: shape,
   signature, trusted issuer, freshness.

Everything rides on existing web standards — HTTP, JSON-LD, WebSub,
Verifiable Credentials — no new protocol, no translation layer in
front of the store.

## Where to go next

- **[What this is](what-it-is.md)** — the problem with data on today's
  web, and the idea in one page
- **[How it works](how-it-works.md)** — the four mechanisms, each with
  an everyday analogy
- **[What it enables](what-it-enables.md)** — what becomes cheap:
  publishing, freshness, direct distribution, verifiable information
- **[Trust layer](trust.md)** — claims that carry their own evidence
- **[Standards implemented](standards.md)** — every spec this rides on,
  linked
- **[Reference](reference.md)** — the complete HTTP surface and
  configuration
- **[Operations](operations.md)** — running, scaling, security,
  observability
- **[WebSub specification](WebSub.md)** — the W3C Recommendation this
  hub implements, full text