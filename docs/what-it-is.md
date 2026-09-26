# What this is

## The problem

The web runs on documents, but the useful stuff in most organizations
lives in **databases** — facts: who employs whom, what a part costs,
which sensor reads what, who published which paper. Getting those facts
onto the web has always meant building **an API**. And that is where the
trouble starts:

- An API is a **product** someone has to build, document, secure,
  version, and keep alive. Most organizations never get to it. Their
  data stays locked up.
- An API is a **promise about tomorrow**: once other software builds on
  your field names and shapes, changing anything becomes expensive.
  So APIs freeze, or break their consumers, or both.
- An API is a **closed description**: whatever isn't written in the
  docs doesn't exist. Ask a running system "what do you actually
  contain?" and it has no answer.
- An API is **silent**: consumers must poll, scrape, or synchronize
  copies. The web of data runs on stale exports.
- And almost nothing a machine reads carries **evidence of who vouched
  for it**. Numbers circulate without provenance.

The result is a web where documents are easy to publish and data is
hard — the exact inverse of what the world now needs.

## The idea

This infrastructure is a service that sits in front of a **triple
store** (a database that stores facts as *subject — predicate — object*
statements) and serves it over the same open, boring protocols the web
itself runs on. No new protocol. No SDK. Nothing to install to read it.

It gives a store four properties that APIs and exports can't:

1. **Pageable, streamed access** — data is read in pages over plain
   HTTP, like turning pages rather than downloading a warehouse.
2. **Self-description** — the service answers "what's in here, what
   does it mean, how do I ask for it" from the *live* store, on every
   request. Description and data are the same thing; they cannot drift
   apart.
3. **Push** — subscribers are notified when data changes, per the W3C
   WebSub standard. The data comes to you; you don't hunt it.
4. **Verifiable provenance** — the self-description can be signed, and
   any signed claim can be checked: who issued it, when, until when,
   revoked or superseded.

Because every part of this is built from existing web standards (see
[Standards implemented](standards.md)), a publisher adopting it isn't
adopting a vendor — they're adopting the web.

## What it is *not*

- Not a database. It sits in front of any SPARQL 1.1 store; the store
  stays the source of truth and keeps its own query engine.
- Not an app. It is the layer that apps, dashboards, feeds, and
  integrations would otherwise each build (and each maintain) by hand.
- Not a crawler or scraper. It is how a publisher *serves* their own
  data, and how anyone *subscribes* to it.