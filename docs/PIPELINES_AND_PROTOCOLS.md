# Pipeline Design & Protocol Specifications

AgentHub replaces chat with deterministic software factories built as Directed Acyclic Graphs (DAGs). This document outlines the node typology and how they communicate via the Model Context Protocol (MCP).

## 1. Directed Acyclic Graphs (DAGs)
A pipeline is a collection of nodes connected by directed edges. Data flows in one direction. Cycles (loops) are permitted *only* through explicitly defined `RouterNodes` acting as state-machine transitions, but the core data flow remains directed.

### Standard Node Typology
1.  **`LLMNode`:** The core compute unit. Takes text inputs, a system prompt, and invokes a specific model. Outputs generated text or function calls.
2.  **`ToolNode` (MCP Node):** Executes a specific action. Proxies inputs directly to an MCP Server (e.g., "Run SQL Query", "Read File") and returns the raw output.
3.  **`EvaluatorNode`:** A specialized LLM node that returns a strict Boolean (`true`/`false`) or a categorical score (e.g., `PASS`, `FAIL`, `NEEDS_REVISION`). Used for quality gates.
4.  **`RouterNode`:** Contains conditional logic to direct the flow of data. E.g., `if upstream.status == 'FAIL' route to Node B, else route to Node C`.
5.  **`HumanInTheLoopNode`:** Pauses the graph execution and alerts the UI. Waits for explicit human approval, text input, or modification before continuing.

## 2. The Model Context Protocol (MCP) Deep Dive
AgentHub is an MCP-native orchestrator.

### How it works:
1.  **Connection:** On startup, AgentHub connects to locally defined MCP servers (e.g., `npx @modelcontextprotocol/server-postgres`).
2.  **Discovery:** AgentHub sends a `tools/list` request. The Postgres MCP server responds with tools like `query_db`, `list_tables`.
3.  **Context Injection:** When an `LLMNode` is configured to have database access, AgentHub dynamically injects the JSON schemas of `query_db` and `list_tables` into the LLM's system prompt payload.
4.  **Execution:** 
    *   LLM generates a tool call: `{"name": "query_db", "arguments": {"sql": "SELECT * FROM users"}}`.
    *   AgentHub catches this, translates it to an MCP JSON-RPC call.
    *   MCP Server executes the query and returns the JSON result.
    *   AgentHub passes the JSON result back to the LLM as a tool response.

## 3. Example Factory: "The Autonomous PR Reviewer"
Here is how a real-world pipeline maps to our DAG:

1.  **Node A (`ToolNode`):** Triggers on a GitHub Webhook. Uses the Git MCP to run `git fetch` and `git diff`.
2.  **Node B (`RetrieveContextNode`):** Takes the diff from Node A, queries the local Vector DB to find the 5 most relevant architectural documentation files in the repository.
3.  **Node C (`LLMNode` - Analyst):** Takes the Diff + Context. System prompt: "Identify potential security vulnerabilities or architectural violations."
4.  **Node D (`EvaluatorNode` - Judge):** Evaluates Node C's output. Does it contain critical blockers? Outputs `PASS` or `FAIL`.
5.  **Node E (`RouterNode`):**
    *   If `PASS`, route to **Node F**.
    *   If `FAIL`, route to **Node G**.
6.  **Node F (`ToolNode`):** Uses GitHub MCP to post an "Approved" comment on the PR.
7.  **Node G (`ToolNode`):** Uses GitHub MCP to post a "Request Changes" review with the detailed breakdown from Node C.

This pipeline runs invisibly, deterministically, and with full observability.