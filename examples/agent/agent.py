#!/usr/bin/env python3
"""
Semantic Graph Agent -- a complete, runnable tutorial example.

An LLM agent (Together AI) that talks to the semantic-web service through
its MCP server. The interesting design point: the agent does NOT hardcode
any graph knowledge. It discovers its tools via MCP `tools/list`, reads
the live agent manifest via the `get_manifest` tool, and plans against
what the graph says exists.

Flow per user question:
    1. MCP initialize + tools/list  -> the agent's toolset (from the server)
    2. get_manifest                 -> grounding: which terms exist, what
                                       they mean, SHACL constraints
    3. Together AI tool loop        -> search_graph / sparql_query /
                                       get_topic / insert_triple / subscribe
    4. Answer, citing URIs it actually saw

Requirements:
    pip install "together>=2.0.0" requests

Environment:
    TOGETHER_API_KEY   your Together AI key
    SEMWEB_URL         default http://localhost:8484 (dedicated project port)
    SEMWEB_MODEL       default meta-llama/Llama-3.3-70B-Instruct-Turbo
    SEMWEB_WRITE_TOKEN write token, if the deployment gates writes (the
                       agent attaches it to insert_triple calls)
    SEMWEB_SUBSCRIBE   set to 1 to also listen on /events (SSE) and print
                       live graph-change notifications while the agent runs

Usage:
    python agent.py "Who works at Acme and what do we know about them?"
"""

import json
import os
import sys
import threading

import requests
from together import Together

SEMWEB = os.environ.get("SEMWEB_URL", "http://localhost:8484")
MODEL = os.environ.get("SEMWEB_MODEL", "meta-llama/Llama-3.3-70B-Instruct-Turbo")
WRITE_TOKEN = os.environ.get("SEMWEB_WRITE_TOKEN")
TOOL_LOOP_BUDGET = 8
MAX_TOOL_RESULT_CHARS = 4000


# ---------------------------------------------------------------------------
# Minimal MCP client over the streamable-HTTP POST transport
# ---------------------------------------------------------------------------
class McpClient:
    def __init__(self, base_url: str):
        self.url = f"{base_url}/mcp"
        self._id = 0

    def _rpc(self, method: str, params: dict | None) -> dict:
        self._id += 1
        payload: dict = {"jsonrpc": "2.0", "id": self._id, "method": method}
        if params is not None:
            payload["params"] = params
        r = requests.post(self.url, json=payload, timeout=30)
        r.raise_for_status()
        return r.json()

    def initialize(self) -> dict:
        result = self._rpc("initialize", {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "semweb-tutorial-agent", "version": "0.1.0"},
        })["result"]
        # notification: no response expected (202)
        requests.post(self.url, json={"jsonrpc": "2.0",
                                      "method": "notifications/initialized"},
                      timeout=10)
        return result

    def list_tools(self) -> list[dict]:
        return self._rpc("tools/list", {})["result"]["tools"]

    def call_tool(self, name: str, arguments: dict) -> str:
        res = self._rpc("tools/call", {"name": name, "arguments": arguments})["result"]
        text = "\n".join(c.get("text", "") for c in res.get("content", []))
        return f"TOOL ERROR: {text}" if res.get("isError") else text

    def read_manifest_resource(self) -> str:
        res = self._rpc("resources/read",
                        {"uri": "manifest://semantic-web/current"})["result"]
        return res["contents"][0]["text"]

    def get_prompt(self, name: str, arguments: dict | None = None) -> str:
        params: dict = {"name": name}
        if arguments:
            params["arguments"] = arguments
        messages = self._rpc("prompts/get", params)["result"]["messages"]
        return "\n\n".join(m["content"]["text"] for m in messages)


# ---------------------------------------------------------------------------
# Together AI <-> MCP tool bridge.
#
# The agent's tools are DISCOVERED from the MCP server (tools/list) -- the
# JSON Schemas pass through nearly unchanged, because Together AI uses the
# same OpenAI-style function schema shape. Adding a tool on the server
# extends the agent without touching this file.
# ---------------------------------------------------------------------------
def to_openai_tool(mcp_tool: dict) -> dict:
    return {
        "type": "function",
        "function": {
            "name": mcp_tool["name"],
            "description": mcp_tool["description"],
            "parameters": mcp_tool["inputSchema"],
        },
    }


def make_system_prompt(manifest: str, guidance: str) -> str:
    return (
        "You are a knowledge-graph assistant. You answer questions using ONLY "
        "the semantic graph behind your tools.\n\n"
        "The graph's live manifest is attached below: it lists which classes "
        "and predicates exist, what they mean (descriptions), their SHACL "
        "constraints, cardinalities, and example SPARQL queries.\n\n"
        "Rules:\n"
        "- Ground every answer in URIs you actually saw in tool output.\n"
        "- Use search_graph for lookups; use sparql_query for anything the "
        "fragments cannot express; the manifest's exampleQuery is a good "
        "starting point.\n"
        "- If the graph does not contain the answer, say so explicitly.\n\n"
        + (f"Method guidance:\n{guidance}\n\n" if guidance else "")
        + f"GRAPH MANIFEST (the fingerprint identifies this exact ontology):\n{manifest}"
    )


def run_agent(mcp: McpClient, question: str) -> None:
    client = Together()  # reads TOGETHER_API_KEY

    # Grounding first: manifest resource + a guided prompt from the server.
    manifest = mcp.read_manifest_resource()
    guidance = mcp.get_prompt("explore_graph", {"focus": "the user's question"})
    tools = [to_openai_tool(t) for t in mcp.list_tools()]

    messages = [
        {"role": "system", "content": make_system_prompt(manifest, guidance)},
        {"role": "user", "content": question},
    ]

    for _ in range(TOOL_LOOP_BUDGET):
        response = client.chat.completions.create(
            model=MODEL,
            messages=messages,
            tools=tools,
        )
        choice = response.choices[0]
        if choice.finish_reason != "tool_calls" or not choice.message.tool_calls:
            print(f"\nAgent: {choice.message.content}")
            return

        messages.append(choice.message)
        for tc in choice.message.tool_calls:
            name = tc.function.name
            try:
                args = json.loads(tc.function.arguments)
            except json.JSONDecodeError:
                args = {}
            # Write authentication: when the deployment gates writes, the
            # agent carries its own credential and the MCP server validates
            # it (insert_triple would fail with a TOOL ERROR otherwise).
            if name == "insert_triple" and WRITE_TOKEN and "token" not in args:
                args["token"] = WRITE_TOKEN
            print(f"  [tool] {name}({json.dumps(args)[:120]})")
            result = mcp.call_tool(name, args)
            messages.append({
                "role": "tool",
                "tool_call_id": tc.id,
                "content": result[:MAX_TOOL_RESULT_CHARS],
            })

    print("\nAgent: [tool loop budget exhausted -- try a narrower question]")


def watch_events() -> None:
    """Print live /events (SSE) notifications in the background -- shows
    that the graph is real-time without the agent having to poll."""
    try:
        with requests.get(f"{SEMWEB}/events", stream=True, timeout=None) as r:
            for line in r.iter_lines():
                if line.startswith("data:"):
                    print(f"  [events] {line[5:].strip()}")
    except requests.RequestException as e:
        print(f"  [events] unavailable: {e}")


def main() -> None:
    question = " ".join(sys.argv[1:]) or "What does the graph know about people?"

    if os.environ.get("TOGETHER_API_KEY") is None:
        sys.exit("Set TOGETHER_API_KEY (get one at https://api.together.ai)")

    mcp = McpClient(SEMWEB)
    info = mcp.initialize()
    print(f"connected: {info['serverInfo']['name']} "
          f"(protocol {info['protocolVersion']})")
    tools = mcp.list_tools()
    print(f"discovered {len(tools)} tools from the server: "
          f"{[t['name'] for t in tools]}")

    if os.environ.get("SEMWEB_SUBSCRIBE") == "1":
        threading.Thread(target=watch_events, daemon=True).start()

    run_agent(mcp, question)


if __name__ == "__main__":
    main()