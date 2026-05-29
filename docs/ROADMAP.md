# Zero-Debt Execution Plan & Product Roadmap (V2: Enterprise Pivot)

This is the exhaustive, hyper-granular master checklist for the new Daemon+Tauri+MCP architecture. Every checkbox represents a mandatory unit of work required to ship a world-class product.

## Phase 1: The Core Daemon & DAG Engine (Foundation)
- [ ] **Daemon Scaffolding**
    - [ ] Initialize workspace: `agenthub-core` (Rust library) and `agenthub-daemon` (Binary).
    - [ ] Setup `tokio` runtime, `tracing` for structured logging, and `clap` for daemon CLI flags.
- [ ] **The DAG Execution Engine**
    - [ ] Define `Node` and `Edge` traits/structs.
    - [ ] Implement `GraphRunner`: Topological sort of nodes to determine execution order.
    - [ ] Implement concurrency: Execute independent nodes in parallel using `tokio::spawn`.
    - [ ] Implement State Machine: Nodes must track state (`Pending`, `Running`, `Completed`, `Failed`).
- [ ] **Provider API Adapters**
    - [ ] Implement `LLMProvider` trait.
    - [ ] Build OpenAI adapter (streaming and standard).
    - [ ] Build Anthropic adapter (streaming and standard).
    - [ ] Build local adapter (Ollama/Llama.cpp integration).

## Phase 2: The Model Context Protocol (MCP) Integration
- [ ] **MCP Client Implementation**
    - [ ] Implement MCP JSON-RPC over `stdio` transport.
    - [ ] Implement MCP JSON-RPC over `SSE` (Server-Sent Events) transport.
- [ ] **Tool Discovery & Binding**
    - [ ] Logic to query an MCP Server for `tools/list`.
    - [ ] Dynamic schema translation: Convert MCP JSON schemas into OpenAI/Anthropic function calling formats.
- [ ] **Execution Routing**
    - [ ] Capture tool call requests from the LLM adapter.
    - [ ] Route the call to the correct MCP Server, await response, and inject back into the LLM context.

## Phase 3: The Intelligence & Context Layer (Local RAG)
- [ ] **Vector Database Embedded**
    - [ ] Integrate `lancedb` or `qdrant` as a local, embedded database.
- [ ] **Workspace Indexer**
    - [ ] Build a background file watcher (`notify` crate) to detect file changes.
    - [ ] Implement text chunking and AST parsing (using `tree-sitter`) for code chunks.
    - [ ] Generate local embeddings using `ort` (ONNX) and an optimized model like `all-MiniLM-L6-v2`.
- [ ] **Semantic Context Node**
    - [ ] Create a specialized DAG Node type: `RetrieveContextNode` that queries the Vector DB based on upstream inputs.

## Phase 4: The Tauri Rich Client (The Studio)
- [ ] **Tauri Scaffolding**
    - [ ] Initialize `create-tauri-app` with React, TypeScript, and TailwindCSS.
    - [ ] Configure `tauri.conf.json` for strict security and IPC permissions.
- [ ] **The Pipeline Builder (UI)**
    - [ ] Integrate `reactflow` for a node-based visual drag-and-drop editor.
    - [ ] Implement Node configuration sidebars (select model, edit system prompts).
    - [ ] Serialize visual graph to the AgentHub JSON Pipeline Schema.
- [ ] **The Execution Dashboard (UI)**
    - [ ] Real-time execution visualizer (highlighting active nodes in the graph).
    - [ ] Streaming log view (CI/CD style terminal output).
    - [ ] Rich Markdown rendering for final outputs (using `react-markdown` and `prismjs` for syntax).

## Phase 5: Enterprise Features & Polish
- [ ] **Time-Travel Debugging**
    - [ ] Implement a SQLite event store capturing every state delta.
    - [ ] UI slider to step backwards through the DAG execution history to see exact prompts/responses at any node.
- [ ] **Secret Management**
    - [ ] Integrate `keyring` crate to store Provider API keys securely in the OS vault.
- [ ] **Graceful Degradation & Resilience**
    - [ ] Automatic retry logic with exponential backoff for rate-limited API calls (429 errors).
    - [ ] Fallback routing (e.g., if Opus fails, fallback to Sonnet).

## Phase 6: Distribution & Ecosystem
- [ ] **Pipeline Registry**
    - [ ] Build a public GitHub repository of official Pipeline YAML files.
    - [ ] In-app "Marketplace" browser to 1-click install standard pipelines.
- [ ] **Binary Packaging**
    - [ ] GitHub Actions matrix for Windows `.msi`/`.exe`, macOS `.dmg` (Universal), Linux `.AppImage`/`.deb`.
    - [ ] macOS App Notarization pipeline.
    - [ ] Windows EV Code Signing pipeline.