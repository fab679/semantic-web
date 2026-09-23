# Semantic Graph Agent (Together AI + MCP)

A complete, runnable tutorial: an LLM agent that answers questions from a
semantic knowledge graph using **Together AI** for reasoning and the
semantic-web service's **MCP server** for its tools.

The design point: the agent hardcodes **nothing** about the graph. It
discovers its tools from the server (`tools/list`), reads the live
manifest (`resources/read` / `get_manifest`), and plans against what the
graph says exists.

```
  user ──question──► agent.py ──tool loop──► Together AI (Llama 3.3 70B)
                        │                        │  tool calls
                        │                        ▼
                        │                 MCP tools/call ──► semweb /mcp
                        │                                          │
                        └── /events (SSE, optional) ◄── publish ────┘
```

## Run it

```sh
# 1. The graph (if not already running)
docker compose up --build -d

# 2. The agent
pip install "together>=2.0.0" requests
export TOGETHER_API_KEY=your_key
python agent.py "Who works at Acme and what do we know about them?"

# Watch live graph changes while the agent works:
SEMWEB_SUBSCRIBE=1 python agent.py "What classes exist in this graph?"
```

## What you will see

```
connected: semantic-web (protocol 2025-06-18)
discovered 6 tools from the server: ['search_graph', 'sparql_query', ...]
  [tool] get_manifest({})
  [tool] search_graph({"predicate": "http://schema.org/worksFor", "limit": 20})

Agent: Acme Corp is an organization (foaf:Organization) founded on
2001-04-03. Two people work there: Alice (http://example.org/alice) and
Bob (http://example.org/bob). Alice knows Bob, and Bob knows Carol...
```

## Why this shape works for agents

1. **Discovery over memorization.** `tools/list` gives the agent its
   capabilities; the manifest gives it the vocabulary, human descriptions
   and SHACL constraints. A new predicate in the graph appears in the
   manifest on the next read — no redeploy.
2. **Execution stays deterministic.** The LLM plans; SPARQL and the
   fragment endpoint execute. The answer is grounded in URIs the tools
   actually returned.
3. **Real-time by subscription.** Point `subscribe` at an agent-side
   callback, or run with `SEMWEB_SUBSCRIBE=1` to watch `/events` — the
   agent sees the graph change live.
4. **Bounded.** Tool outputs are capped by the server (`_truncated`) and
   per-turn by the agent, so the context window stays manageable.