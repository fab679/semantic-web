# Semblr — a microblog on a knowledge graph

A real, small social app built **entirely on the semantic-web service**.
No backend of its own, no build step: one static page plus vanilla JS,
talking to the same API humans and agents share.

```sh
# 1. the stack
docker compose up --build -d

# 2. serve the app (any static server works — CORS is enabled on the service)
python3 -m http.server 8080 -d examples/microblog
open http://localhost:8080
```

The app seeds itself on first load (real inserts — watch the pulse bar).

Verify the app's API flows headlessly (no browser needed):

```sh
python3 examples/microblog/smoke.py        # seed, feed, likes, suggestions,
                                           # ship-a-feature, agent node — exit 0 = all good
```

## What each feature actually is

| App feature | The infrastructure underneath |
|---|---|
| Feed | one SPARQL query over `POST /sparql`: posts by you + people you follow, `ORDER BY DESC(date)` |
| Like | an insert on `sem:likes`; counts are a live `GROUP BY` |
| Follow | `foaf:knows` insert |
| People you may know | multi-hop SPARQL (`foaf:knows` chains + `FILTER NOT EXISTS`) |
| Live updates | SSE `GET /events?topic=/topics/data` — every insert anywhere triggers a refetch; the pulse bar shows the `event_id` trail |
| Profile card | `GET /fragments?subject=…` — click **triples** on any card to see the raw JSON-LD lines |
| "Ship a feature →" | inserts a brand-new predicate (`sem:reaction`) — the manifest fingerprint changes and the cards learn to render reactions **with no redeploy** |
| Schema banner | the app diffs `GET /manifest` predicates on every change event; anything unknown is learned live |
| Agent panel | `POST /mcp` tools + Together AI, posting through the same token-gated write path as humans |

## The agent alongside humans

The agent is **a node in the same graph** (`http://semblr.dev/u/agent`, a
`foaf:Person`) and uses the exact same surface as humans — no special
agent APIs:

- **Free chat**: "who knows Carol?", "who liked the most posts?" — answered
  with `sparql_query`/`search_graph` on URIs it actually saw; tool calls
  shown inline.
- **Digest the timeline**: reads recent posts via SPARQL, writes a summary
  post *as itself* (4 `insert_triple` calls). Its post appears in the feed
  like anyone's — because in the graph, it is.
- **Suggest follows**: grounded in actual mutual-follow counts.
- **Write my bio**: drafts a bio from your posts; **Apply** writes it as
  `schema:description` — a predicate the feed/profile UI then learns live.
- **Governance is visible**: the agent carries the write token; revoke it
  (settings field) and the agent becomes read-only. Humans decide what
  agents may write.

Needs a [Together AI](https://api.together.ai) key (entered in the panel,
kept in `localStorage` — nothing is sent anywhere but Together and your
service). Without a key the app is fully usable; the agent panel simply
stays offline.

## Notes

- Writes use the compose demo token. A production app fronts
  `/admin/insert` with real auth (per-user credentials), which is the one
  piece this demo intentionally skips.
- Point `Service URL` at another instance (`http://localhost:8485` after
  `docker compose --profile scale up`) — the app works unchanged. That is
  the federation story in one line.
- Pretty URIs: run the stack with
  `SEMWEB_EXTRA_PREFIXES="sem=http://semblr.dev/ns#"` to get `sem:likes`-style
  compaction in `/manifest`, `/context.jsonld` and the triples view.