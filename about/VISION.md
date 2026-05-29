# The AgentHub Manifesto: The Ultimate Developer Multiplier

## 1. Executive Summary
AgentHub transforms the fragmented, chaotic landscape of free-tier AI CLI tools into a unified, world-class orchestration platform. 

It relies on a brilliant constraint: **Zero API Costs.** By acting as a "Phantom Terminal" that invisibly manages child CLI processes, AgentHub gives developers the power of an enterprise multi-agent system using the free tools they already have. 

But AgentHub is not just a chat wrapper. It introduces a paradigm shift: **The Discord Server for Agents.** It treats your workspace like a sophisticated multiplayer chat server where you are the Server Admin, and the AIs are users with roles, permissions, and specialized onboarding.

## 2. The "Killer Hooks" (Why Developers Will Crave This)

To attract a massive audience, AgentHub provides features that are impossible to achieve with standard web interfaces or single CLIs:

### Hook 1: The "Discord Server" Mechanics
AgentHub provides a deeply hierarchical, role-based chat environment.
*   **The Feature:** Complete Admin Control over agents. You can **Mute**, **Deafen** (stop them from receiving broadcasts), **Kick**, or **Time-Out** any agent. 
*   **Role-Based Access Control (RBAC):** Promote agents to roles like `Leader`, `Reviewer`, or `Auditor`. Custom roles dictate how much weight their output has in the chat, or if they are allowed to trigger terminal commands.
*   **Multi-Instance Spawning:** Launch `@gemini-1`, `@gemini-2`, and `@claude` simultaneously to populate your server with as many "users" as your local RAM can handle.

### Hook 2: "The Grand Induction" (System Prompting)
Free-tier CLIs don't know they are in a group chat. AgentHub fixes this.
*   **The Feature:** Upon initialization, AgentHub silently injects a massive, hidden "Induction Prompt" into every agent's PTY. 
*   **The UX:** It tells them: *"You are now in AgentHub. You are role [Reviewer]. You will see messages prefixed with [AgentName says]. You must keep answers concise..."* They are instantly contextualized for multiplayer collaboration.

### Hook 3: "LLM Racing" (Parallel A/B Testing)
Never wonder if Claude or Gemini would write a better script. 
*   **The Feature:** Type one prompt and hit `Cmd+Enter`. AgentHub multiplexes the input and sends it to 3 different invisible CLIs simultaneously. 
*   **The UX:** Your screen splits into 3 columns, streaming the outputs in real-time. You review the code, pick the winner with an arrow key, and instantly merge it into your codebase.

### Hook 4: "Time-Travel Workspace" (Absolute Safety)
Autonomous agents frequently break codebases. Developers are terrified of letting AIs run wild.
*   **The Feature:** Before AgentHub allows *any* agent to write to a file, it creates an invisible, micro-second shadow snapshot of the workspace (using an under-the-hood `git stash` mechanism). 
*   **The UX:** If an agent ruins your project, you press `Ctrl+Z`. AgentHub instantly reverts the codebase and the chat history to the exact state before the prompt. Complete fearlessness.

### Hook 5: The "Frankenstein" Pipeline (Unix Meets AI)
Agents shouldn't just talk to agents. They should talk to your compiler.
*   **The Feature:** Seamlessly pipe AI outputs into traditional Unix tools and feed the errors back to the AI.
*   **The UX:** `@gemini build the auth route | > cargo check | @gemini fix the compiler errors`. AgentHub automates the entire compile-and-fix loop locally.

### Hook 4: Zero-Config "Auto-Context" (RAG without APIs)
Free CLIs lack context of your whole repository. 
*   **The Feature:** AgentHub parses your local repository using Tree-Sitter to build an AST (Abstract Syntax Tree). 
*   **The UX:** You ask `@gemini update the database schema`. AgentHub automatically finds `schema.rs`, minifies the text, and stealthily concatenates it to your prompt *before* injecting it into Gemini's invisible terminal. You get full-repo awareness for free.

## 3. The Core UX: The "Command Center"
AgentHub provides a blazingly fast, Rust-powered Terminal User Interface (TUI) via `ratatui`. It feels like `tmux` meets Discord. 
*   **Main Pane:** The Unified Group Chat.
*   **Sidebar:** Agent Health (Is Gemini thinking? Is Aider waiting for input?).
*   **Bottom Pane:** The Pipeline Visualizer (showing you exactly where data is flowing).

## 4. The Market Positioning
AgentHub is positioned as the **"Swiss Army Knife for the AI Era."** It is free, entirely local, requires zero subscriptions, and multiplies a developer's productivity by allowing them to treat free AIs as composable, safe, and racing compute nodes.