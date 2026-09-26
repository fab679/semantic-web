# How it works

Four mechanisms. Each one is small; together they change what a
publisher can offer and what a reader can expect.

---

## 1. Data comes in pages — fragments

A triple store holds facts as triples: *subject — predicate — object*
(`Alice — works for — Acme`). Asking for "everything that matches this
shape" is a **triple pattern**: fill in any of the three positions,
leave the rest as wildcards.

`GET /fragments` answers with a **fragment**: the matching facts
streamed as one line of JSON per fact (NDJSON-LD), with a cursor to
fetch the next page. A reader needs nothing but HTTP — no query engine,
no library, no bulk download.

```
curl "http://localhost:8484/fragments?subject=http://example.org/alice"

{"@id":"http://example.org/alice","name":"Alice"}
{"@id":"http://example.org/alice","employer":{"@id":"http://example.org/acme"}}
```

**The analogy:** this is how the web serves *documents* — one GET, one
page, follow the cursor like a "next" link. It brings the same
property to *data*.

---

## 2. It describes itself — the manifest

`GET /manifest` is regenerated from the live store on **every** request.
It answers the questions a reader actually has:

- *What kinds of things are in here?* — every class, with live counts
- *What relationships exist?* — every predicate, with live counts
- *What do these terms mean?* — human descriptions read from the data
- *What's required of each thing?* — SHACL shapes, if the store carries them
- *How do I ask?* — a runnable example query per class, and the list of
  prefixes
- *Did the shape of the data change?* — a schema fingerprint to diff

Because it is generated, never written, there is no documentation that
can rot. The description is a *view of the data*, like the label
printed on the jar — always for the jar you're holding.

---

## 3. It pushes — WebSub

Polling is how data integration dies: consumers re-fetch everything on
a timer, publishers get scraped, both pay for the privilege.

This service ships a **WebSub hub** — the W3C Recommendation
([full spec](WebSub.md)) for subscriptions on the open web:

- A subscriber registers a callback URL at `POST /hub` (the same form
  encoding as an HTML form — no JSON ceremony).
- The hub verifies the subscriber actually wants the subscription by
  challenging their endpoint.
- When data changes, the hub **pushes** the full content to every
  subscriber, HMAC-signed with a per-subscription secret so the
  subscriber can prove the delivery is genuine.
- Subscriptions have leases that expire; a subscriber can leave with a
  `410 Gone`; the hub persists subscriptions across restarts and across
  replicas.

**The analogy:** a magazine subscription. The kiosk (polling) makes
you walk there to check; the subscription (WebSub) makes the new issue
arrive. RSS did this for articles in 1999; WebSub does it for data.

---

## 4. Claims can be proven — the trust layer

A manifest (or any signed claim) can carry a **cryptographic proof**:
an Ed25519 signature over the document, made by the publisher under
their web identity (`did:web` — the same convention used by `https`
certificates: identity derived from the web location).

The public key is published at a conventional URL
(`/.well-known/did.json`), so verifying requires nothing but HTTP and a
key you've decided to trust.

A verifier checks four things, and gets the answer to each:

1. **Shape** — is there a well-formed proof at all?
2. **Signature** — does the content match the signature, byte for byte?
3. **Issuer** — is the signer someone *this reader* decided to trust?
4. **Freshness** — was it issued inside its validity window, and has it
   been revoked or superseded since?

The point is not "secure" as a badge. The point is that **a claim
arrives with its own evidence** — who vouches, until when — instead of
circulating naked like everything else on the web. (Transport security
took one decade to go from novelty to assumption; claim provenance is
that same kind of default waiting to happen.)

---

## What holds it together

All four mechanisms ride on standards the web already speaks — HTTP,
JSON-LD, WebSub, Data Integrity — so every piece composes with the rest
of the web by construction: a fragment URL is a URL; a topic is a URL
with Link headers; the manifest is JSON-LD; the DID document is at its
conventional address. Nothing here is an island.