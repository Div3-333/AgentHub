# Vision & Core Philosophy

## The Problem
In the modern AI-assisted development landscape, developers often find themselves "vibe coding" across multiple specialized agent CLIs (Gemini, Cursor, Codex, etc.). Managing these in separate terminal windows is cumbersome, fragmented, and prevents agents from effectively collaborating or peer-reviewing each other's work.

## The Solution: AgentHub
AgentHub is a "super control" orchestrator designed to unify these disparate agents into a single, cohesive group chat environment. By wrapping multiple CLI tools, AgentHub allows for:

1.  **Unified Context:** A single pane of glass for all agent interactions.
2.  **Agent Collaboration:** Enabling agents to prompt, challenge, and refine each other's output.
3.  **Advanced Orchestration:** Moving from simple 1:1 chat to complex multi-agent workflows.

## Core Aims
*   **Performance:** Minimal overhead using Rust.
*   **Extensibility:** Easy "Adapter" pattern for adding new CLI tools.
*   **Control:** Providing the user with director-level control over agent participation.
*   **Persistent Memory:** Ensuring that the "group vibe" and history are preserved for long-running projects.
