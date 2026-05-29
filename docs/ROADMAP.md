# The Definitive AgentHub Architectural Blueprint (Enterprise V3)

This document is the exhaustive, component-by-component, struct-by-struct architectural specification for AgentHub. As a world-class product, we do not guess at implementation. Every module, trait, state slice, and test case is mapped below. Completion of this document represents a shippable, zero-debt Enterprise software factory.

## Phase 1: Core Daemon Architecture (`agenthub-core`)

### 1.1 Workspace Configuration
- [ ] Initialize Cargo Workspace.
- [ ] `[workspace]` members: `core`, `daemon`, `mcp`, `rag`, `xtask`.
- [ ] `[workspace.dependencies]` centralized version management.
- [ ] `tokio = "1.40"`
- [ ] `serde = "1.0"`
- [ ] `tracing = "0.1"`
- [ ] `crossbeam = "0.8"`

### 1.2 Global Error Handling (`agenthub-core/src/error.rs`)
- [ ] `enum AgentHubError`
  - [ ] `Variant: Config(String)`
  - [ ] `Variant: Io(std::io::Error)`
  - [ ] `Variant: Graph(GraphError)`
  - [ ] `Variant: Mcp(McpError)`
  - [ ] `Variant: Provider(ProviderError)`
  - [ ] `Variant: Database(sqlx::Error)`
- [ ] Implement `std::error::Error` for `AgentHubError`.
- [ ] Implement `From<std::io::Error>` for `AgentHubError`.
- [ ] Implement `From<sqlx::Error>` for `AgentHubError`.
- [ ] `type Result<T> = std::result::Result<T, AgentHubError>;`

### 1.3 The Directed Acyclic Graph (DAG) Engine (`agenthub-core/src/graph`)
- [ ] **Module: `types.rs`**
  - [ ] `struct GraphId(uuid::Uuid)`
  - [ ] `struct NodeId(uuid::Uuid)`
  - [ ] `struct EdgeId(uuid::Uuid)`
  - [ ] `enum NodeStatus`
    - [ ] `Pending`
    - [ ] `Running`
    - [ ] `Yielded`
    - [ ] `Completed`
    - [ ] `Failed(String)`
    - [ ] `Cancelled`
  - [ ] `struct ExecutionState`
    - [ ] `field: graph_id: GraphId`
    - [ ] `field: node_states: HashMap<NodeId, NodeStatus>`
    - [ ] `field: outputs: HashMap<NodeId, serde_json::Value>`
- [ ] **Module: `node.rs`**
  - [ ] `trait NodeExecutor: Send + Sync`
    - [ ] `async fn execute(&self, ctx: &ExecutionContext, inputs: Value) -> Result<Value>`
    - [ ] `fn node_type(&self) -> NodeType`
    - [ ] `fn validate_inputs(&self, inputs: &Value) -> Result<()>`
  - [ ] `enum NodeType`
    - [ ] `Llm(LlmConfig)`
    - [ ] `Tool(ToolConfig)`
    - [ ] `Router(RouterConfig)`
    - [ ] `Human(HumanConfig)`
- [ ] **Module: `edge.rs`**
  - [ ] `struct Edge`
    - [ ] `field: source: NodeId`
    - [ ] `field: target: NodeId`
    - [ ] `field: condition: Option<String> // JS/Expr logic`
    - [ ] `field: mapping: Option<Value> // JSONata transformation`
- [ ] **Module: `runner.rs`**
  - [ ] `struct GraphRunner`
    - [ ] `field: graph: DirectedGraph<NodeId, EdgeId>`
    - [ ] `field: state: Arc<RwLock<ExecutionState>>`
    - [ ] `field: tx: broadcast::Sender<GraphEvent>`
  - [ ] `impl GraphRunner`
    - [ ] `async fn start(&mut self) -> Result<()>`
    - [ ] `async fn step(&mut self) -> Result<bool>`
    - [ ] `fn topological_sort(&self) -> Result<Vec<NodeId>>`
    - [ ] `fn get_ready_nodes(&self) -> Vec<NodeId>`
    - [ ] `async fn execute_node(&self, id: NodeId) -> Result<()>`
  - [ ] **Unit Tests: `runner_tests.rs`**
    - [ ] `test_linear_graph_execution`
    - [ ] `test_branching_graph_execution`
    - [ ] `test_cycle_detection_fails_validation`
    - [ ] `test_conditional_edge_routing_true`
    - [ ] `test_conditional_edge_routing_false`

### 1.4 State Persistence (`agenthub-core/src/db.rs`)
- [ ] **Module: `sqlite.rs`**
  - [ ] `struct DbClient { pool: sqlx::SqlitePool }`
  - [ ] `async fn init_pool(url: &str) -> Result<DbClient>`
  - [ ] `async fn run_migrations(&self) -> Result<()>`
- [ ] **Migrations (`migrations/`)**
  - [ ] `001_initial_schema.sql`
    - [ ] `CREATE TABLE graphs (id TEXT PRIMARY KEY, name TEXT, definition JSON)`
    - [ ] `CREATE TABLE executions (id TEXT PRIMARY KEY, graph_id TEXT, status TEXT, started_at I64, completed_at I64)`
    - [ ] `CREATE TABLE node_logs (id TEXT PRIMARY KEY, exec_id TEXT, node_id TEXT, inputs JSON, outputs JSON, stdout TEXT)`

## Phase 2: Model Context Protocol Mesh (`agenthub-mcp`)

### 2.1 JSON-RPC 2.0 Base (`agenthub-mcp/src/jsonrpc.rs`)
- [ ] `struct Request { jsonrpc: String, id: Value, method: String, params: Option<Value> }`
- [ ] `struct Response { jsonrpc: String, id: Value, result: Option<Value>, error: Option<RpcError> }`
- [ ] `struct Notification { jsonrpc: String, method: String, params: Option<Value> }`
- [ ] `struct RpcError { code: i32, message: String, data: Option<Value> }`
- [ ] Implement `Serialize` / `Deserialize` for all RPC structs.

### 2.2 MCP Types (`agenthub-mcp/src/types.rs`)
- [ ] `struct InitializeRequestParams { protocolVersion: String, capabilities: ClientCapabilities, clientInfo: Implementation }`
- [ ] `struct InitializeResult { protocolVersion: String, capabilities: ServerCapabilities, serverInfo: Implementation }`
- [ ] `struct Tool { name: String, description: String, inputSchema: Value }`
- [ ] `struct CallToolRequestParams { name: String, arguments: Value }`
- [ ] `struct CallToolResult { content: Vec<Content>, isError: bool }`
- [ ] `enum Content`
  - [ ] `Text(TextContent)`
  - [ ] `Image(ImageContent)`
  - [ ] `Resource(ResourceContent)`

### 2.3 Transports (`agenthub-mcp/src/transport`)
- [ ] `trait Transport: Send + Sync`
  - [ ] `async fn send(&self, msg: String) -> Result<()>`
  - [ ] `async fn recv(&self) -> Result<String>`
- [ ] **Module: `stdio.rs`**
  - [ ] `struct StdioTransport`
    - [ ] `field: child: tokio::process::Child`
    - [ ] `field: stdin: tokio::io::WriteHalf<ChildStdin>`
    - [ ] `field: stdout: tokio::io::Lines<BufReader<ChildStdout>>`
  - [ ] `impl Transport for StdioTransport`
- [ ] **Module: `sse.rs`**
  - [ ] `struct SseTransport`
    - [ ] `field: client: reqwest::Client`
    - [ ] `field: endpoint: String`
  - [ ] `impl Transport for SseTransport`

### 2.4 MCP Client (`agenthub-mcp/src/client.rs`)
- [ ] `struct McpClient`
  - [ ] `field: transport: Box<dyn Transport>`
  - [ ] `field: request_id: AtomicU64`
  - [ ] `field: pending_requests: Arc<DashMap<u64, oneshot::Sender<Response>>>`
- [ ] `impl McpClient`
  - [ ] `async fn connect(transport: Box<dyn Transport>) -> Result<Self>`
  - [ ] `async fn initialize(&self) -> Result<InitializeResult>`
  - [ ] `async fn list_tools(&self) -> Result<Vec<Tool>>`
  - [ ] `async fn call_tool(&self, name: &str, args: Value) -> Result<CallToolResult>`
- [ ] **Unit Tests: `client_tests.rs`**
  - [ ] `test_handshake_flow`
  - [ ] `test_tool_list_parsing`
  - [ ] `test_concurrent_request_dispatch`

## Phase 3: The Intelligence Provider Layer (`agenthub-provider`)

### 3.1 Provider Abstractions (`agenthub-provider/src/lib.rs`)
- [ ] `struct Prompt { system: Option<String>, messages: Vec<Message>, tools: Vec<Tool> }`
- [ ] `enum Message { User(String), Assistant(String), ToolResult(String) }`
- [ ] `struct Completion { content: String, tool_calls: Vec<ToolCall>, usage: UsageStats }`
- [ ] `trait LlmProvider: Send + Sync`
  - [ ] `async fn generate(&self, prompt: Prompt) -> Result<Completion>`
  - [ ] `async fn stream(&self, prompt: Prompt) -> BoxStream<Result<Chunk>>`

### 3.2 OpenAI Implementation (`agenthub-provider/src/openai.rs`)
- [ ] `struct OpenAiProvider { api_key: String, client: reqwest::Client }`
- [ ] `impl LlmProvider for OpenAiProvider`
- [ ] Data mapper: `Prompt` -> OpenAI `/v1/chat/completions` request.
- [ ] Data mapper: OpenAI Tool Schema <-> MCP Tool Schema.

### 3.3 Anthropic Implementation (`agenthub-provider/src/anthropic.rs`)
- [ ] `struct AnthropicProvider { api_key: String, client: reqwest::Client }`
- [ ] `impl LlmProvider for AnthropicProvider`
- [ ] Data mapper: `Prompt` -> Anthropic Messages API request.
- [ ] Anthropic strictly requires `x-api-key` and `anthropic-version` headers.

## Phase 4: Local RAG Subsystem (`agenthub-rag`)

### 4.1 Vector Database (`agenthub-rag/src/db.rs`)
- [ ] `struct VectorDb { client: lancedb::Connection }`
- [ ] `async fn open_table(name: &str) -> Result<Table>`
- [ ] `struct DocumentChunk { id: String, path: String, text: String, vector: Vec<f32> }`
- [ ] `async fn upsert_chunks(&self, chunks: Vec<DocumentChunk>) -> Result<()>`
- [ ] `async fn semantic_search(&self, vector: Vec<f32>, limit: usize) -> Result<Vec<DocumentChunk>>`

### 4.2 Workspace Indexer (`agenthub-rag/src/indexer.rs`)
- [ ] `struct Indexer { db: VectorDb, embedder: Embedder }`
- [ ] `async fn scan_workspace(&self, path: &Path) -> Result<()>`
- [ ] File ignored logic (`.gitignore`, `.dockerignore` parsing via `ignore` crate).
- [ ] AST-based chunking using `tree-sitter` for Rust, TS, Python, Go.
- [ ] Fallback: Recursive character splitting for `.md`, `.txt`.

### 4.3 ONNX Embedder (`agenthub-rag/src/embedder.rs`)
- [ ] `struct Embedder { session: ort::Session }`
- [ ] Auto-download weights for `all-MiniLM-L6-v2` to `~/.agenthub/models/`.
- [ ] `fn encode(&self, text: &str) -> Result<Vec<f32>>`
  - [ ] Tokenize text using `hf-hub` tokenizer.
  - [ ] Forward pass through ONNX session.
  - [ ] Mean pooling & L2 normalization of output tensors.

## Phase 5: The Headless Daemon Server (`agenthub-daemon`)

### 5.1 Axum Local Server (`agenthub-daemon/src/server.rs`)
- [ ] `struct AppState { graph_runner: Arc<GraphRunner>, db: DbClient, mcp_registry: McpRegistry }`
- [ ] `fn create_router(state: AppState) -> Router`
- [ ] **Routes:**
  - [ ] `POST /api/v1/graphs` -> Upload a new graph JSON.
  - [ ] `POST /api/v1/graphs/:id/execute` -> Start execution.
  - [ ] `GET /api/v1/executions/:id` -> Get execution status.
  - [ ] `GET /api/v1/executions/:id/stream` -> SSE endpoint for real-time node logs.
  - [ ] `GET /api/v1/mcp/servers` -> List connected MCP servers.
  - [ ] `POST /api/v1/mcp/connect` -> Connect a new MCP server.

### 5.2 Server Security
- [ ] Bind strictly to `127.0.0.1` (Localhost only).
- [ ] Implement CORS middleware restricted to `tauri://localhost`.
- [ ] Generates a secure session token on startup, written to `~/.agenthub/daemon.token`, which the UI must pass in the `Authorization` header.

## Phase 6: Tauri Studio Frontend (`agenthub-studio`)

### 6.1 Frontend Tooling Setup
- [ ] `package.json` setup.
- [ ] `vite.config.ts` setup.
- [ ] Install React 18, React DOM, TypeScript.
- [ ] Install TailwindCSS, PostCSS, Autoprefixer.
- [ ] Install Shadcn UI components (Radix primitives).
- [ ] Install `zustand` for state management.
- [ ] Install `reactflow` for DAG visualization.

### 6.2 Core UI Layout (`src/layouts/MainLayout.tsx`)
- [ ] Sidebar Navigation (Pipelines, Executions, MCP Servers, Settings).
- [ ] Top Header (Daemon connection status, active environment).
- [ ] Main Content Area (Dynamic routing).

### 6.3 Pipeline Visual Editor (`src/features/editor/PipelineEditor.tsx`)
- [ ] Implement `ReactFlow` canvas.
- [ ] **Custom Nodes:**
  - [ ] `LLMNodeComponent.tsx`: Shows provider logo, model selection dropdown, system prompt preview.
  - [ ] `ToolNodeComponent.tsx`: Shows MCP server icon, tool selection dropdown.
  - [ ] `RouterNodeComponent.tsx`: Shows conditional logic inputs.
- [ ] **Custom Edges:**
  - [ ] Animated edges during execution.
  - [ ] Colored edges (Green=Pass, Red=Fail logic).
- [ ] Sidebar Property Panel (`src/features/editor/PropertyPanel.tsx`)
  - [ ] Select a node to edit its specific JSON payload.
  - [ ] Real-time JSON validation against schema.

### 6.4 Execution Dashboard (`src/features/execution/Dashboard.tsx`)
- [ ] Split pane: Left side shows Read-Only graph, Right side shows Log Stream.
- [ ] Hook into Daemon SSE endpoint `/api/v1/executions/:id/stream`.
- [ ] **Component:** `LogViewer.tsx`
  - [ ] Virtualized list for performance (using `@tanstack/react-virtual`).
  - [ ] Syntax highlighting for output code blocks using `prismjs`.
  - [ ] Auto-scroll to bottom toggle.

### 6.5 MCP Server Manager (`src/features/mcp/ServerManager.tsx`)
- [ ] Table displaying all connected MCP servers.
- [ ] Status indicators (Connected, Disconnected, Error).
- [ ] Button: "Add Server" -> Modal to input command (e.g., `npx @modelcontextprotocol/server-postgres --db url`).
- [ ] View exposed tools per server.

### 6.6 Global State Slices (`src/store/`)
- [ ] `useDaemonStore.ts`: Manages connection state, auth token.
- [ ] `useGraphStore.ts`: Manages current editor nodes, edges, undo/redo stack.
- [ ] `useExecutionStore.ts`: Manages active execution logs, node statuses.

## Phase 7: Quality Assurance & Testing

### 7.1 Rust Test Matrix
- [ ] Run `cargo clippy --workspace -- -D warnings`.
- [ ] Run `cargo fmt --all -- --check`.
- [ ] Implement `tarpaulin` for coverage. Mandate >85% coverage on `agenthub-core` and `agenthub-mcp`.
- [ ] **Integration Tests (`tests/`)**
  - [ ] `test_full_pipeline_execution.rs`: Spawns a dummy MCP server, executes a graph, asserts output.
  - [ ] `test_mcp_protocol_compliance.rs`: Validates JSON-RPC strictness.
  - [ ] `test_rag_vector_upsert.rs`: Validates LanceDB insertion and retrieval accuracy.

### 7.2 Frontend Test Matrix
- [ ] Install `vitest` and `@testing-library/react`.
- [ ] Component tests for `LogViewer` rendering.
- [ ] Component tests for `ReactFlow` node rendering.
- [ ] E2E tests using Playwright:
  - [ ] `create_pipeline.spec.ts`: Simulates drag-and-drop creation of a graph.
  - [ ] `run_pipeline.spec.ts`: Simulates execution and log verification.

## Phase 8: Enterprise Security Posture

### 8.1 Key Management
- [ ] Never store `OPENAI_API_KEY` or `ANTHROPIC_API_KEY` in plain text files.
- [ ] Use `keyring` crate in Rust to store keys in macOS Keychain / Windows Credential Manager / Linux Secret Service.
- [ ] UI prompts for key, sends securely over localhost IPC, Rust commits to Vault.

### 8.2 Log Scrubbing
- [ ] Implement a regex-based `Scrubber` struct in the Daemon.
- [ ] Intercept all `stdout`/`stderr` from MCP servers.
- [ ] Mask detected secrets (`sk-ant-api03-*`, `sk-[a-zA-Z0-9]{48}`, etc.) before writing to SQLite Audit Log.

## Phase 9: Packaging & Deployment (CI/CD)

### 9.1 GitHub Actions Workflows (`.github/workflows/`)
- [ ] **`lint_and_test.yml`**: Runs clippy, fmt, tests, and vitest on every PR.
- [ ] **`build_release.yml`**: Triggers on tags (`v*`).
  - [ ] Job: `build-windows` (target: `x86_64-pc-windows-msvc`).
  - [ ] Job: `build-macos` (target: `aarch64-apple-darwin` and `x86_64-apple-darwin`).
  - [ ] Job: `build-linux` (target: `x86_64-unknown-linux-gnu`).

### 9.2 Tauri Build Optimization
- [ ] `Cargo.toml` release profile:
  - [ ] `opt-level = 3`
  - [ ] `lto = true`
  - [ ] `codegen-units = 1`
  - [ ] `strip = true`
- [ ] Tauri updater integration (generate signature files for `.tar.gz` and `.zip`).

### 9.3 Code Signing
- [ ] Windows: Sign `.msi` and `.exe` using Azure Key Vault or physical HSM EV Certificate.
- [ ] macOS: Automate `rcodesign` and Apple Notarization payload submission within the CI runner.

## Phase 10: Documentation & Governance

### 10.1 User Manual (`docs/manual/`)
- [ ] `getting_started.md`: Installation and 60-second quickstart.
- [ ] `concepts.md`: Explaining DAGs, Nodes, and MCP.
- [ ] `building_pipelines.md`: UI guide to the React Flow editor.
- [ ] `mcp_servers.md`: How to attach local DBs, Git, and custom scripts.

### 10.2 Developer Guide (`docs/dev/`)
- [ ] `architecture.md`: High-level system overview.
- [ ] `contributing.md`: PR guidelines, commit message formatting.
- [ ] `mcp_spec.md`: Internal documentation on how AgentHub routes MCP calls.

## Definition of Done (DoD)
The product is declared "World-Class" and ready for public launch **only** when every single checkbox from Phase 1.1 to 10.2 is verified, code-reviewed, and merged to the `main` branch. No technical debt will be carried into v1.0.