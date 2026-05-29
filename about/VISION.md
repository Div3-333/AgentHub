# The AgentHub Manifesto: The Hacker's Orchestrator

## 1. Executive Summary
AgentHub is built on a specific, powerful constraint: **Zero API Costs.**

The modern AI landscape is heavily gated behind expensive API paywalls. However, developers have access to incredibly powerful free-tier tools packaged as standalone CLIs (Gemini CLI, Cursor, Aider, GitHub Copilot CLI). 

AgentHub is an **Industrial-Grade CLI Wrapper and Orchestrator**. It does not use APIs. It acts as a "Super Terminal," invisibly launching your free-tier CLI tools in the background, hijacking their input/output streams, and forcing them into a unified, collaborative "Group Chat" interface.

## 2. The Core Challenge: Taming the Chaos
Wrapping interactive CLIs is notoriously difficult. They use colors, moving loading spinners, and unexpected prompts (`y/n`). They behave differently if they realize they are being automated.

AgentHub solves this by acting as a **Phantom Terminal**:
*   It uses Pseudo-Terminals (PTY) to trick the CLIs into thinking a human is typing.
*   It uses aggressive, state-machine-based ANSI stripping to clean up loading bars and colors into pure text.
*   It uses "Driver Profiles" to understand the specific quirks of each CLI (e.g., knowing exactly what the Gemini CLI "waiting for input" prompt looks like).

## 3. The User Experience
1.  You open AgentHub.
2.  You tell AgentHub: "Spin up Gemini and Codex."
3.  AgentHub silently launches `gemini-cli` and `codex-cli` as invisible background processes attached to PTYs.
4.  You type in the unified chat: *"@gemini write a rust script, then @codex review it."*
5.  AgentHub injects your prompt into Gemini's invisible terminal, reads the output, sanitizes it, displays it to you, and then injects that output into Codex's invisible terminal.

## 4. The Goal
To build a tool that feels like a premium, enterprise-grade multi-agent API platform, built entirely on top of free, local CLI wrappers. It is the ultimate hacker's orchestrator.