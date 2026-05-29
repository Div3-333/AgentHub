//! Agent initialization (Grand Induction) protocol (blueprint §8.4).

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::broadcast;

use crate::bus::BusEvent;
use crate::error::{AgentHubError, Result};
use crate::pty::{AgentPty, PtyStatus};
use crate::server::modes::{get_mode, simplified_induction, WorkspaceModeId};
use crate::server::state::ServerState;

const INDUCTION_TIMEOUT: Duration = Duration::from_secs(30);

fn induction_timeout() -> Duration {
    if let Ok(ms) = std::env::var("AGENTHUB_INDUCTION_TIMEOUT_MS") {
        if let Ok(n) = ms.parse::<u64>() {
            return Duration::from_millis(n);
        }
    }
    INDUCTION_TIMEOUT
}

#[must_use]
const fn workspace_mode_label(mode: WorkspaceModeId) -> &'static str {
    match mode {
        WorkspaceModeId::Dm => "Direct Message",
        WorkspaceModeId::GroupChat => "Group Chat",
        WorkspaceModeId::Server => "Server",
    }
}

/// Returns `true` when sanitized output contains an exact `READY` line (case-insensitive).
#[must_use]
pub fn output_contains_ready(text: &str) -> bool {
    text.lines()
        .any(|line| line.trim().eq_ignore_ascii_case("READY"))
}

/// Renders the grand induction prompt for `agent`.
#[must_use]
pub fn render_induction_prompt(agent: &AgentPty, state: &ServerState) -> String {
    let role = agent.role();
    let mandate = state
        .role_induction_overrides
        .get(&role)
        .map(|entry| entry.value().clone())
        .unwrap_or_else(|| format!("Fulfill the responsibilities of the {role} role."));

    let mode = get_mode(state);
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| ".".into());

    if simplified_induction(mode) {
        return format!(
            "You are now running inside AgentHub, a multi-agent orchestration system.\n\n\
=== YOUR IDENTITY ===\n\
Your name in this session is: @{tag}\n\
Your assigned role is: {role}\n\
Your role's mandate is: {mandate}\n\n\
=== ENVIRONMENT ===\n\
Workspace directory: {cwd}\n\
Workspace mode: Direct Message (single agent)\n\n\
=== YOUR ROLE BEHAVIOR ===\n\
{mandate}\n\n\
=== CRITICAL RULES ===\n\
1. You are in an automated environment. Do not ask for human confirmation for routine operations.\n\
2. Complete tasks without asking \"Is there anything else?\"\n\
3. Stay in your role unless instructed otherwise.\n\n\
Please acknowledge these instructions with only the word: \"READY\"\n",
            tag = agent.tag,
            role = role,
            mandate = mandate,
            cwd = cwd,
        );
    }

    let others: Vec<String> = state
        .agents
        .iter()
        .filter(|entry| entry.key() != &agent.id)
        .map(|entry| format!("@{} ({})", entry.value().tag, entry.value().role()))
        .collect();

    format!(
        "You are now running inside AgentHub, a multi-agent orchestration system.\n\n\
=== YOUR IDENTITY ===\n\
Your name in this session is: @{tag}\n\
Your assigned role is: {role}\n\
Your role's mandate is: {mandate}\n\n\
=== ENVIRONMENT ===\n\
AgentHub manages multiple AI CLI tools simultaneously. You are one of {total} active agents.\n\
Other agents currently online: {others}\n\
Workspace directory: {cwd}\n\
Workspace mode: {mode}\n\n\
=== COMMUNICATION PROTOCOL ===\n\
- When you see a message starting with \"[{{name}} says]: \", it means another agent or the user has addressed the group. Read and consider it carefully.\n\
- When you see \"[System]: \", it is an automated notification from AgentHub. Acknowledge it but do not repeat it.\n\
- When the user addresses you directly with @{tag}, respond to them specifically.\n\
- Keep your responses focused and concise. Do not include unnecessary preambles like \"Sure!\" or \"Of course!\".\n\
- Do not repeat what was just said to you. Jump directly to your response.\n\
- Do not output ANSI color codes or markdown decorations that render as raw escape sequences.\n\n\
=== YOUR ROLE BEHAVIOR ===\n\
{mandate}\n\n\
=== CRITICAL RULES ===\n\
1. You are in an automated environment. Do not ask for human confirmation for routine operations unless explicitly instructed.\n\
2. If you receive a task, complete it and signal completion by ending your response. Do not ask \"Is there anything else?\"\n\
3. If you are a Builder, write code. If you are a Reviewer, critique code. Stay in your role unless instructed otherwise.\n\
4. You may see your own previous responses quoted back to you. Do not be confused by this — it is context injection for continuity.\n\n\
Please acknowledge these instructions with only the word: \"READY\"\n",
        tag = agent.tag,
        total = state.agents.len(),
        others = if others.is_empty() {
            "(none)".to_string()
        } else {
            others.join(", ")
        },
        role = role,
        mandate = mandate,
        cwd = cwd,
        mode = workspace_mode_label(mode),
    )
}

/// Runs grand induction: inject prompt, wait for `READY`, emit lifecycle events.
///
/// On success: `AgentOnline` plus join system message. On timeout: logs
/// [`AgentHubError::InductionTimeout`], posts a failure system message, still
/// marks the agent idle and emits `AgentOnline` (graceful degradation).
pub async fn run_induction(
    agent: Arc<AgentPty>,
    state: Arc<ServerState>,
    bus_tx: broadcast::Sender<BusEvent>,
) {
    let prompt = render_induction_prompt(&agent, &state);
    let payload = format!("[System]: AGENTHUB induction\n{prompt}\n");
    if let Err(e) = agent.write_stdin(payload.as_bytes()) {
        tracing::warn!(agent = %agent.tag, "induction write failed: {e}");
    }

    let ring = agent.ring_buffer();
    let ready = tokio::time::timeout(induction_timeout(), async {
        loop {
            let bytes = ring.peek_all();
            let text = String::from_utf8_lossy(&bytes);
            if output_contains_ready(&text) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;

    let id = agent.id;
    let tag = agent.tag.clone();
    let role = agent.role();

    agent
        .status
        .store(PtyStatus::Idle.as_u8(), Ordering::Release);

    match ready {
        Ok(true) => {
            let _ = bus_tx.send(BusEvent::AgentOnline {
                id,
                tag: tag.clone(),
                role: role.clone(),
            });
            let _ = bus_tx.send(BusEvent::SystemMessage {
                content: format!("[System]: @{tag} ({role}) has joined the session."),
                timestamp: Utc::now(),
            });
        }
        _ => {
            tracing::warn!(
                agent = %tag,
                error = %AgentHubError::InductionTimeout(id),
                "induction timed out"
            );
            let _ = bus_tx.send(BusEvent::SystemMessage {
                content: format!(
                    "[System]: @{tag} failed to acknowledge induction. The agent may still function but is not context-aware. Kick and respawn if issues arise."
                ),
                timestamp: Utc::now(),
            });
            let _ = bus_tx.send(BusEvent::AgentOnline { id, tag, role });
        }
    }
}

/// Induction for tests: inject and poll until READY or timeout.
pub async fn run_induction_for_test(agent: Arc<AgentPty>, state: &ServerState) -> Result<()> {
    let prompt = render_induction_prompt(&agent, state);
    let payload = format!("[System]: AGENTHUB induction\n{prompt}\n");
    agent.write_stdin(payload.as_bytes())?;

    let ring = agent.ring_buffer();
    let deadline = tokio::time::Instant::now() + induction_timeout();
    while tokio::time::Instant::now() < deadline {
        let bytes = ring.peek_all();
        let text = String::from_utf8_lossy(&bytes);
        if output_contains_ready(&text) {
            agent
                .status
                .store(PtyStatus::Idle.as_u8(), Ordering::Release);
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(AgentHubError::InductionTimeout(agent.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::modes::{mode_to_u8, WorkspaceModeId};
    use std::sync::Arc;

    fn test_pty(tag: &str, role: &str) -> Arc<AgentPty> {
        let (agent, _capture) = crate::pty::manager::mock_agent_with_capture(
            uuid::Uuid::new_v4(),
            tag,
            PtyStatus::Initializing,
            true,
        );
        agent.set_role(role);
        agent
    }

    #[test]
    fn ready_detection_exact_line() {
        assert!(output_contains_ready("READY\n"));
        assert!(output_contains_ready("  ready  \r\n"));
        assert!(!output_contains_ready("NOT READY YET\n"));
        assert!(!output_contains_ready("READY TO GO\n"));
    }

    #[test]
    fn dm_prompt_omits_multi_agent_section() {
        let state = ServerState::new();
        state
            .mode
            .store(mode_to_u8(WorkspaceModeId::Dm), Ordering::Release);
        let agent = test_pty("solo-1", "Builder");
        let prompt = render_induction_prompt(&agent, &state);
        assert!(prompt.contains("Direct Message"));
        assert!(!prompt.contains("Other agents currently online"));
    }

    #[test]
    fn group_prompt_lists_environment() {
        let state = ServerState::new();
        let agent = test_pty("gemini-1", "Builder");
        let other = test_pty("other-1", "Reviewer");
        state.agents.insert(other.id, Arc::clone(&other));
        let prompt = render_induction_prompt(&agent, &state);
        assert!(prompt.contains("Other agents currently online"));
        assert!(prompt.contains("Please acknowledge"));
    }

    #[test]
    fn render_includes_role_override() {
        let state = ServerState::new();
        state.role_induction_overrides.insert(
            "Reviewer".to_string(),
            "Find security bugs only.".to_string(),
        );
        let agent = test_pty("rev-1", "Reviewer");
        let prompt = render_induction_prompt(&agent, &state);
        assert!(prompt.contains("Find security bugs only."));
    }
}
