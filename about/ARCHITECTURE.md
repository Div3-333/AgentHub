# System Architecture & Engineering Blueprint

This document defines the highly decoupled, enterprise-grade architecture of the AgentHub platform.

## 1. High-Level Topology
AgentHub operates on a Client-Server model running entirely on the user's local machine (Localhost).
1.  **AgentHub Core (Rust Daemon):** The background service managing graphs, state, memory, and API communication.
2.  **AgentHub Studio (Tauri/React GUI):** The rich frontend client.
3.  **MCP Tool Servers:** External local processes (e.g., a local Postgres DB, a local git interface) that expose tools to the Daemon via the Model Context Protocol.

```mermaid
graph TD
    UI[AgentHub Studio<br>(Tauri / React)] <-->|IPC / WebSockets| Core[AgentHub Core<br>(Rust Daemon)]
    Core <-->|API Calls| LLM[LLM Providers<br>(OpenAI, Anthropic, Local)]
    Core <-->|Model Context Protocol| MCP1[Local Filesystem MCP]
    Core <-->|Model Context Protocol| MCP2[GitHub/GitLab MCP]
    Core <-->|Local Embeddings| VDB[(Local Vector DB<br>Qdrant/LanceDB)]
```

## 2. Component 1: The Rust Daemon (The Engine)
The heart of AgentHub. Built for concurrent, lock-free execution of complex AI workflows.
*   **Runtime:** `tokio` (multi-threaded async runtime).
*   **State Management:** `crossbeam` channels and `Arc<RwLock<T>>` for managing the execution state of thousands of concurrent pipeline nodes.
*   **DAG Execution Engine:** A custom graph runner. It evaluates node dependencies, triggers execution when inputs are satisfied, and handles conditional edge routing based on node outputs.
*   **The RAG Subsystem:** Uses `ort` (ONNX Runtime) for local embedding generation (e.g., `all-MiniLM-L6-v2`) and interfaces with a local Vector DB (e.g., embedded LanceDB) to index the user's workspace for semantic context injection.

## 3. Component 2: The Presentation Layer (Tauri + React)
A modern, hardware-accelerated desktop application.
*   **Framework:** Tauri (Rust backend, web frontend) ensures tiny binary sizes and native OS integration.
*   **Frontend:** React with TypeScript, using TailwindCSS for styling and `reactflow` for the visual pipeline builder.
*   **IPC Bridge:** Communication between React and the Rust Daemon occurs via Tauri's IPC commands for discrete actions, and WebSockets for real-time streaming of token generation and pipeline execution logs.

## 4. Component 3: The Model Context Protocol (MCP) Mesh
AgentHub acts as an **MCP Client**. Instead of hardcoding tools (like "read file" or "search web"), AgentHub dynamically discovers tools via MCP Servers.
*   **Standardization:** When an Agent Node executes, it is provided a list of available MCP tools. The LLM decides to use a tool, the Rust Core translates this into an MCP JSON-RPC call, routes it to the correct MCP Server, and returns the result to the LLM.
*   **Sandboxing:** MCP Servers can run in isolated Docker containers, ensuring that if an LLM goes rogue, it cannot delete the user's root directory unless the MCP Server explicitly allows it.

## 5. Data Models & Schemas

### 5.1 The Pipeline Definition (JSON/YAML)
Pipelines are defined as declarative configurations, making them version-controllable and shareable.

```json
{
  "pipeline_id": "pr_reviewer_v1",
  "name": "Automated PR Review Factory",
  "nodes": [
    {
      "id": "fetch_diff",
      "type": "tool_node",
      "mcp_server": "github_mcp",
      "tool_name": "get_pr_diff"
    },
    {
      "id": "review_code",
      "type": "llm_node",
      "provider": "anthropic/claude-3-opus",
      "system_prompt": "You are a senior security auditor...",
      "inputs": ["fetch_diff.output"]
    },
    {
      "id": "decision_router",
      "type": "router_node",
      "logic": "if review_code.output contains 'BLOCKER' route to 'fail_pr', else 'approve_pr'"
    }
  ],
  "edges": [
    {"from": "fetch_diff", "to": "review_code"},
    {"from": "review_code", "to": "decision_router"}
  ]
}
```

## 6. Security & Sandboxing (Enterprise Posture)
*   **Secret Vault:** API keys are never stored in plaintext. They are encrypted using the OS's native secure enclave (macOS Keychain, Windows Credential Manager).
*   **Execution Auditing:** Every node execution generates a cryptographic hash of its inputs, prompt, and output, stored in an immutable local SQLite SQLite database (the Audit Log).