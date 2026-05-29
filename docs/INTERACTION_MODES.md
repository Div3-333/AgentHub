# Agent Interaction Modes

AgentHub supports multiple ways for agents to interact, allowing the user to switch between "Director" and "Observer" roles.

## 1. Explicit Routing (Tagging)
*   **How it works:** User explicitly calls agents using the `@` symbol.
*   **Example:** `@gemini brainstorm a fix, then @cursor implement it.`
*   **Use Case:** Precise control over who does what.

## 2. Autonomous Orchestration
*   **How it works:** A designated "Moderator" agent monitors the chat and decides which agent is best suited to respond next.
*   **Example:** User asks a general question, Moderator assigns it to `@specialist`.
*   **Use Case:** Natural-feeling collaboration and brainstorming.

## 3. Pipeline / Sequential
*   **How it works:** Outputs of one agent are automatically fed as inputs to the next in a pre-defined chain.
*   **Example:** `Write -> Review -> Test` chain.
*   **Use Case:** Automated code generation and verification cycles.

## 4. Collaborative / Competitive
*   **How it works:** Multiple agents are prompted with the same task and asked to critique each other's solutions.
*   **Use Case:** Higher quality output through peer review and "Adversarial" prompting.
