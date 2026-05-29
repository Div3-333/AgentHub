# AgentHub Phases 1-10 Orchestration

## Squad roster (Builder → Auditor)

| Phase | Builder scope | Auditor mandate |
|-------|---------------|-----------------|
| 1 | `crates/core` | DAG tests, migrations, no stubs |
| 2 | `crates/mcp` | JSON-RPC compliance, transport tests |
| 3 | `crates/provider` | Real API mapping, trait completeness |
| 4 | `crates/rag` | Indexer + embedder functional |
| 5 | `crates/daemon` | All REST routes + localhost security |
| 6 | `studio/` | Tauri + React Flow editor |
| 7 | `tests/`, CI hooks | clippy -D warnings, integration tests |
| 8 | `crates/daemon` security | keyring + scrubber |
| 9 | `.github/workflows` | lint + release pipelines |
| 10 | `docs/manual`, `docs/dev` | Complete user + dev guides |

## Rules

1. Builders complete ROADMAP checklist for their phase before auditor starts.
2. Auditors fix all gaps themselves if builder left debt.
3. `cargo test --workspace` must pass before phase sealed.
