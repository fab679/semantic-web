# Demo app: Semblr, the microblog on a knowledge graph

A real, small social app built entirely on this service — the fastest way
to *feel* what the infrastructure can do. One static page (no build step,
no backend) that seeds a small social graph, then:

- **Feed** — one SPARQL query (`POST /sparql`): posts by you + people you
  follow, newest first.
- **Follow / like / post** — plain inserts through the token-gated write
  path; the pulse bar shows every live SSE event with its `event_id`.
- **People you may know** — multi-hop `foaf:knows` SPARQL.
- **Profile cards + "triples" button** — `GET /fragments?subject=…`; any
  card's raw JSON-LD lines are one click away.
- **"Ship a feature →"** — inserts a brand-new predicate
  (`sem:reaction`); the manifest fingerprint changes and the cards learn
  to render reactions **with no redeploy**. The core "living API" claim,
  demonstrated in five seconds.
- **Agent panel** — the agent is *a node in the same graph*
  (`http://semblr.dev/u/agent`) using the same MCP tools and write path
  as humans: answers questions grounded in the graph, digests the
  timeline as a post of its own, suggests follows from real mutual-count
  data, and drafts bios that the UI then learns live. Governance is
  visible: revoke its write token and it becomes read-only.

## Run it

```sh
docker compose up --build -d
python3 -m http.server 8080 -d examples/microblog
open http://localhost:8080
```

Full walkthrough (including the agent's role and its Together AI key):
[`examples/microblog/README.md`](https://github.com/fab679/semantic-web/blob/master/examples/microblog/README.md).

## Why this demo matters

Every feature above is *not* bespoke app code talking to a bespoke
backend — each one is a thin client over the three primitives
(read / describe / push) plus the MCP surface. Swap the domain vocabulary
from "people and posts" to anything else and the app shape survives:
that is what "the description IS the data" buys you.