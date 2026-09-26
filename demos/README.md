# Harborlight — multiple apps on the infrastructure

Three independent apps that communicate **only through the
infrastructure**. Each runs its own service, its own store, its own
hub, and speaks its own ontology. None of them imports another one,
calls another one's API by agreement, or shares a database. What they
share is the **intersection**: the `foaf:name` / `rdf:type` conventions,
and the open fragment surface that lets any app read what any other
app wrote.

```
                ┌──────────────────────┐
   Directory →  │ service :8484 + hub  │ → store :7878   (foaf + schema terms)
                └──────────────────────┘
                ┌──────────────────────┐
   Market    →  │ service :8486 + hub  │ → store :7879   (mkt: terms on schema:Product)
                └──────────────────────┘
                ┌──────────────────────┐
   Board     →  │ service :8487 + hub  │ → store :7880   (brd: terms)
                └──────────────────────┘
```

- **Directory** owns people and organizations. Its detail view shows a
  subject's facts **grouped by which app wrote them** — including
  Board posts, found with one fragment query against the Board's
  service.
- **Market** owns coffee lots (prices, harvest dates, certification).
  It resolves the org a lot was sourced from by querying the
  Directory's fragments over HTTP, and shows Board posts about each lot.
- **Board** owns posts only. It *mentions* people and *talks about*
  products it does not own — labels resolve live from the neighbors'
  stores, because everyone labels with `foaf:name`. New posts land on
  the feed instantly via the Board hub's SSE mirror.

Each service signs its manifest under its own `did:web` identity
(`:8484`, `:8486`, `:8487` — three issuers, three registries).

## Run

```sh
# from the repo root: the three services + three stores
docker compose up -d

# the four demo pages (three apps + the claims verifier)
pnpm install
pnpm --filter @demos/directory dev    # http://localhost:5173
pnpm --filter @demos/market dev       # http://localhost:5174
pnpm --filter @demos/board dev        # http://localhost:5175
pnpm --filter @demos/showcase dev     # http://localhost:5170 (claims verifier)
```

## The walkthrough

1. **Open Directory and Market side by side.** Pick Alice in the
   Directory: her facts carry a "written by Directory" badge — and the
   board badge if the Board mentioned her. Pick a lot in the Market:
   the sourcing org shows a name that was resolved from the
   Directory's store, live.
2. **Post on the Board** mentioning Alice about lot #7. No code
   changed anywhere. Reload nothing: the Board feed updates via its
   own hub; open the Directory and pick Alice again — the mention is
   there, found with one fragment query.
3. **The vocabulary moment.** Post with a brand-new predicate (edit
   the composer? or insert via `curl`): every app's manifest grows the
   new term, and the schema fingerprint changes — the schema-change
   event fires on `/topics/schema`.
4. **The trust moment.** Each service signs under its own DID. The
   claims verifier (showcase, :5170) verifies any of the three
   manifests — different issuers, different registries, same four
   gates.

## What the demo shows about the infrastructure

- Apps integrate through **data conventions**, not code contracts:
  shared `foaf:name` + open fragments = enough.
- Each app keeps full ownership of its own ontology and store; the
  service in front of each is the only thing the others see.
- The WebSub/SSE surface per service makes every app live without any
  of them polling any other.
- Nothing here is federated by hand: no webhook wiring, no ETL — the
  URIs point across services, and reading them is just HTTP.