# AgentHub Platform: Engineering Directives

**CRITICAL MANDATE:** We have pivoted. AgentHub is no longer a CLI wrapper or a TUI chat app. It is a headless Rust Daemon orchestrating DAG-based pipelines via the Model Context Protocol (MCP), accompanied by a Tauri/React desktop client. 

Read and strictly adhere to the following directives for all future development.

## 1. The Technology Stack
*   **Core Engine (Daemon):** Rust, Tokio (Async), Crossbeam (Concurrency), Axum/Tonic (if local API is needed).
*   **Desktop Client (GUI):** Tauri v2, React 18+, TypeScript, TailwindCSS, React Flow (for DAG visualization).
*   **Data & Memory:** SQLite (for execution audit logs and configuration), embedded Vector DB (LanceDB/Qdrant) for RAG.
*   **Protocol:** Model Context Protocol (MCP) is the *only* way tools are integrated. Do not write bespoke API wrappers for random tools.

## 2. Architectural Invariants (Do Not Violate)
1.  **Strict Decoupling:** The Rust Daemon must be entirely ignorant of the GUI. It must function perfectly as a headless background service. All interactions with the Daemon occur via strict IPC payloads or a local API.
2.  **No Screen Scraping:** Never spawn a child process to read its `stdout` as a way of interacting with an LLM. We only interface with LLMs via official REST APIs or official SDKs.
3.  **Deterministic State:** The state of an executing graph (Pipeline) must be serializable at any point. If the daemon crashes midway through a 10-node pipeline, it must be able to reboot, read the SQLite state, and resume exactly where it left off.
4.  **Security First:** Never log API keys. Never commit `.env` files. Ensure the local vector DB storage is locked to user permissions.

## 3. Development Workflow
*   Always start with defining the data schema (JSON/Structs) before implementing logic.
*   When implementing a new feature in the Rust Core, write unit tests for the discrete logic *before* wiring it up to the Tauri frontend.
*   Consult `about/ARCHITECTURE.md` and `docs/ROADMAP.md` before initiating any architectural changes.

**Note to Agents:** If a user requests a feature that involves "wrapping a CLI" or "adding a chat box", kindly remind them of the enterprise pivot towards Pipelines and MCP, and propose the solution in that paradigm.