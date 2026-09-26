# Showcase demos

Three small React apps (Vite + Tailwind + shadcn/ui) that verify the
infrastructure's documented claims live, against a running service.
They exist for the room: every number on screen arrived over plain
HTTP moments ago.

| Demo | Tab | Claim it verifies |
|---|---|---|
| **Atlas** | Read | It describes itself (manifest regenerated live, fingerprint, shapes) · Data comes in pages, streamed (fragment workbench with cursor pagination) |
| **Pulse** | Watch | It pushes (SSE change wire + publish form; watch the event land) |
| **Verity** | Verify | Claims can be proven (four-gate verification, tamper toggle, attestations with expiry) |

## Run

```sh
# 1. the infrastructure
docker compose up -d          # from the repo root; service on :8484

# 2. the demos
pnpm install
pnpm dev                      # → http://localhost:5173
```

The service URL defaults to `http://localhost:8484`; override at build
time with `VITE_SEMWEB_URL`.

## The demo walkthrough

1. **Read** — the manifest says what the graph contains, regenerated on
   every fetch; open a class for its description, SHACL shapes and a
   runnable example query. Then the fragment workbench: fill any of the
   three positions, stream the matching facts, follow the cursor. The
   stream is the raw response body — the app adds nothing.
2. **Watch** — the change wire listens on `/events`. Publish a fact with
   the form (compose sets `SEMWEB_WRITE_TOKEN: demo-write-token`);
   watch the event arrive within milliseconds. Insert it twice: the
   second write changes nothing and pushes nothing.
3. **Verify** — fetch the signed manifest, see the four gates answer.
   Flip *tamper with one byte* and the signature gate refuses. Issue an
   attestation, then an already-expired one — the freshness gate
   answers differently. The publisher's key is at `/.well-known/did.json`.

The tamper test needs a signing-enabled service (the compose file ships
a demo signing key — see docs/trust.md). Without one, Verity reports
that the surface is unsigned, which is itself the documented behavior.