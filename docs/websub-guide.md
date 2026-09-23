# Real-time push (WebSub guide)

The graph pushes instead of making you poll. This guide covers the
practical parts; the normative reference is the
[W3C WebSub Recommendation](WebSub.md) this hub implements.

## The roles

| Role | Who | What it does |
|---|---|---|
| Publisher | the semantic-web service | owns the topics, signals changes |
| Hub | the semantic-web service | validates subscriptions, delivers content |
| Subscriber | **your** app/service/agent-side worker | receives signed pushes |

Publisher and hub being the same service is the simplest valid WebSub
topology. Your subscribers can run anywhere HTTP can reach.

## Topics

| Topic | Fires when | Content delivered |
|---|---|---|
| `/topics/data` | any instance data changed | every triple, as NDJSON |
| `/topics/schema` | a class or predicate appeared that wasn't in use before | the full schema surface JSON (with `schemaFingerprint`) |

## Subscribing

```sh
curl -X POST http://localhost:8000/hub \
  -d "hub.mode=subscribe" \
  -d "hub.topic=http://localhost:8000/topics/data" \
  -d "hub.callback=https://myapp.example/webhook/graph" \
  -d "hub.secret=my-hmac-secret" \
  -d "hub.lease_seconds=86400"
```

What happens next (all per the spec):

1. `202 Accepted` immediately — verification is asynchronous.
2. The hub GETs your callback with a `hub.challenge`; your endpoint must
   echo it back with 2xx. (See `crates/semweb/src/bin/demo-subscriber.rs`
   for a minimal implementation — it also shows the spec's anti-XSS
   rules: safe media type + `nosniff`.)
3. On every change, the hub POSTs the **full topic content** to your
   callback with a `Link` header (`rel=self` + `rel=hub`) and, when you
   registered a secret, an `X-Hub-Signature: sha256=...` header.
4. Answer `2xx` fast (the ack means "received", not "processed"). Return
   `410 Gone` to delete the subscription. Failures retry on a
   1s/5s/15s schedule; the subscription survives until the lease ends.
5. Leases expire (never perpetual): re-request before
   `hub.lease_seconds` elapses — the same `subscribe` call renews.

## Subscriber checklist (spec §8.2)

- Use an **unguessable callback URL** (a capability URL) and HTTPS when
  registering secrets.
- **Validate the signature**: recompute HMAC-SHA256 of the raw body with
  your secret; discard mismatches locally (still 2xx).
- Treat notifications as hints: the body is the full topic content, so
  reprocessing is idempotent.

## Two transports, one bus

| Transport | Endpoint | For |
|---|---|---|
| WebSub | `POST /hub` | durable backend services — can be offline between events |
| SSE | `GET /events?topic=/topics/data` | live sessions (browser tabs, agent runs) |

Both fire from the same in-process publish; pick per consumer. SSE events
carry only `{topic, event_id}` — refetch the topic for content, keeping
the stream tiny.

## Durability guarantees

- Subscriptions persist in the store — restart the service and they're
  reloaded; expired leases are dropped per spec.
- Every delivery is logged before it is queued and acked when finished:
  a crash mid-flight redelivers on startup (**at-least-once** — a
  duplicate is possible across a crash/retry window; make handling
  idempotent).
- Secrets are AES-256-GCM encrypted at rest when `SEMWEB_SECRET_KEY` is
  set (compose sets one).
- With multiple replicas, each subscription is owned by exactly one
  shard (consistent hash) — scaling out needs no extra coordination.

## Live demo inside the stack

```sh
docker compose --profile demo up -d demo-subscriber
# subscribe (above), then insert something, then:
docker compose logs -f demo-subscriber
```