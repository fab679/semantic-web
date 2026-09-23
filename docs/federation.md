# Federation: decentralized WebSub, for real

WebSub was designed (as PubSubHubbub, 2009) to be **decentralized**: a
three-party ecosystem where **publishers** own content, **hubs** do the
infrastructure heavy lifting, and **subscribers** consume. Nobody needs
a central registry — a publisher declares which hub(s) it trusts right
in its HTTP headers, and subscribers discover that via the topic's Link
headers (spec §4).

This service implements all three roles and can participate in that
topology in every direction.

## The three topologies

### 1. Self-hosted hub (the default — "centralization for the sake of decentralization")

The simplest valid topology: publisher and hub are one service. This is
what `docker compose up` gives you. A small service runs its **own**
high-availability hub instead of depending on a public one —
subscribers push the load to you, and you are immune to third-party hub
outages and policy changes.

Scaling that hub is built in: replicas share one store and each delivers
only its consistent-hash shard (`SEMWEB_REPLICA_COUNT/INDEX`), with a
durable delivery log — the "high-availability hub" problem from the
centralization critique, solved without running an infrastructure
monopoly.

### 2. Open hub (serve third-party publishers)

Set `SEMWEB_OPEN_HUB=1` and the hub accepts **third-party topics**: any
publisher can subscribe followers to any `http(s)` topic URL through
your hub, and notify you on updates:

```sh
# a third-party publisher subscribes its feed to your hub
curl -X POST http://localhost:8484/hub \
  -d "hub.mode=subscribe" \
  -d "hub.topic=https://other.example/feed.xml" \
  -d "hub.callback=https://other.example/subscriber"

# the publisher updates its feed and notifies the hub (§6)
curl -X POST http://localhost:8484/hub \
  -d "hub.mode=publish" \
  -d "hub.url=https://other.example/feed.xml"
```

Per spec §7, the hub then **fetches the topic URL at publish time** and
distributes exactly what the publisher served — Content-Type preserved,
Link headers pointing at the publisher's canonical URL, capped at 16
MiB. Verification, leases, HMAC signing, retries, the durable delivery
log and shard ownership all apply unchanged.

This is "anyone can run a hub" from your text, as a config flag — with
the same policies a public hub would apply (spec §5.1: "Any hub MAY
implement its own policies on who can use it").

### 3. Federated instances (two semantic-web services cross-subscribing)

The decentralized endgame: independent publishers use independent hubs,
and subscribers pick whichever hub each publisher advertises.

```
 semantic-web A (store A)          semantic-web B (store B)
 topics: /topics/data ...          topics: /topics/data ...
 hub: http://a:8484/hub            hub: http://b:8485/hub
        │                                 │
        └── A's subscriber follows B's topic ──┘
```

Because discovery is standard Link headers, subscribing across
instances needs nothing special:

1. Discover B's hub: `curl -I http://b:8485/topics/data` →
   `Link: <http://b:8485/topics/data>; rel="self", <http://b:8485/hub>; rel="hub"`
2. Subscribe at **B's** hub (the one the publisher trusts), using B's
   `rel=self` URL as `hub.topic`.
3. From then on B's hub pushes to your callback — signed, retried,
   leased — regardless of which instance you run.

## Multi-hub advertisement and notify (fault tolerance, §4)

A publisher "MAY advertise more than one hub" — if one fails to
propagate, another still will. Configure external hubs and the service
will:

- advertise **all** of them in every topic's `Link` headers, and
- **notify each** on every mutation (`hub.mode=publish` POSTs, §6 — the
  common convention; the publisher/hub pairing is unspecified, and this
  is what public hubs accept).

```sh
# ours + a public hub for redundancy
SEMWEB_HUB_URLS="https://superfeedr.com/hub" docker compose up -d
```

Subscribers then choose: subscribe at ours (low latency, full
self-description) or at the public hub (third-party availability) — or
both, which is exactly the redundancy the spec intends.

## One operational caveat: network reachability

The hub fetches the topic URL **from its own network position** — a
URL that works in the publisher's browser may be unreachable from the
hub (loopback, NAT, firewalls). This is inherent to the architecture,
not a bug: a third-party publisher must expose its topic URL somewhere
the hub can reach. In the demo compose stack, use the in-network URL
(`http://semantic-web:8000/manifest`), not the host-mapped one.

## The honest limits

- Our canonical topics live at our own URLs; a third-party publisher
  using our open hub keeps serving its own content — we fetch at
  publish time (§7 allows nothing less: the hub MUST send the full
  contents of the topic URL; diffs are only permitted for Atom/RSS,
  which is why JSON/NDJSON topics get full bodies).
- The de-facto-centralization critique in the ecosystem is real: running
  an always-on hub is the engineering cost. This project's answer is that
  the hub is ~10 MB of Rust next to your data, sharded across replicas —
  you already run it; it isn't somebody else's chokepoint.

## Configuration

| Variable | Effect |
|---|---|
| `SEMWEB_OPEN_HUB` | accept third-party topics (default off — §5.1 policy) |
| `SEMWEB_HUB_URLS` | comma-separated external hubs to advertise + notify (§4/§6) |
| `SEMWEB_PUBLIC_URL` | the rel=self/rel=hub base for your own topics |