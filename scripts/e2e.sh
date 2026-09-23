#!/usr/bin/env bash
# End-to-end battery for the whole stack (docker compose based).
# Exit code 0 = every mechanism verified. Used locally and in CI.
set -euo pipefail

cd "$(dirname "$0")/.."
BASE=${BASE:-http://localhost:8484}
TOKEN="demo-write-token"
FAIL=0

say() { printf "\n== %s\n" "$1"; }
check() { # name, condition_exit_code
  if [ "$2" -eq 0 ]; then echo "PASS: $1"; else echo "FAIL: $1"; FAIL=1; fi
}

say "bring up the stack"
docker compose --profile demo down -v --remove-orphans >/dev/null 2>&1 || true
docker compose --profile demo up --build -d >/dev/null
for i in $(seq 1 30); do curl -sf "$BASE/health" >/dev/null && break; sleep 1; done
curl -sf "$BASE/health" >/dev/null; check "health" $?

# Seed + shapes load in a background task with retries; wait until the
# manifest actually reflects data before asserting on it.
for i in $(seq 1 30); do
  if curl -sf "$BASE/manifest" | python3 -c 'import json,sys; d=json.load(sys.stdin); sys.exit(0 if len(d["classes"])>0 else 1)' 2>/dev/null; then
    break
  fi
  sleep 1
done

say "self-description + discovery headers"
curl -sf "$BASE/" | grep -q '"classes"'; check "self-description" $?
curl -sI "$BASE/topics/data" | grep -qi 'link:.*rel="self"'; check "discovery link headers" $?

say "fragments: cursor pagination walks without overlap"
P1=$(curl -sf "$BASE/fragments?limit=2")
AFTER=$(echo "$P1" | tail -1 | python3 -c 'import json,sys; print(json.load(sys.stdin).get("after",""))')
if [ -n "$AFTER" ]; then check "cursor in control line" 0; else check "cursor in control line" 1; fi
P2=$(curl -sf "$BASE/fragments?limit=2&after=$AFTER")
# the page boundary: page 2 must start AFTER page 1's last data line
LAST1=$(echo "$P1" | head -2 | tail -1)
FIRST2=$(echo "$P2" | head -1)
if [ "$LAST1" != "$FIRST2" ]; then check "cursor advances past page 1" 0; else check "cursor advances past page 1" 1; fi

say "manifest: fingerprint, cardinalities, SHACL shapes"
if curl -sf "$BASE/manifest" | python3 -c '
import json,sys
d=json.load(sys.stdin)
assert d["schemaFingerprint"].startswith("sha256:")
assert len(d["classes"]) > 0
assert any(c.get("shapes") for c in d["classes"])
assert any(c.get("description") for c in d["classes"])
'; then check "manifest fields" 0; else check "manifest fields" 1; fi

say "MCP: initialize / list / read"
if curl -sf -X POST "$BASE/mcp" -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' | grep -q protocolVersion; then check "mcp initialize" 0; else check "mcp initialize" 1; fi
if curl -sf -X POST "$BASE/mcp" -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":2,"method":"resources/read","params":{"uri":"manifest://semantic-web/current"}}' \
  | python3 -c 'import json,sys; d=json.load(sys.stdin)["result"]["contents"][0]; assert d["uri"].startswith("manifest://")'; then check "mcp read" 0; else check "mcp read" 1; fi

say "MCP tools: list / call"
if curl -sf -X POST "$BASE/mcp" -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":5,"method":"tools/list","params":{}}' \
  | python3 -c 'import json,sys; ts=json.load(sys.stdin)["result"]["tools"]; assert len(ts)==6'; then check "mcp tools/list (6 tools)" 0; else check "mcp tools/list (6 tools)" 1; fi
if curl -sf -X POST "$BASE/mcp" -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"search_graph","arguments":{"predicate":"http://schema.org/worksFor","limit":2}}}' \
  | python3 -c 'import json,sys; t=json.load(sys.stdin)["result"]["content"][0]["text"]; assert "worksFor" in t or "schema:worksFor" in t'; then check "mcp tools/call search_graph" 0; else check "mcp tools/call search_graph" 1; fi

say "write auth + write path"
CODE=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$BASE/admin/insert" -H 'Content-Type: application/json' \
  -d '{"subject":"http://example.org/e2e","predicate":"http://xmlns.com/foaf/0.1/name","object":"E2E"}')
if [ "$CODE" = "401" ]; then check "write without token -> 401" 0; else check "write without token -> 401" 1; fi
curl -sf -X POST "$BASE/admin/insert" -H "Authorization: Bearer demo-write-token" -H 'Content-Type: application/json' \
  -d '{"subject":"http://example.org/e2e","predicate":"http://xmlns.com/foaf/0.1/name","object":"E2E"}' \
  | grep -q event_id; check "write with token" $?

say "WebSub: subscribe -> signed full-content delivery -> unsubscribe"
docker compose --profile demo up -d demo-subscriber >/dev/null
curl -sf -X POST "$BASE/hub" -d "hub.mode=subscribe" -d "hub.topic=http://localhost:8484/topics/data" \
  -d "hub.callback=http://demo-subscriber:9000/callback" -d "hub.secret=demo" >/dev/null
sleep 2
count_verified() {
  docker compose logs demo-subscriber 2>/dev/null | python3 -c \
    'import sys; print(sum(1 for l in sys.stdin if "signature verified" in l))'
}
BEFORE=$(count_verified)
INSERT_RESP=$(curl -sf -X POST "$BASE/admin/insert" -H "Authorization: Bearer demo-write-token" -H 'Content-Type: application/json' \
  -d '{"subject":"http://example.org/e2e","predicate":"http://xmlns.com/foaf/0.1/knows","object":"http://example.org/acme"}')
NOTIFIED=$(echo "$INSERT_RESP" | python3 -c 'import json,sys; print(json.load(sys.stdin)["data_subscribers_notified"])')
echo "insert: data_subscribers_notified=$NOTIFIED"
# delivery is async (bounded queue + worker): poll up to 12s
AFTER_N=$BEFORE
for i in $(seq 1 12); do
  AFTER_N=$(count_verified)
  if [ "$AFTER_N" -gt "$BEFORE" ]; then break; fi
  sleep 1
done
if [ "$AFTER_N" -gt "$BEFORE" ]; then check "signed delivery arrived" 0; else check "signed delivery arrived" 1; fi
curl -sf -X POST "$BASE/hub" -d "hub.mode=unsubscribe" -d "hub.topic=/topics/data" \
  -d "hub.callback=http://demo-subscriber:9000/callback" >/dev/null
if [ $? -eq 0 ]; then check "unsubscribe" 0; else check "unsubscribe" 1; fi

say "SSE /events"
(timeout 12 curl -sN "$BASE/events?topic=/topics/data" > /tmp/semweb-e2e-sse.log) &
sleep 2
curl -sf -X POST "$BASE/admin/insert" -H "Authorization: Bearer demo-write-token" -H 'Content-Type: application/json' \
  -d '{"subject":"http://example.org/e2e-sse","predicate":"http://xmlns.com/foaf/0.1/name","object":"E2E"}' >/dev/null
sleep 2
grep -q 'data: {"topic":"/topics/data"' /tmp/semweb-e2e-sse.log; check "sse event delivered" $?

say "metrics"
curl -sf "$BASE/metrics" | grep -q semweb_deliveries_total; check "metrics exposed" $?

say "teardown"
if [ "$FAIL" -ne 0 ]; then
  echo "--- debug: subscriber log"; docker compose logs demo-subscriber 2>&1 | grep -vE "^\s*$" | tail -10
  echo "--- debug: hub log"; docker compose logs semantic-web 2>&1 | grep -iE "verif|delivery|subscri" | tail -12
fi
docker compose --profile demo down -v --remove-orphans >/dev/null 2>&1 || true

if [ "$FAIL" -ne 0 ]; then echo "E2E: FAILURES PRESENT"; exit 1; fi
echo "E2E: all checks passed"