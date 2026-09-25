/* Semblr — a microblog on a knowledge graph.
   Hand-rolled, zero build step: same philosophy as the service itself.
   Every feature maps onto the semantic-web API:
     feed        -> POST /sparql (posts from people you follow)
     likes       -> triples on http://semblr.dev/ns#likes + GROUP BY count
     suggestions -> multi-hop SPARQL over foaf:knows
     live feed   -> SSE GET /events, refetch on every event
     profile     -> GET /fragments?subject=...
     schema      -> GET /manifest fingerprint; unknown predicates learned live
     agent       -> POST /mcp tools + Together AI, posting through the same write path */

"use strict";

const DEFAULT_API = "http://localhost:8484";
const DEMO_TOKEN = "demo-write-token";
const NS = "http://semblr.dev/ns#";
const U = (n) => `http://semblr.dev/u/${n}`;
const P = (id) => `http://semblr.dev/p/${id}`;

const FOAF = "http://xmlns.com/foaf/0.1/";
const SCHEMA = "http://schema.org/";
const RDF_TYPE = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const SOCIAL_POST = "http://schema.org/SocialMediaPosting";
const AUTHOR = "http://schema.org/author";
const BODY = "http://schema.org/articleBody";
const DATE = "http://schema.org/datePublished";
const DESCRIBE = "http://schema.org/description";

const state = {
  api: (localStorage.getItem("semblr_api") || DEFAULT_API).replace(/\/$/, ""),
  token: localStorage.getItem("semblr_token") || DEMO_TOKEN,
  me: null,                     // { uri, name }
  users: new Map(),             // uri -> name
  knownPredicates: new Set(),   // predicates seen in the manifest
  fingerprint: null,
  sseCount: 0,
  refetchTimer: null,
  fpTimer: null,
  agent: null,                  // { key, tools }
};

/* ---------------- tiny helpers ---------------- */

const $ = (sel) => document.querySelector(sel);
const esc = (s) =>
  String(s ?? "").replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));

function toast(msg, ms = 3500) {
  const t = $("#toast");
  t.textContent = msg;
  t.classList.remove("hidden");
  clearTimeout(t._h);
  t._h = setTimeout(() => t.classList.add("hidden"), ms);
}

const bind = (doc) => (doc.results ? doc.results.bindings : []);

async function sparql(query) {
  const r = await fetch(`${state.api}/sparql`, {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({ query }),
  });
  if (!r.ok) throw new Error(`sparql ${r.status}: ${await r.text()}`);
  return r.json();
}

async function insert(subject, predicate, object) {
  const r = await fetch(`${state.api}/admin/insert`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${state.token}`,
    },
    body: JSON.stringify({ subject, predicate, object }),
  });
  if (!r.ok) throw new Error((await r.text()) || r.statusText);
  return r.json();
}

const fetchManifest = () => fetch(`${state.api}/manifest`).then((r) => r.json());

async function fragments(params) {
  const q = new URLSearchParams(params).toString();
  const text = await fetch(`${state.api}/fragments?${q}`).then((r) => r.text());
  return text.split("\n")
    .filter((l) => l.trim() && !l.includes('"@control"'))
    .map((l) => JSON.parse(l));
}

/* fragments lines are compacted JSON-LD: predicates are aliased
   ("name", "knows"), unknown namespaces stay full URIs */
const termVal = (v) => (typeof v === "object" ? (v["@id"] ?? v["@value"]) : v);
const lineVal = (lines, key) => {
  const l = lines.find((x) => x[key] !== undefined);
  return l ? termVal(l[key]) : null;
};
const lineVals = (lines, key) =>
  lines.filter((l) => l[key] !== undefined).map((l) => termVal(l[key]));

/* ---------------- demo seed ---------------- */

const SEED = {
  users: [["alice", "Alice"], ["bob", "Bob"], ["carol", "Carol"],
          ["dave", "Dave"], ["erin", "Erin"]],
  follows: [["alice", "bob"], ["alice", "carol"], ["bob", "alice"],
            ["bob", "carol"], ["bob", "dave"], ["carol", "alice"],
            ["dave", "alice"], ["dave", "bob"], ["erin", "alice"]],
  posts: [
    ["alice", "Day 1 on Semblr: my profile is literally a few triples and I can see them. Feels weird. Feels right."],
    ["bob", "Hot take: ORDER BY DESC(?date) is all a feed ever was."],
    ["carol", "TIL every card here is fetched with one SPARQL query. No bespoke endpoints."],
    ["dave", "The ship-a-feature button inserts a predicate nobody has ever used. The UI learns it live."],
    ["alice", "Following people is just foaf:knows. Suggested follows come from multi-hop graph queries."],
    ["bob", "My likes are triples too — append-only, like real life."],
    ["carol", "Ask the agent panel anything about this network; it reads the same manifest we do."],
    ["erin", "First post! Every post here has a URI you can copy. Try the 'triples' button."],
  ],
};

async function postTriples(id, author, text, when) {
  await insert(id, RDF_TYPE, SOCIAL_POST);
  await insert(id, AUTHOR, author);
  await insert(id, BODY, text);
  await insert(id, DATE, when);
}

async function ensureUser(name) {
  const uri = U(name);
  await insert(uri, RDF_TYPE, `${FOAF}Person`);
  await insert(uri, `${FOAF}name`, NAME_OF[name] ?? name);
  return uri;
}

const NAME_OF = { alice: "Alice", bob: "Bob", carol: "Carol",
                  dave: "Dave", erin: "Erin" };

async function bootstrap() {
  const ask = await sparql(`ASK { <${U("alice")}> a <${FOAF}Person> }`);
  if (ask.boolean === true) return;
  toast("seeding the demo graph (real inserts — watch the pulse)…");
  for (const [n] of SEED.users) await ensureUser(n);
  for (const [a, b] of SEED.follows) await insert(U(a), `${FOAF}knows`, U(b));
  const now = Date.now();
  for (let i = 0; i < SEED.posts.length; i++) {
    const [who, text] = SEED.posts[i];
    const when = new Date(now - (SEED.posts.length - i) * 47 * 60 * 1000).toISOString();
    await postTriples(P(`seed-${i}`), U(who), text, when);
  }
}

/* ---------------- feed ---------------- */

async function loadFeed() {
  if (!state.me) return;
  const me = state.me.uri;
  const rows = bind(await sparql(`
    PREFIX foaf: <${FOAF}> PREFIX schema: <${SCHEMA}>
    SELECT DISTINCT ?post ?body ?author ?authorName ?date WHERE {
      { ?post schema:author <${me}> }
      UNION
      { <${me}> foaf:knows ?followed . ?post schema:author ?followed }
      ?post schema:articleBody ?body ; schema:author ?author ; schema:datePublished ?date .
      ?author foaf:name ?authorName .
    } ORDER BY DESC(?date) LIMIT 60`));

  const [likeRows, myLikeRows, reactRows] = await Promise.all([
    sparql(`SELECT ?post (COUNT(?l) AS ?c) WHERE { ?l <${NS}likes> ?post } GROUP BY ?post`),
    sparql(`SELECT ?post WHERE { <${me}> <${NS}likes> ?post }`),
    state.knownPredicates.has(`${NS}reaction`)
      ? sparql(`SELECT ?post ?r (COUNT(*) AS ?c) WHERE { ?post <${NS}reaction> ?r } GROUP BY ?post ?r`)
      : Promise.resolve({ results: { bindings: [] } }),
  ]);
  const likeCount = new Map(bind(likeRows).map((r) => [r.post.value, +r.c.value]));
  const myLikes = new Set(bind(myLikeRows).map((r) => r.post.value));
  const reactions = new Map();
  for (const r of bind(reactRows)) {
    const m = reactions.get(r.post.value) || {};
    m[r.r.value] = +r.c.value;
    reactions.set(r.post.value, m);
  }

  $("#feed").innerHTML = rows.map((r) => postCard(r, likeCount, myLikes, reactions)).join("");
  wireFeed();
}

function postCard(r, likeCount, myLikes, reactions) {
  const uri = r.post.value;
  const chips = Object.entries(reactions.get(uri) || {})
    .map(([e, c]) => `<span class="chip reaction">${esc(e)}${c > 1 ? ` ×${c}` : ""}</span>`)
    .join(" ");
  const liked = myLikes.has(uri);
  return `
  <article class="card post">
    <div class="post-head">
      <button class="author" data-profile="${esc(r.author.value)}">${esc(r.authorName.value)}</button>
      <span class="when">${esc(new Date(r.date.value).toLocaleString())}</span>
    </div>
    <p class="body">${esc(r.body.value)}</p>
    <div class="post-foot">
      <button class="btn tiny ${liked ? "liked" : ""}" data-like="${esc(uri)}">♥ ${likeCount.get(uri) || 0}${liked ? " · liked" : ""}</button>
      <button class="btn tiny ghost" data-triples="${esc(uri)}">triples</button>
      <code class="uri-hint" title="${esc(uri)}">${esc(uri.replace("http://semblr.dev", ""))}</code>
    </div>
    ${chips ? `<div class="chips">${chips}</div>` : ""}
  </article>`;
}

function wireFeed() {
  document.querySelectorAll("[data-like]").forEach((b) =>
    b.addEventListener("click", () => likePost(b.dataset.like)));
  document.querySelectorAll("[data-triples]").forEach((b) =>
    b.addEventListener("click", () => showTriples(b.dataset.triples)));
  document.querySelectorAll("[data-profile]").forEach((b) =>
    b.addEventListener("click", () => showProfile(b.dataset.profile)));
}

/* ---------------- profile + triples modals ---------------- */

async function showTriples(uri) {
  const lines = await fragments({ subject: uri, limit: 50 });
  openModal(`<h3>Triples of <code>${esc(uri)}</code></h3>` +
    lines.map((l) => `<pre class="triple">${esc(JSON.stringify(l))}</pre>`).join("") +
    `<p class="hint">Streamed from GET /fragments — one compacted JSON-LD statement
     per line. Full URIs live in
     <a href="${state.api}/context.jsonld" target="_blank">/context.jsonld</a>.</p>`);
}

async function showProfile(uri) {
  const lines = await fragments({ subject: uri, limit: 60 });
  const name = lineVal(lines, "name") ?? "(unknown)";
  const knows = lineVals(lines, "knows");
  const bio = lineVal(lines, "description"); // schema:description (may be agent-written)
  const posts = bind(await sparql(`
    PREFIX schema: <${SCHEMA}>
    SELECT ?post ?body ?date WHERE {
      ?post schema:author <${uri}> ;
            schema:articleBody ?body ;
            schema:datePublished ?date .
    } ORDER BY DESC(?date) LIMIT 30`));
  openModal(`
    <h3>${esc(name)}</h3>
    <code class="uri-hint">${esc(uri)}</code>
    ${bio ? `<p class="bio">${esc(bio)}</p>` : ""}
    <div class="row-label">Follows (foaf:knows)</div>
    <ul class="plain-list">${knows.map((f) =>
      `<li><button class="linkish" data-profile2="${esc(f)}">${esc(state.users.get(f) ?? f)}</button></li>`)
      .join("") || "<li class='hint'>nobody yet</li>"}</ul>
    <div class="row-label">Posts</div>
    ${posts.map((p) => `
      <div class="card slim-post">
        <p class="body">${esc(p.body.value)}</p>
        <span class="hint">${esc(new Date(p.date.value).toLocaleString())}</span>
      </div>`).join("")}`);
  document.querySelectorAll("#modal-card [data-profile]").forEach((b) =>
    b.addEventListener("click", () => showProfile(b.dataset.profile)));
}

function openModal(html) {
  $("#modal-card").innerHTML = html;
  $("#modal").classList.remove("hidden");
}
$("#modal").addEventListener("click", (e) => {
  if (e.target.id === "modal") $("#modal").classList.add("hidden");
});

/* ---------------- writes ---------------- */

async function doPost() {
  const box = $("#post-box");
  const text = box.value.trim();
  if (!text || !state.me) return;
  const btn = $("#post-btn");
  btn.disabled = true;
  try {
    await postTriples(P(`t${Date.now().toString(36)}`),
      state.me.uri, text, new Date().toISOString());
    box.value = "";
    $("#charcount").textContent = 280;
    await loadFeed();
  } catch (e) {
    toast(`post failed: ${e.message}`);
  } finally {
    btn.disabled = false;
  }
}

async function likePost(uri) {
  try {
    await insert(state.me.uri, `${NS}likes`, uri);
    await loadFeed();
  } catch (e) {
    toast(`like failed: ${e.message}`);
  }
}

async function follow(target) {
  await insert(state.me.uri, `${FOAF}knows`, target);
  await Promise.all([loadSidebar(), loadFeed()]);
  toast(`now following ${state.users.get(target) ?? target}`);
}

async function createUser(name) {
  const uri = U(name.toLowerCase().replace(/[^a-z0-9-]/g, "") || `u${Date.now()}`);
  await insert(uri, RDF_TYPE, `${FOAF}Person`);
  await insert(uri, `${FOAF}name`, name);
  return uri;
}

/* ---------------- the ship-a-feature moment ---------------- */

const EMOJI = ["🎉", "🔥", "💡", "🤯", "🧠", "🚀"];

async function shipFeature() {
  const rows = bind(await sparql(`
    PREFIX schema: <${SCHEMA}>
    SELECT ?post WHERE { ?post schema:datePublished ?d } ORDER BY DESC(?d) LIMIT 1`));
  const post = rows[0]?.post.value ?? P("seed-0");
  const emoji = EMOJI[Math.floor(Math.random() * EMOJI.length)];
  await insert(post, `${NS}reaction`, emoji);
  toast(`inserted: <post> sem:reaction "${emoji}" — watch the UI learn it`);
}

/* ---------------- schema awareness (the living UI) ---------------- */

async function refreshSchema() {
  const m = await fetchManifest();
  $("#fp").textContent = m.schemaFingerprint.slice(7, 23) + "…";
  $("#fp-classes").textContent = m.classes.map((c) => c.compact).join(", ") || "–";
  $("#pred-count").textContent = m.predicates.length;
  if (state.fingerprint && m.schemaFingerprint !== state.fingerprint) flash($("#fp"));
  state.fingerprint = m.schemaFingerprint;

  const news = m.predicates.map((p) => p.uri)
    .filter((u) => !state.knownPredicates.has(u));
  news.forEach((u) => state.knownPredicates.add(u));
  if (news.length) {
    $("#schema-banner").classList.remove("hidden");
    $("#schema-banner").innerHTML = `⚡ schema changed — new predicate` +
      `${news.length > 1 ? "s" : ""}: ` +
      news.map((u) => `<code>${esc(u)}</code>`).join(", ") +
      ` — the UI adapted with no deploy.`;
  }
  if (news.some((u) => u === `${NS}reaction`)) await loadFeed(); // re-render chips
}

function flash(el) {
  el.classList.add("flash");
  setTimeout(() => el.classList.remove("flash"), 1600);
}

/* ---------------- the agent ---------------- */

const AGENT_URI = U("agent");
const AGENT_NAME = "Semblr Agent";
const MODEL = "meta-llama/Llama-3.3-70B-Instruct-Turbo";

function mcpRpc(method, params) {
  return fetch(`${state.api}/mcp`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: Math.random(), method, params }),
  }).then(async (r) => {
    const doc = await r.json();
    if (doc.error) throw new Error(doc.error.message);
    return doc;
  });
}

function agentSay(html, cls = "") {
  const div = document.createElement("div");
  div.className = `agent-msg ${cls}`;
  div.innerHTML = html;
  $("#agent-log").appendChild(div);
  $("#agent-log").scrollTop = 1e9;
}

async function agentConnect() {
  const key = $("#together-key").value.trim() ||
    localStorage.getItem("semblr_together_key") || "";
  if (!key) return toast("paste a Together AI key first");
  localStorage.setItem("semblr_together_key", key);
  try {
    const init = await mcpRpc("initialize", {
      protocolVersion: "2025-06-18", capabilities: {},
      clientInfo: { name: "semblr-web-agent", version: "0.1.0" },
    });
    const tools = (await mcpRpc("tools/list", {})).result.tools;
    state.agent = { key, tools };
    // The agent joins the network as a first-class node (idempotent —
    // duplicate inserts are suppressed by the write path).
    await insert(AGENT_URI, RDF_TYPE, `${FOAF}Person`);
    await insert(AGENT_URI, `${FOAF}name`, AGENT_NAME);
    await loadSidebar();
    $("#agent-auth").classList.add("hidden");
    $("#agent-body").classList.remove("hidden");
    agentSay(`connected: ${init.result.serverInfo.name} — ${tools.length} tools
      discovered from the MCP server. I am now a member of this graph (${esc(AGENT_URI)}).
      Follow me from the suggestions panel and my posts will appear in your feed.`);
  } catch (e) {
    toast(`agent failed to connect: ${e.message}`);
  }
}

function openAiTools() {
  return state.agent.tools.map((t) => ({
    type: "function",
    function: { name: t.name, description: t.description, parameters: t.inputSchema },
  }));
}

async function agentLoop(userText, budget = 7) {
  const messages = [{
    role: "system",
    content: `You are the Semblr Agent, a member of a microblog whose database is a knowledge graph.
Your identity: ${AGENT_URI} (a foaf:Person named "${AGENT_NAME}").
Today: ${new Date().toISOString()}.

Rules:
- Ground every claim in ACTUAL tool results from this conversation. Never invent URIs.
- URIs are case-sensitive: copy them exactly from tool output.
- SPARQL needs PREFIX declarations — copy them from get_manifest's "prefixes" block, and add
  PREFIX sem: <${NS}> for app-specific terms (likes = sem:likes).
- The current user is ${state.me ? state.me.uri : "(unknown)"}. Refer to them as "you".
- To post AS YOURSELF: call insert_triple several times for one new subject
  http://semblr.dev/p/<yourid>: rdf:type ${SOCIAL_POST}, then schema:author ${AGENT_URI},
  then schema:articleBody with the text, then schema:datePublished with an ISO timestamp.
  Pass the write token "${state.token}" as the tool's "token" argument on every call.
- Keep posts under 280 characters. Do not narrate actions you did not perform.`,
  }, { role: "user", content: userText }];

  for (let i = 0; i < budget; i++) {
    const r = await fetch("https://api.together.xyz/v1/chat/completions", {
      method: "POST",
      headers: { "Content-Type": "application/json", Authorization: `Bearer ${state.agent.key}` },
      body: JSON.stringify({ model: MODEL, messages, tools: openAiTools() }),
    });
    if (!r.ok) { agentSay(`⚠ together api ${r.status}`); return; }
    const doc = await r.json();
    const choice = doc.choices?.[0];
    if (!choice) { agentSay(`⚠ ${doc.error?.message ?? "model error"}`); return; }
    if (choice.finish_reason !== "tool_calls" || !choice.message.tool_calls?.length) {
      agentSay(esc(choice.message.content || ""));
      return;
    }
    messages.push(choice.message);
    for (const tc of choice.message.tool_calls) {
      let args = {};
      try { args = JSON.parse(tc.function.arguments || "{}"); } catch { /* noop */ }
      agentSay(`<span class="chip tool">${esc(tc.function.name)}</span>
        <code class="dim">${esc(JSON.stringify(args).slice(0, 150))}</code>`, "toolcall");
      try {
        const res = await mcpRpc("tools/call", { name: tc.function.name, arguments: args });
        const out = (res.result?.content ?? []).map((c) => c.text).join("\n");
        messages.push({ role: "tool", tool_call_id: tc.id, content: String(out).slice(0, 4000) });
        agentSay(`<code class="dim">${esc(String(out).slice(0, 200))}</code>`, "toolresult");
      } catch (e) {
        messages.push({ role: "tool", tool_call_id: tc.id, content: `TOOL ERROR: ${e.message}` });
        agentSay(`<code class="dim">TOOL ERROR: ${esc(e.message)}</code>`, "toolresult");
      }
    }
  }
  agentSay("(tool budget exhausted)");
}

async function quickAction(kind) {
  if (!state.agent) return toast("bring the agent online first");
  const me = state.me;
  if (kind === "digest") {
    agentSay(`<b>→ digesting the timeline…</b>`);
    await agentLoop(
      `Read the most recent posts (sparql_query: posts with schema:articleBody ordered by
       schema:datePublished DESC LIMIT 12). Then write ONE digest post as yourself (<=280
       chars, mention who said the most interesting things by name). Post it, then confirm.`);
  } else if (kind === "suggest") {
    agentSay(`<b>→ who ${esc(me.name)} should follow</b>`);
    await agentLoop(
      `Suggest up to 3 people the current user (${me.uri}) should follow. Use sparql_query
       over foaf:knows mutual-follow counts (exclude people they already follow and
       themselves). Ground every reason in the actual counts you saw. Keep it short.`);
  } else if (kind === "bio") {
    agentSay(`<b>→ drafting a bio for ${esc(me.name)}</b>`);
    await agentLoop(
      `Read the current user's posts (sparql_query for schema:author ${me.uri}).
       Then propose a bio for them (max 140 chars) and show it prefixed with PROPOSAL:.
       Do NOT insert anything for this request.`);
    // offer an apply button for the last PROPOSAL: line
    const lines = [...document.querySelectorAll("#agent-log .agent-msg")]
      .map((n) => n.textContent);
    const prop = [...lines].reverse().find((t) => t.includes("PROPOSAL:"));
    if (prop) {
      const text = prop.split("PROPOSAL:")[1].trim().slice(0, 280);
      const apply = document.createElement("button");
      apply.className = "btn small primary";
      apply.textContent = "Apply as my bio";
      apply.onclick = async () => {
        await insert(me.uri, DESCRIBE, text);
        agentSay(`bio written to your profile as <code>schema:description</code> —
          open your profile card to see it (a predicate the UI learned live).`);
        apply.disabled = true;
      };
      $("#agent-log").appendChild(apply);
      $("#agent-log").scrollTop = 1e9;
    }
  }
}

/* ---------------- SSE live ---------------- */

function connectSSE() {
  const es = new EventSource(`${state.api}/events?topic=/topics/data`);
  es.onopen = () => { $("#pulse-text").textContent = "live — 0 events"; };
  es.onmessage = (ev) => {
    state.sseCount += 1;
    let id = "";
    try { id = JSON.parse(ev.data).event_id ?? ""; } catch { /* noop */ }
    $("#pulse-text").textContent = `live — ${state.sseCount} events · last ${id.slice(0, 8)}`;
    $("#pulse").classList.add("hot");
    setTimeout(() => $("#pulse").classList.remove("hot"), 400);
    clearTimeout(state.refetchTimer);
    state.refetchTimer = setTimeout(() => { loadFeed(); }, 500);
    clearTimeout(state.fpTimer);
    state.fpTimer = setTimeout(() => refreshSchema().catch(() => {}), 2500);
  };
  es.onerror = () => { $("#pulse-text").textContent = "reconnecting…"; };
}

/* ---------------- sidebar ---------------- */

async function loadSidebar() {
  const me = state.me.uri;
  const rows = bind(await sparql(
    `SELECT ?u ?n WHERE { ?u a <${FOAF}Person> ; <${FOAF}name> ?n } ORDER BY ?n`));
  state.users = new Map(rows.map((r) => [r.u.value, r.n.value]));

  const sel = $("#user-select");
  sel.innerHTML = rows.map((r) =>
    `<option value="${esc(r.u.value)}">${esc(r.n.value)}</option>`).join("");
  sel.value = me;

  const [followRows, suggRows] = await Promise.all([
    sparql(`SELECT ?p ?n WHERE { <${me}> <${FOAF}knows> ?p . ?p <${FOAF}name> ?n }`),
    sparql(`SELECT ?p ?n (COUNT(DISTINCT ?f) AS ?m) WHERE {
       <${me}> <${FOAF}knows> ?f . ?f <${FOAF}knows> ?p . ?p <${FOAF}name> ?n .
       FILTER(?p != <${me}>) FILTER NOT EXISTS { <${me}> <${FOAF}knows> ?p }
     } GROUP BY ?p ?n LIMIT 6`),
  ]);

  $("#following-list").innerHTML = bind(followRows).map((r) => `
    <li><button class="linkish" data-profile="${esc(r.p.value)}">${esc(r.n.value)}</button></li>`)
    .join("") || `<li class="hint">nobody yet — use the suggestions</li>`;

  let sugg = bind(suggRows).map((r) => ({ uri: r.p.value, name: r.n.value, m: +r.m.value }));
  if (!sugg.length) {
    // directory fallback for fresh accounts: anyone you don't follow yet
    const all = bind(await sparql(
      `SELECT ?u ?n WHERE { ?u a <${FOAF}Person> ; <${FOAF}name> ?n .
         FILTER(?u != <${me}>) FILTER NOT EXISTS { <${me}> <${FOAF}knows> ?u } } LIMIT 5`));
    sugg = all.map((r) => ({ uri: r.u.value, name: r.n.value, m: 0 }));
  }
  $("#suggestions").innerHTML = sugg.map((s) => `
    <li><button class="linkish" data-profile="${esc(s.uri)}">${esc(s.name)}</button>
        ${s.m ? `<span class="hint">${s.m} mutual</span>` : ""}
        <button class="btn tiny" data-follow="${esc(s.uri)}">follow</button></li>`).join("")
    || `<li class="hint">you follow everyone here — create a new user</li>`;

  document.querySelectorAll("[data-follow]").forEach((b) =>
    b.addEventListener("click", () => follow(b.dataset.follow).catch((e) => toast(e.message))));
  document.querySelectorAll("[data-profile]").forEach((b) =>
    b.addEventListener("click", () => showProfile(b.dataset.profile)));
}

/* ---------------- boot ---------------- */

async function boot() {
  $("#api-base").value = state.api;
  $("#write-token").value = state.token;
  $("#api-base").addEventListener("change", (e) => {
    state.api = e.target.value.replace(/\/$/, "");
    localStorage.setItem("semblr_api", state.api);
    location.reload();
  });
  $("#write-token").addEventListener("change", (e) => {
    state.token = e.target.value.trim() || DEMO_TOKEN;
    localStorage.setItem("semblr_token", state.token);
    toast("token saved");
  });

  await bootstrap();
  const rows = bind(await sparql(
    `SELECT ?u ?n WHERE { ?u a <${FOAF}Person> ; <${FOAF}name> ?n } ORDER BY ?n`));
  if (!rows.length) return toast("seed still loading — refresh in a second");
  state.users = new Map(rows.map((r) => [r.u.value, r.n.value]));

  const saved = localStorage.getItem("semblr_me");
  const meUri = saved && rows.some((r) => r.u.value === saved) ? saved : rows[0].u.value;
  localStorage.setItem("semblr_me", meUri);
  state.me = { uri: meUri, name: state.users.get(meUri) ?? meUri };

  const sel = $("#user-select");
  sel.innerHTML = rows.map((r) =>
    `<option value="${esc(r.u.value)}">${esc(r.n.value)}</option>`).join("");
  sel.value = meUri;
  sel.addEventListener("change", (e) => {
    localStorage.setItem("semblr_me", e.target.value);
    location.reload();
  });

  $("#create-user").addEventListener("click", async () => {
    const name = $("#new-user-name").value.trim();
    if (!name) return;
    const uri = await createUser(name);
    localStorage.setItem("semblr_me", uri);
    toast(`created ${name} — you are now them`);
    location.reload();
  });

  $("#post-btn").addEventListener("click", doPost);
  const box = $("#post-box");
  box.addEventListener("input", () => { $("#charcount").textContent = 280 - box.value.length; });
  box.addEventListener("keydown", (e) => {
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") doPost();
  });
  $("#ship-feature").addEventListener("click", () => shipFeature().catch((e) => toast(e.message)));

  $("#agent-connect").addEventListener("click", () => agentConnect().catch((e) => toast(e.message)));
  $("#agent-send").addEventListener("click", async () => {
    const q = $("#agent-input").value.trim();
    if (!q) return;
    $("#agent-input").value = "";
    agentSay(`<b>you:</b> ${esc(q)}`);
    await agentLoop(q);
  });
  $("#agent-input").addEventListener("keydown", (e) => {
    if (e.key === "Enter") $("#agent-send").click();
  });
  document.querySelectorAll("[data-quick]").forEach((b) =>
    b.addEventListener("click", () => quickAction(b.dataset.quick).catch((e) => toast(e.message))));

  const m = await fetchManifest();
  m.predicates.forEach((p) => state.knownPredicates.add(p.uri));
  await loadSidebar();
  await loadFeed();
  await refreshSchema();
  connectSSE();
}

boot().catch((e) => {
  document.body.insertAdjacentHTML("beforeend",
    `<div class="toast">cannot reach ${state.api} — ${esc(e.message)} (is docker compose up?)</div>`);
});