//! Autonomous agent loop / Sparring (blueprint §10.3).

use chrono::Utc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::bus::BusEvent;
use crate::config::AgentHubConfig;
use crate::db::DbClient;
use crate::error::{AgentHubError, Result};
use crate::pty::AgentPty;
use crate::server::modes;
use crate::server::ServerState;
use crate::vfs::{create_snapshot, ensure_session, SnapshotTrigger};

use super::executor::{
    ensure_agent_rbac, find_agent_by_tag, line_ending, wait_for_agent_message, AGENT_STAGE_TIMEOUT,
};

/// User abort flag (Escape key in TUI sets this).
pub static SPAR_ABORT: AtomicBool = AtomicBool::new(false);

/// Default sparring turns when `--turns` is omitted.
pub const DEFAULT_SPAR_TURNS: u8 = 5;
/// Maximum allowed sparring turns.
pub const MAX_SPAR_TURNS: u8 = 20;
/// Stagnation similarity threshold (blueprint §10.3).
pub const STAGNATION_RATIO: f64 = 0.95;

/// Parsed `/spar` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SparConfig {
    pub agent_a_tag: String,
    pub agent_a_role: String,
    pub agent_b_tag: String,
    pub agent_b_role: String,
    pub max_turns: u8,
    pub goal: String,
}

/// Outcome of a sparring session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SparResult {
    pub turns_completed: u8,
    pub aborted: bool,
    pub stagnation: bool,
    pub last_output: String,
}

/// Parse `/spar @a as RoleA vs @b as RoleB [--turns N] [--goal "..."]`.
pub fn parse_spar_command(input: &str) -> Result<SparConfig> {
    let trimmed = input.trim();
    if !trimmed.to_ascii_lowercase().starts_with("/spar") {
        return Err(AgentHubError::Config("not a /spar command".into()));
    }

    let rest = trimmed.strip_prefix("/spar").unwrap_or(trimmed).trim();
    let vs_idx = rest
        .to_ascii_lowercase()
        .find(" vs ")
        .ok_or_else(|| AgentHubError::Config("spar command missing ' vs '".into()))?;

    let left = rest[..vs_idx].trim();
    let right_and_flags = rest[vs_idx + 4..].trim();

    let (right, flags) = split_flags(right_and_flags);
    let (agent_a_tag, agent_a_role) = parse_agent_role(left)?;
    let (agent_b_tag, agent_b_role) = parse_agent_role(right)?;

    let mut max_turns = DEFAULT_SPAR_TURNS;
    let mut goal = String::new();

    let mut parts = flags.split_whitespace();
    while let Some(token) = parts.next() {
        if token == "--turns" {
            let n = parts
                .next()
                .ok_or_else(|| AgentHubError::Config("spar --turns requires a value".into()))?;
            max_turns = n
                .parse::<u8>()
                .map_err(|_| AgentHubError::Config(format!("invalid --turns value: {n}")))?;
        } else if token == "--goal" {
            goal = parse_quoted_goal(&mut parts)?;
        }
    }

    if max_turns == 0 || max_turns > MAX_SPAR_TURNS {
        return Err(AgentHubError::Config(format!(
            "spar turns must be 1..={MAX_SPAR_TURNS}"
        )));
    }

    if goal.is_empty() {
        return Err(AgentHubError::Config("spar requires --goal".into()));
    }

    Ok(SparConfig {
        agent_a_tag,
        agent_a_role,
        agent_b_tag,
        agent_b_role,
        max_turns,
        goal,
    })
}

fn split_flags(right_and_flags: &str) -> (&str, &str) {
    if let Some(idx) = right_and_flags.find("--") {
        let (agent_part, flags) = right_and_flags.split_at(idx);
        (agent_part.trim(), flags.trim())
    } else {
        (right_and_flags, "")
    }
}

fn parse_agent_role(segment: &str) -> Result<(String, String)> {
    let segment = segment.trim();
    let tag = segment
        .strip_prefix('@')
        .ok_or_else(|| AgentHubError::Config("spar agent must start with @".into()))?;
    let as_idx = tag
        .to_ascii_lowercase()
        .find(" as ")
        .ok_or_else(|| AgentHubError::Config("spar agent missing ' as ' role label".into()))?;
    let (name, role) = tag.split_at(as_idx);
    Ok((name.trim().to_string(), role[4..].trim().to_string()))
}

fn parse_quoted_goal<'a, I>(parts: &mut I) -> Result<String>
where
    I: Iterator<Item = &'a str>,
{
    let first = parts
        .next()
        .ok_or_else(|| AgentHubError::Config("spar --goal requires a value".into()))?;
    if first.starts_with('"') {
        let mut goal = first.trim_start_matches('"').to_string();
        if goal.ends_with('"') && goal.len() > 1 {
            goal.pop();
            return Ok(goal);
        }
        for token in parts {
            goal.push(' ');
            goal.push_str(token);
            if token.ends_with('"') {
                goal.pop();
                return Ok(goal);
            }
        }
        return Err(AgentHubError::Config("unterminated --goal string".into()));
    }
    Ok(first.to_string())
}

/// Runs a sparring session between two agents.
pub struct SparEngine {
    state: Arc<ServerState>,
    bus_tx: broadcast::Sender<BusEvent>,
    cwd: std::path::PathBuf,
    session_id: Uuid,
    config: AgentHubConfig,
    db: Option<Arc<DbClient>>,
}

impl SparEngine {
    #[must_use]
    pub fn new(
        state: Arc<ServerState>,
        bus_tx: broadcast::Sender<BusEvent>,
        cwd: impl Into<std::path::PathBuf>,
        session_id: Uuid,
        config: AgentHubConfig,
        db: Option<Arc<DbClient>>,
    ) -> Self {
        Self {
            state,
            bus_tx,
            cwd: cwd.into(),
            session_id,
            config,
            db,
        }
    }

    pub async fn run(&self, config: &SparConfig) -> Result<SparResult> {
        SPAR_ABORT.store(false, Ordering::SeqCst);

        if let Some(db) = &self.db {
            ensure_session(&db.pool, self.session_id, &self.cwd).await?;
            let shadow = if self.config.shadow_dir.is_absolute() {
                self.config.shadow_dir.clone()
            } else {
                self.cwd.join(&self.config.shadow_dir)
            };
            create_snapshot(
                &db.pool,
                &self.cwd,
                &shadow,
                self.session_id,
                SnapshotTrigger::Sparring,
                Some(&self.bus_tx),
            )
            .await?;
        }

        if poll_spar_abort().await {
            emit_system(&self.bus_tx, "[Spar]: Manually aborted by user.");
            return Ok(SparResult {
                turns_completed: 0,
                aborted: true,
                stagnation: false,
                last_output: config.goal.clone(),
            });
        }

        let (id_a, agent_a) = find_agent_by_tag(&self.state, &config.agent_a_tag, 0)?;
        let (id_b, agent_b) = find_agent_by_tag(&self.state, &config.agent_b_tag, 0)?;
        ensure_agent_rbac(&self.state, &agent_a);
        ensure_agent_rbac(&self.state, &agent_b);
        modes::enforce_send_messages(&self.state, id_a)?;
        modes::enforce_send_messages(&self.state, id_b)?;

        inject_role_prompt(
            &agent_a,
            &config.agent_a_role,
            &config.agent_a_role,
            &config.goal,
            true,
        )?;
        inject_role_prompt(
            &agent_b,
            &config.agent_b_role,
            &config.agent_a_role,
            &config.goal,
            false,
        )?;

        let mut bus_rx = self.bus_tx.subscribe();
        let mut last_output = config.goal.clone();
        let mut turn: u8 = 0;
        let mut history_a: Vec<String> = Vec::new();
        let mut history_b: Vec<String> = Vec::new();
        let mut aborted = false;
        let mut stagnation = false;

        while turn < config.max_turns {
            if SPAR_ABORT.load(Ordering::SeqCst) {
                aborted = true;
                emit_system(&self.bus_tx, "[Spar]: Manually aborted by user.");
                break;
            }

            // Agent A
            let prompt_a = format!(
                "{} feedback:\n{last_output}\n\nPlease respond as {}.",
                config.agent_b_role, config.agent_a_role
            );
            inject_line(&agent_a, &prompt_a)?;
            let out_a =
                match wait_for_agent_message(&mut bus_rx, id_a, 0, AGENT_STAGE_TIMEOUT).await {
                    Ok(v) => v,
                    Err(AgentHubError::RateLimit(id)) => {
                        emit_rate_limit_abort(&self.bus_tx, &config.agent_a_tag, id);
                        return Ok(SparResult {
                            turns_completed: turn,
                            aborted: true,
                            stagnation: false,
                            last_output,
                        });
                    }
                    Err(e) => return Err(e),
                };
            emit_spar_turn(&self.bus_tx, turn, &config.agent_a_tag, &out_a);
            if check_stagnation(&mut history_a, &out_a) {
                stagnation = true;
                emit_system(
                    &self.bus_tx,
                    "[Spar Warning]: Stagnation detected. Agents are repeating themselves. \
                     Use /spar again with a more specific goal or fewer turns.",
                );
                break;
            }
            last_output = out_a;

            if SPAR_ABORT.load(Ordering::SeqCst) {
                aborted = true;
                emit_system(&self.bus_tx, "[Spar]: Manually aborted by user.");
                break;
            }

            // Agent B
            let prompt_b = format!(
                "{} produced:\n{last_output}\n\nPlease respond as {}.",
                config.agent_a_role, config.agent_b_role
            );
            inject_line(&agent_b, &prompt_b)?;
            let out_b =
                match wait_for_agent_message(&mut bus_rx, id_b, 0, AGENT_STAGE_TIMEOUT).await {
                    Ok(v) => v,
                    Err(AgentHubError::RateLimit(id)) => {
                        emit_rate_limit_abort(&self.bus_tx, &config.agent_b_tag, id);
                        return Ok(SparResult {
                            turns_completed: turn,
                            aborted: true,
                            stagnation: false,
                            last_output,
                        });
                    }
                    Err(e) => return Err(e),
                };
            emit_spar_turn(&self.bus_tx, turn, &config.agent_b_tag, &out_b);
            if check_stagnation(&mut history_b, &out_b) {
                stagnation = true;
                emit_system(
                    &self.bus_tx,
                    "[Spar Warning]: Stagnation detected. Agents are repeating themselves. \
                     Use /spar again with a more specific goal or fewer turns.",
                );
                break;
            }
            last_output = out_b;
            turn += 1;
        }

        if !aborted && !stagnation {
            emit_system(
                &self.bus_tx,
                &format!("[Spar]: Session complete after {turn} turns."),
            );
        }

        Ok(SparResult {
            turns_completed: turn,
            aborted,
            stagnation,
            last_output,
        })
    }
}

fn inject_role_prompt(
    agent: &AgentPty,
    role: &str,
    partner_role: &str,
    goal: &str,
    is_starter: bool,
) -> Result<()> {
    let text = if is_starter {
        format!(
            "For this sparring session, you are the {role}. Your goal: {goal}. \
             Start by addressing the goal directly."
        )
    } else {
        format!(
            "For this sparring session, you are the {role}. You will review and critique \
             what the {partner_role} produces. Goal context: {goal}."
        )
    };
    inject_line(agent, &text)
}

/// Yields briefly so Escape can abort before agent resolution (integration tests / TUI).
async fn poll_spar_abort() -> bool {
    for _ in 0..10 {
        if SPAR_ABORT.load(Ordering::SeqCst) {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    false
}

fn inject_line(agent: &AgentPty, text: &str) -> Result<()> {
    let payload = format!("{text}{}", line_ending());
    agent
        .write_stdin(payload.as_bytes())
        .map(|_| ())
        .map_err(|e| AgentHubError::Context(e.to_string()))
}

fn emit_spar_turn(bus_tx: &broadcast::Sender<BusEvent>, turn: u8, tag: &str, body: &str) {
    let content = format!("[Spar Turn {turn} — {tag}]\n{body}");
    let _ = bus_tx.send(BusEvent::SystemMessage {
        content,
        timestamp: Utc::now(),
    });
}

fn emit_rate_limit_abort(bus_tx: &broadcast::Sender<BusEvent>, tag: &str, id: Uuid) {
    let _ = bus_tx.send(BusEvent::RateLimitDetected {
        id,
        tag: tag.to_string(),
    });
    emit_system(
        bus_tx,
        &format!("[Spar]: Rate limit detected on @{tag}. Sparring aborted."),
    );
}

fn emit_system(bus_tx: &broadcast::Sender<BusEvent>, content: &str) {
    let _ = bus_tx.send(BusEvent::SystemMessage {
        content: content.to_string(),
        timestamp: Utc::now(),
    });
}

/// Returns true when the last two outputs from the same agent are near-identical.
pub fn check_stagnation(history: &mut Vec<String>, output: &str) -> bool {
    history.push(output.to_string());
    if history.len() < 2 {
        return false;
    }
    let prev = &history[history.len() - 2];
    let curr = &history[history.len() - 1];
    similarity_ratio(prev, curr) >= STAGNATION_RATIO
}

/// Normalized Levenshtein similarity in `[0.0, 1.0]` (`1.0` = identical).
#[must_use]
pub fn similarity_ratio(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    let max_len = a.chars().count().max(b.chars().count());
    if max_len == 0 {
        return 1.0;
    }
    let dist = levenshtein(a, b);
    1.0 - (dist as f64 / max_len as f64)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0; b.len() + 1];

    for (i, ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_spar_blueprint_example() {
        let cfg = parse_spar_command(
            "/spar @gemini as Coder vs @claude as Reviewer --turns 5 --goal \"Write a Rust TCP server\"",
        )
        .expect("parse");
        assert_eq!(cfg.agent_a_tag, "gemini");
        assert_eq!(cfg.agent_a_role, "Coder");
        assert_eq!(cfg.agent_b_tag, "claude");
        assert_eq!(cfg.agent_b_role, "Reviewer");
        assert_eq!(cfg.max_turns, 5);
        assert_eq!(cfg.goal, "Write a Rust TCP server");
    }

    #[test]
    fn stagnation_detects_near_identical_outputs() {
        let mut hist = vec!["hello world".to_string()];
        assert!(!check_stagnation(&mut hist, "hello world!"));
        assert!(check_stagnation(&mut vec!["alpha".to_string()], "alpha"));
    }

    #[test]
    fn similarity_ratio_identical_is_one() {
        assert!((similarity_ratio("same", "same") - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn spar_turns_clamped_by_parser() {
        let err = parse_spar_command("/spar @a as X vs @b as Y --turns 99 --goal test")
            .expect_err("too many turns");
        assert!(matches!(err, AgentHubError::Config(_)));
    }
}
