#!/usr/bin/env python3
"""
Bench harness for the semantic-web service.

Measures, against a running stack (docker compose, service :8484):
  1. load    bulk-load N triples via the Graph Store Protocol
  2. read    fragment latency (p50/p95/max) for three pattern shapes,
             self-description cost (GET /, GET /manifest, SPARQL COUNT)
  3. fanout  one insert -> K in-network subscribers (per-subscriber
             delivery latency, via the demo-subscriber /stats endpoint)
  4. crash   pending durable-log entry -> redelivery after restart

Usage:
    python bench.py load  --triples 100000
    python bench.py read
    python bench.py fanout --fanout 50
    python bench.py crash
Env: SEMWEB_URL (default http://localhost:8484),
     SEMWEB_SPARQL_HOST (default http://localhost:7878).
"""

import json
import os
import subprocess
import sys
import tempfile
import time

import requests

BASE = os.environ.get("SEMWEB_URL", "http://localhost:8484")
TOKEN = os.environ.get("SEMWEB_WRITE_TOKEN", "demo-write-token")
STORE = os.environ.get("SEMWEB_SPARQL_HOST", "http://localhost:7878")
SUBSCRIBER = os.environ.get("SEMWEB_BENCH_SUBSCRIBER", "http://demo-subscriber:9000")
ROUNDS = 60


def pct(xs, p):
    xs = sorted(xs)
    return xs[max(0, min(len(xs) - 1, int(round(p / 100 * (len(xs) - 1)))))] if xs else float("nan")


def fmt(label, lat3):
    if lat3 is None:
        return f"  {label:<24}  FAILED"
    return (f"  {label:<24}  p50 {lat3[0]:7.1f} ms   p95 {lat3[1]:7.1f} ms"
            f"   max {lat3[2]:8.1f} ms")


def stats_deliveries() -> dict:
    """Delivery timestamps from the in-network demo-subscriber bench
    endpoint, polled through docker exec (host cannot reach it directly)."""
    out = subprocess.run(
        ["docker", "compose", "exec", "-T", "demo-subscriber",
         "curl", "-sf", "http://127.0.0.1:9000/stats"],
        capture_output=True, text=True)
    try:
        return json.loads(out.stdout)["deliveries"]
    except (json.JSONDecodeError, KeyError):
        return {}


def cmd_load():
    n = int(sys.argv[sys.argv.index("--triples") + 1]) if "--triples" in sys.argv else 100_000
    print(f"[load] generating {n} triples …", flush=True)
    people = n // 4
    chunks = ["@prefix foaf: <http://xmlns.com/foaf/0.1/> .\n"
              "@prefix schema: <http://schema.org/> .\n"
              "@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n"]
    for i in range(people):
        p = f"http://bench.example/person/{i}"
        chunks.append(f'<{p}> a foaf:Person ; foaf:name "Person {i}" .\n'
                      f'<{p}> schema:worksFor <http://bench.example/org/{i % 500}> .\n'
                      f'<{p}> foaf:knows <http://bench.example/person/{(i + 1) % people}> .\n')
        chunks.append(f'<http://bench.example/org/{i % 500}> schema:foundingDate '
                      f'"20{i % 50:02d}-01-01"^^xsd:date .\n')
    with tempfile.NamedTemporaryFile("w", suffix=".ttl", delete=False) as f:
        f.write("".join(chunks))
        path = f.name
    with open(path, "rb") as f:
        t0 = time.perf_counter()
        r = requests.post(STORE + "/store", params={"default": None}, data=f.read(),
                          headers={"Content-Type": "text/turtle"}, timeout=1800)
        assert r.status_code < 300, f"load failed: {r.status_code}"
    os.unlink(path)
    secs = time.perf_counter() - t0
    print(f"[load] {n} triples in {secs:.1f}s  ({n / secs:,.0f} triples/s via GSP)", flush=True)


def cmd_read():
    patterns = {
        "paged full scan (100)": BASE + "/fragments?limit=100",
        "predicate-bound": BASE + "/fragments?predicate=http://schema.org/worksFor&limit=100",
        "subject-bound": BASE + "/fragments?subject=http://bench.example/person/42&limit=100",
    }
    for name, url in patterns.items():
        lat = []
        for _ in range(ROUNDS):
            t0 = time.perf_counter()
            requests.get(url, timeout=60).content
            lat.append((time.perf_counter() - t0) * 1000)
        print(fmt(name, (pct(lat, 50), pct(lat, 95), lat[-1])), flush=True)
    urls = {
        "GET /": BASE + "/",
        "GET /manifest": BASE + "/manifest",
        "SPARQL COUNT": STORE + "/query?query=SELECT%20%28COUNT%28%2A%29%20AS%20%3Fc%29%20WHERE%20%7B%20%3Fs%20%3Fp%20%3Fo%20%7D",
    }
    for name, url in urls.items():
        lat = []
        for _ in range(20):
            t0 = time.perf_counter()
            requests.get(url, timeout=60).content
            lat.append((time.perf_counter() - t0) * 1000)
        print(fmt(name, (pct(lat, 50), pct(lat, 95), lat[-1])), flush=True)


def subscribe_k(k, topic="http://localhost:8484/topics/data", suffix="cb", baseline=None):
    if baseline is None:
        baseline = sum(requests.get(BASE + "/hub", timeout=10).json()["subscriptions"].values())
    for i in range(k):
        requests.post(BASE + "/hub", data={
            "hub.mode": "subscribe", "hub.topic": topic,
            "hub.callback": f"{SUBSCRIBER}/cb/{suffix}-{i}", "hub.secret": "demo",
        }, timeout=10)
    # wait until the k callbacks of THIS run are verified: capture the
    # baseline before subscribing, poll until total active >= baseline + k
    deadline = time.time() + 60
    while time.time() < deadline:
        subs = requests.get(BASE + "/hub", timeout=10).json()["subscriptions"]
        if sum(subs.values()) >= k + baseline:
            break
        time.sleep(0.5)
    time.sleep(1)


def cmd_fanout():
    k = int(sys.argv[sys.argv.index("--fanout") + 1]) if "--fanout" in sys.argv else 50
    print(f"subscribing {k} in-network callbacks …", flush=True)
    subscribe_k(k)
    stats_deliveries = stats_deliveries_snapshot()
    t0_ms = int(time.time() * 1000)
    requests.post(BASE + "/admin/insert", headers={
        "Authorization": f"Bearer {TOKEN}", "Content-Type": "application/json",
    }, json={"subject": "http://bench.example/person/9999",
             "predicate": "http://xmlns.com/foaf/0.1/name", "object": "Bench"},
        timeout=60)
    deadline = time.time() + 120
    while time.time() < deadline:
        now = {k: v for k, v in stats_deliveries_snapshot().items()
               if v >= t0_ms}
        if len(now) >= k:
            break
        time.sleep(0.1)
    lat = sorted(v - t0_ms for v in now.values())
    print(f"  delivered {len(lat)}/{k} · "
          + (fmt("per-subscriber", (pct(lat, 50), pct(lat, 95), lat[-1])) if lat else "FAILED"),
          flush=True)
    # teardown: unsubscribe + clear the durable delivery log, so the
    # heavier phases start clean
    for i in range(k):
        requests.post(BASE + "/hub", data={
            "hub.mode": "unsubscribe", "hub.topic": "http://localhost:8484/topics/data",
            "hub.callback": f"{SUBSCRIBER}/cb/bench-{i}",
        }, timeout=10)
    print("  (callbacks unsubscribed)", flush=True)


def stats_deliveries_snapshot() -> dict:
    return {str(k): int(v) for k, v in stats_deliveries().items()}


def stats_deliveries():
    return stats_deliveries_raw()


def stats_deliveries_raw():
    return _stats()


def _stats():
    out = subprocess.run(
        ["docker", "compose", "exec", "-T", "demo-subscriber",
         "curl", "-sf", "http://127.0.0.1:9000/stats"],
        capture_output=True, text=True)
    try:
        return json.loads(out.stdout)["deliveries"]
    except (json.JSONDecodeError, KeyError):
        return {}


def cmd_crash():
    # 1. subscribe the crash callback properly (signed, like a real sub)
    requests.post(BASE + "/hub", data={
        "hub.mode": "subscribe", "hub.topic": "http://localhost:8484/topics/data",
        "hub.callback": f"{SUBSCRIBER}/cb/crash-777", "hub.secret": "demo",
    }, timeout=10)
    time.sleep(2)

    print("  stopping service; copying the encrypted secret; planting the "
          "pending delivery …", flush=True)
    subprocess.run(["docker", "compose", "stop", "semantic-web"], capture_output=True)
    time.sleep(2)
    # the persisted secret is the AES-GCM ciphertext the hub stored for this
    # subscription -- copy it verbatim so the hub can sign on redelivery
    q = ("PREFIX h: <http://semweb.dev/ns/hub#> SELECT ?s WHERE { GRAPH "
         "<http://semweb.dev/graph/hub/subscriptions> { ?sub h:secret ?s ; "
         "h:callback \"http://demo-subscriber:9000/cb/crash-777\" } }")
    r = requests.get(STORE + "/query", params={"query": q},
                     headers={"Accept": "application/sparql-results+json"}, timeout=10)
    bindings = r.json()["results"]["bindings"]
    assert bindings, "no encrypted secret found for the crash subscription"
    enc = bindings[0]["s"]["value"]
    enc_escaped = enc.replace("\\", "\\\\").replace('"', '\\"')
    body = ("PREFIX h: <http://semweb.dev/ns/hub#> "
            "INSERT DATA { GRAPH <http://semweb.dev/graph/hub/deliveries> { "
            "<urn:semweb:delivery:bench-crash> h:event 'bench-crash-event' ; "
            "h:topic 'http://localhost:8484/topics/data' ; "
            "h:callback 'http://demo-subscriber:9000/cb/crash-777' ; "
            "h:selfUrl 'http://localhost:8484/topics/data' ; "
            "h:hubUrl 'http://localhost:8484/hub' ; "
            f"h:secret '{enc_escaped}' ; "
            "h:enqueuedAt 0 } }")
    subprocess.run(["curl", "-s", "-X", "POST", STORE + "/update",
                    "-H", "Content-Type: application/sparql-update",
                    "--data-binary", body], capture_output=True)
    subprocess.run(["docker", "compose", "start", "semantic-web"], capture_output=True)
    t0 = time.time()
    print("  service restarted; waiting for the shard's log scan …", flush=True)
    for _ in range(180):
        time.sleep(1)
        if "crash-777" in json.dumps(_stats()):
            print(f"  redelivered after {time.time() - t0:.1f}s "
                  "(restart + log scan interval + delivery)", flush=True)
            return
    print("  NOT delivered within 180s", flush=True)


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else ""
    print(f"bench: {BASE}\n", flush=True)
    if cmd == "load":
        cmd_load()
    elif cmd == "read":
        measure_read()
    elif cmd == "fanout":
        cmd_fanout()
    elif cmd == "crash":
        cmd_crash()
    else:
        sys.exit("usage: bench.py load|read|fanout|crash [--triples N] [--fanout K]")


def measure_read():
    patterns = {
        "paged full scan (100)": BASE + "/fragments?limit=100",
        "predicate-bound": BASE + "/fragments?predicate=http://schema.org/worksFor&limit=100",
        "subject-bound": BASE + "/fragments?subject=http://bench.example/person/42&limit=100",
    }
    for name, url in patterns.items():
        lat = []
        for _ in range(ROUNDS):
            t0 = time.perf_counter()
            requests.get(url, timeout=60).content
            lat.append((time.perf_counter() - t0) * 1000)
        print(fmt(name, (pct(lat, 50), pct(lat, 95), lat[-1])), flush=True)
    urls = {
        "GET /": BASE + "/",
        "GET /manifest": BASE + "/manifest",
        "SPARQL COUNT": STORE + "/query?query=SELECT%20%28COUNT%28%2A%29%20AS%20%3Fc%29%20WHERE%20%7B%20%3Fs%20%3Fp%20%3Fo%20%7D",
    }
    for name, url in urls.items():
        lat = []
        for _ in range(20):
            t0 = time.perf_counter()
            requests.get(url, timeout=60).content
            lat.append((time.perf_counter() - t0) * 1000)
        print(fmt(name, (pct(lat, 50), pct(lat, 95), lat[-1])), flush=True)


if __name__ == "__main__":
    main()