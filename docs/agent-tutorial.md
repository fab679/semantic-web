# Agent tutorial: Together AI + MCP

A complete, runnable tutorial (~200 lines of Python). The agent answers
questions from the graph with **Together AI** doing the reasoning and the
semantic-web **MCP server** providing every tool. Full source:
[`examples/agent/agent.py`](https://github.com/fabisch/semantic-web/blob/master/examples/agent/agent.py).

## The architecture in one picture

```
  user ──question──► agent.py ──tool loop──► Together AI (Llama 3.3 70B)
                        │                        │  tool calls
                        │                        ▼
                        │                 MCP tools/call ──► semweb /mcp
                        │                                          │
                        └── /events (SSE, optional) ◄── publish ────┘
```

## Step 1 — Run the graph

```sh
docker compose up --build -d
```

## Step 2 — The agent discovers its tools

The agent does not hardcode tools. It asks the server:

```python
class McpClient:
    def _rpc(self, method, params):
        self._id += 1
        r = requests.post(f"{self.url}/mcp",
                          json={"jsonrpc": "2.0", "id": self._id,
                                "method": method, "params": params})
        return r.json()

    def initialize(self):
        return self._rpc("initialize", {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "semweb-tutorial-agent", "version": "0.1.0"},
        })["result"]

    def list_tools(self):
        return self._rpc("tools/list", {})["result"]["tools"]

    def call_tool(self, name, arguments):
        res = self._rpc("tools/call", {"name": name, "arguments": arguments})["result"]
        text = "\n".join(c.get("text", "") for c in res.get("content", []))
        return f"TOOL ERROR: {text}" if res.get("isError") else text

    def read_manifest_resource(self):
        res = self._rpc("resources/read",
                        {"uri": "manifest://semantic-web/current"})["result"]
        return res["contents"][0]["text"]
```

Because Together AI's function-calling uses the same OpenAI-style schema
shape, the bridge is one line per tool:

```python
def to_openai_tool(mcp_tool):
    return {"type": "function", "function": {
        "name": mcp_tool["name"],
        "description": mcp_tool["description"],
        "parameters": mcp_tool["inputSchema"],
    }}
```

**Adding a tool on the server extends every agent without touching
them.** That is the whole point of MCP here.

## Step 3 — Ground on the manifest

Before the loop, the agent reads the manifest (via the MCP *resource*,
not a hardcoded endpoint) and builds the system prompt:

```python
manifest = mcp.read_manifest_resource()
guidance = mcp.get_prompt("explore_graph", {"focus": "the user's question"})
system = make_system_prompt(manifest, guidance)
```

The manifest is what makes the LLM reliable: it names every term, explains
it in human language, and hands it example queries.

## Step 4 — The Together AI tool loop

The standard loop: model → tool calls → execute via MCP → append results
→ repeat until `finish_reason == "stop"`.

```python
for _ in range(8):  # bounded
    response = client.chat.completions.create(
        model="meta-llama/Llama-3.3-70B-Instruct-Turbo",
        messages=messages,
        tools=[to_openai_tool(t) for t in mcp.list_tools()],
    )
    choice = response.choices[0]
    if choice.finish_reason != "tool_calls":
        print(choice.message.content)
        return
    messages.append(choice.message)
    for tc in choice.message.tool_calls:
        args = json.loads(tc.function.arguments)
        result = mcp.call_tool(tc.function.name, args)
        messages.append({"role": "tool", "tool_call_id": tc.id,
                         "content": result[:4000]})
```

## Step 5 — Run it

```sh
# uv-managed (recommended -- installs into the example's own env):
export TOGETHER_API_KEY=your_key
uv run --project examples/agent agent.py "Who works at Acme and what do we know about them?"

# or plain pip in your own env:
pip install "together>=2.0.0" requests
python examples/agent/agent.py "Who works at Acme and what do we know about them?"
```

```
connected: semantic-web (protocol 2025-06-18)
discovered 6 tools from the server: ['search_graph', 'sparql_query', ...]
  [tool] get_manifest({})
  [tool] search_graph({"predicate": "http://schema.org/worksFor", "limit": 20})

Agent: Acme Corp is an organization founded on 2001-04-03. Two people
work there: Alice (http://example.org/alice) and Bob
(http://example.org/bob)...
```

Watch the graph change live while the agent works:

```sh
SEMWEB_SUBSCRIBE=1 python agent.py "What classes exist?"
# [events] {"topic":"/topics/data","event_id":"9c7c..."}
```

## Extending the tutorial

- **Write path**: give the agent `SEMWEB_WRITE_TOKEN` and it can
  `insert_triple` — the agent attaches the token to those tool calls
  automatically (`agent.py` does this when the env var is set), and the
  server rejects writes that present no valid token.
- **Real-time**: `SEMWEB_SUBSCRIBE=1` runs a background SSE listener;
  or call the `subscribe` tool to register a callback for durable push.
- **Multi-graph**: point `SEMWEB_URL` at replicas; or use the `graph`
  argument for tenant isolation.

## The three takeaways

1. **Manifest in, facts out, URIs cited** — grounding prevents
   hallucinated predicates.
2. **Tools are discovered, not wired** — MCP makes the server the source
   of the agent's capabilities.
3. **The LLM plans, SPARQL executes** — deterministic execution with
   model-driven intent.