#!/usr/bin/env python3
"""Smoke test for the Semblr microblog: runs the exact queries and writes
the app performs, against a running stack. Exit 0 = all good."""

import json
import os
import sys
import time
import urllib.parse
import urllib.request

BASE = os.environ.get("SEMWEB_URL", "http://localhost:8484")
TOKEN = os.environ.get("SEMWEB_WRITE_TOKEN", "demo-write-token")
NS = "http://semblr.dev/ns#"
FOAF = "http://xmlns.com/foaf/0.1/"
SCHEMA = "http://schema.org/"
RDF_TYPE = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type"
FAIL = 0

U = lambda n: f"http://semblr.dev/u/{n}"  # noqa: E731
P = lambda i: f"http://semblr.dev/p/{i}"  # noqa: E731


def check(name, ok, detail=""):
    global FAIL
    print(("PASS: " if ok else "FAIL: ") + name + (f" — {detail}" if detail and not ok else ""))
    if not ok:
        FAIL = 1


def insert(s, p, o):
    body = json.dumps({"subject": s, "predicate": p, "object": o}).encode()
    req = urllib.request.Request(
        BASE + "/admin/insert", data=body,
        headers={"Content-Type": "application/json",
                 "Authorization": f"Bearer {TOKEN}"}, method="POST")
    with urllib.request.urlopen(req) as r:
        return json.load(r)


def sparql(query):
    body = urllib.parse.urlencode({"query": query}).encode()
    req = urllib.request.Request(
        BASE + "/sparql", data=body,
        headers={"Content-Type": "application/x-www-form-urlencoded"}, method="POST")
    with urllib.request.urlopen(req) as r:
        return json.load(r)


def rows(doc):
    return doc["results"]["bindings"]


def fragments(**params):
    q = urllib.parse.urlencode(params)
    text = urllib.request.urlopen(f"{BASE}/fragments?{q}").read().decode()
    return [json.loads(l) for l in text.splitlines() if l.strip() and "@control" not in l]


print("== seed (idempotent — duplicate inserts suppressed)")
users = [("alice", "Alice"), ("bob", "Bob"), ("carol", "Carol"),
         ("dave", "Dave"), ("erin", "Erin")]
ask = sparql(f"ASK {{ <{U('alice')}> a <{FOAF}Person> }}")
if not ask["boolean"]:
    for n, name in users:
        insert(U(n), RDF_TYPE, f"{FOAF}Person")
        insert(U(n), f"{FOAF}name", name)
    for a, b in [("alice", "bob"), ("alice", "carol"), ("bob", "alice"),
                 ("carol", "alice"), ("dave", "bob")]:
        insert(U(a), f"{FOAF}knows", U(b))
    now = time.time()
    posts = [
        ("alice", "Day 1 on Semblr: my profile is literally triples."),
        ("bob", "Hot take: ORDER BY DESC(?date) is all a feed ever was."),
        ("carol", "TIL every card here is one SPARQL query."),
    ]
    for i, (who, text) in enumerate(posts):
        pid = P(f"seed-{i}")
        insert(pid, RDF_TYPE, f"{SCHEMA}SocialMediaPosting")
        insert(pid, f"{SCHEMA}author", U(who))
        insert(pid, f"{SCHEMA}articleBody", text)
        insert(pid, f"{SCHEMA}datePublished",
               time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(now - (3 - i) * 60)))
else:
    print("(already seeded)")
check("seed users exist", sparql(f"ASK {{ <{U('alice')}> <{FOAF}name> \"Alice\" }}")["boolean"])

print("== feed query (the app's core query)")
feed = sparql(f"""
    PREFIX foaf: <{FOAF}> PREFIX schema: <{SCHEMA}>
    SELECT DISTINCT ?post ?body ?author ?authorName ?date WHERE {{
      {{ ?post schema:author <{U('alice')}> }}
      UNION
      {{ <{U('alice')}> foaf:knows ?followed . ?post schema:author ?followed }}
      ?post schema:articleBody ?body ; schema:author ?author ; schema:datePublished ?date .
      ?author foaf:name ?authorName .
    }} ORDER BY DESC(?date) LIMIT 60""")
check("feed returns posts", len(rows(feed)) >= 3, str(len(rows(feed))))
check("feed ordered desc",
      all(rows(feed)[i]["date"]["value"] >= rows(feed)[i + 1]["date"]["value"]
          for i in range(len(rows(feed)) - 1)))

print("== likes + count")
insert(U("bob"), f"{NS}likes", P("seed-0"))
likes = sparql(f"SELECT ?post (COUNT(?l) AS ?c) WHERE {{ ?l <{NS}likes> ?post }} GROUP BY ?post")
check("like count = 1", any(r["c"]["value"] == "1" and r["post"]["value"] == P("seed-0")
                            for r in rows(likes)))

print("== duplicate like is suppressed (no counter drift)")
insert(U("bob"), f"{NS}likes", P("seed-0"))
likes = sparql(f"SELECT ?post (COUNT(?l) AS ?c) WHERE {{ ?l <{NS}likes> ?post }} GROUP BY ?post")
check("like still 1 after duplicate insert",
      all(r["c"]["value"] == "1" for r in rows(likes)))

print("== multi-hop suggestions for a fresh user")
insert(U("zoe"), RDF_TYPE, f"{FOAF}Person")
insert(U("zoe"), f"{FOAF}name", "Zoe")
insert(U("zoe"), f"{FOAF}knows", U("bob"))
sugg = sparql(f"""
    SELECT ?p ?n (COUNT(DISTINCT ?f) AS ?m) WHERE {{
      <{U('zoe')}> <{FOAF}knows> ?f . ?f <{FOAF}knows> ?p . ?p <{FOAF}name> ?n .
      FILTER(?p != <{U('zoe')}>) FILTER NOT EXISTS {{ <{U('zoe')}> <{FOAF}knows> ?p }}
    }} GROUP BY ?p ?n LIMIT 6""")
check("suggestions found", len(rows(sugg)) > 0)

print("== profile via fragments (alias keys)")
prof = fragments(subject=U("alice"), limit=50)
check("profile name via alias key", any(l.get("name") == "Alice" for l in prof))
check("profile knows via alias key", any(l.get("knows", {}).get("@id") == U("bob") for l in prof))

print("== ship-a-feature: new predicate learned")
man = json.load(urllib.request.urlopen(BASE + "/manifest"))
before = {p["uri"] for p in man["predicates"]}
insert(P("seed-0"), f"{NS}reaction", "🎉")
man2 = json.load(urllib.request.urlopen(BASE + "/manifest"))
after = {p["uri"] for p in man2["predicates"]}
check("fingerprint changed", man["schemaFingerprint"] != man2["schemaFingerprint"])
check("new predicate in manifest", f"{NS}reaction" in after - before)
reactions = sparql(f"SELECT ?post ?r (COUNT(*) AS ?c) WHERE {{ ?post <{NS}reaction> ?r }} GROUP BY ?post ?r")
check("reaction chips query", any(r["r"]["value"] == "🎉" for r in rows(reactions)))

print("== duplicate post suppresses notification")
r1 = insert(U("alice"), f"{SCHEMA}likes", U("bob"))
dup = insert(U("alice"), f"{SCHEMA}likes", U("bob"))
check("duplicate flagged", dup["duplicate"] is True and dup["data_subscribers_notified"] == 0)

print("== agent joins as a first-class node (MCP)")
def mcp(method, params):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = urllib.request.Request(BASE + "/mcp", data=body,
                                 headers={"Content-Type": "application/json"})
    return json.load(urllib.request.urlopen(req))

tools = mcp("tools/list", {})["result"]["tools"]
check("mcp tools listed", {t["name"] for t in tools} >=
      {"search_graph", "sparql_query", "get_manifest", "insert_triple"})
insert(U("agent"), RDF_TYPE, f"{FOAF}Person")
insert(U("agent"), f"{FOAF}name", "Semblr Agent")
# a human follows the agent (the app's suggestion flow)
insert(U("alice"), f"{FOAF}knows", U("agent"))
pid = P("agent-1")
insert(pid, RDF_TYPE, f"{SCHEMA}SocialMediaPosting")
insert(pid, f"{SCHEMA}author", U("agent"))
insert(pid, f"{SCHEMA}articleBody", "The agent posts like anyone else — same triples, same rules.")
insert(pid, f"{SCHEMA}datePublished", time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()))
feed2 = sparql(f"""
    PREFIX foaf: <{FOAF}> PREFIX schema: <{SCHEMA}>
    SELECT ?post ?authorName WHERE {{
      {{ ?post schema:author <{U('alice')}> }} UNION
      {{ <{U('alice')}> foaf:knows ?f . ?post schema:author ?f }}
      ?post schema:articleBody ?b ; schema:author ?a ; schema:datePublished ?d .
      ?a foaf:name ?authorName .
    }}""")
check("agent post appears in feed like anyone's",
      any("Semblr Agent" == r["authorName"]["value"] for r in rows(feed2)))

print()
sys.exit(FAIL)