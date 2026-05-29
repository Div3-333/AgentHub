//! LLM Racing: multi-@ prompt fan-out (blueprint §11).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use dashmap::{DashMap, DashSet};
use uuid::Uuid;

use chrono::Utc;
use tokio::sync::broadcast;

use crate::config::AgentHubConfig;
use crate::db::{DbClient, NewMessage};
use crate::error::{AgentHubError, Result};
use crate::server::ServerState;
use crate::vfs::{create_snapshot, ensure_session, SnapshotTrigger};

use super::event::BusEvent;

/// Strip a leading `@` so bus tags match racing pane tags.
#[must_use]
pub fn normalize_racing_tag(tag: &str) -> String {
    tag.strip_prefix('@').unwrap_or(tag).to_string()
}

/// Maximum wall-clock spread between first and last PTY inject (Phase 8 DoD).
pub const INJECT_SPREAD_MS: u64 = 50;

/// Parsed racing activation: leading `@tag` tokens and trailing prompt body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRacingInput {
    pub tags: Vec<String>,
    pub prompt: String,
}

/// One agent in an active race.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RacingContestant {
    pub id: Uuid,
    pub tag: String,
}

/// In-flight racing session state.
#[derive(Debug)]
pub struct RacingSession {
    pub session_id: Uuid,
    pub contestants: Vec<RacingContestant>,
    pub prompt: String,
    pub outputs: Arc<DashMap<Uuid, String>>,
    pub completed: Arc<DashSet<Uuid>>,
    pub started_at: Instant,
}

impl RacingSession {
    #[must_use]
    pub fn new(contestants: Vec<RacingContestant>, prompt: String) -> Self {
        Self {
            session_id: Uuid::new_v4(),
            contestants,
            prompt,
            outputs: Arc::new(DashMap::new()),
            completed: Arc::new(DashSet::new()),
            started_at: Instant::now(),
        }
    }

    #[must_use]
    pub fn all_contestants_done(&self) -> bool {
        self.completed.len() >= self.contestants.len()
    }
}

/// Timestamp recorded when a contestant PTY received the prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InjectRecord {
    pub agent_id: Uuid,
    pub at: Instant,
}

/// Returns true when input should activate racing (≥2 leading `@tags`, no pipeline ` | `).
#[must_use]
pub fn is_racing_input(input: &str) -> bool {
    if input.contains(" | ") {
        return false;
    }
    count_leading_tags(input) >= 2
}

/// Parse `@a @b prompt` into tags and prompt. Errors if fewer than two tags or empty prompt.
pub fn parse_racing_input(input: &str) -> Result<ParsedRacingInput> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(AgentHubError::PipelineParse {
            pos: 0,
            msg: "empty racing input".into(),
        });
    }
    if trimmed.contains(" | ") {
        return Err(AgentHubError::PipelineParse {
            pos: trimmed.find(" | ").unwrap_or(0),
            msg: "pipeline syntax disables racing mode".into(),
        });
    }

    let tags = collect_leading_tags(trimmed);
    if tags.len() < 2 {
        return Err(AgentHubError::PipelineParse {
            pos: 0,
            msg: "racing requires at least two @tags".into(),
        });
    }

    let prompt = prompt_after_tags(trimmed, tags.len());
    if prompt.is_empty() {
        return Err(AgentHubError::PipelineParse {
            pos: trimmed.len(),
            msg: "racing prompt must not be empty".into(),
        });
    }

    Ok(ParsedRacingInput { tags, prompt })
}

/// Build a tag → agent id map from live [`ServerState`] agents.
#[must_use]
pub fn agent_tag_registry(state: &ServerState) -> HashMap<String, Uuid> {
    state
        .agents
        .iter()
        .map(|entry| {
            let agent = entry.value();
            let key = agent
                .tag
                .strip_prefix('@')
                .unwrap_or(&agent.tag)
                .to_string();
            (key, agent.id)
        })
        .collect()
}

/// Resolve tags; returns [`AgentHubError::Config`] when a tag is missing from `registry`.
pub fn resolve_contestants_by_tag(
    tags: &[String],
    registry: &HashMap<String, Uuid>,
) -> Result<Vec<RacingContestant>> {
    let mut out = Vec::with_capacity(tags.len());
    for tag in tags {
        let id = registry
            .get(tag)
            .copied()
            .ok_or_else(|| AgentHubError::Config(format!("unknown agent tag: @{tag}")))?;
        out.push(RacingContestant {
            id,
            tag: tag.clone(),
        });
    }
    Ok(out)
}

/// Inject `prompt` to every contestant concurrently; spread must be ≤ [`INJECT_SPREAD_MS`].
pub async fn inject_racing_prompts<F>(
    contestants: &[RacingContestant],
    prompt: &str,
    inject: F,
) -> Result<Vec<InjectRecord>>
where
    F: Fn(Uuid, &str) -> Result<()> + Send + Sync + Clone + 'static,
{
    use tokio::task::JoinSet;

    let prompt = prompt.to_string();
    let mut set: JoinSet<Result<InjectRecord>> = JoinSet::new();

    for c in contestants {
        let id = c.id;
        let prompt = prompt.clone();
        let inject = inject.clone();
        set.spawn(async move {
            let at = Instant::now();
            inject(id, &prompt)?;
            Ok(InjectRecord { agent_id: id, at })
        });
    }

    let mut records = Vec::with_capacity(contestants.len());
    while let Some(joined) = set.join_next().await {
        let record =
            joined.map_err(|e| AgentHubError::Pty(format!("racing inject task: {e}")))??;
        records.push(record);
    }

    if records.len() >= 2 {
        let Some(min) = records.iter().map(|r| r.at).min() else {
            return Ok(records);
        };
        let Some(max) = records.iter().map(|r| r.at).max() else {
            return Ok(records);
        };
        let spread_ms = max.duration_since(min).as_millis() as u64;
        if spread_ms > INJECT_SPREAD_MS {
            return Err(AgentHubError::Pty(format!(
                "racing inject spread {spread_ms}ms exceeds {INJECT_SPREAD_MS}ms limit"
            )));
        }
    }

    Ok(records)
}

/// Inject prompts through live agent PTYs on [`ServerState`].
pub async fn inject_racing_prompts_on_state(
    state: Arc<ServerState>,
    contestants: &[RacingContestant],
    prompt: &str,
) -> Result<Vec<InjectRecord>> {
    let state_c = Arc::clone(&state);
    inject_racing_prompts(contestants, prompt, move |id, p| {
        let agent = state_c
            .agents
            .get(&id)
            .ok_or_else(|| AgentHubError::AgentNotFound(id))?;
        agent.write_stdin(p.as_bytes())?;
        agent.write_stdin(b"\n")?;
        Ok(())
    })
    .await
}

/// Tracks active races and maps contestants → `session_id` for [`BusEvent::AgentMessage`] tagging.
#[derive(Debug, Default)]
pub struct RacingRegistry {
    sessions: DashMap<Uuid, Arc<RacingSession>>,
    agent_races: DashMap<Uuid, Uuid>,
}

impl RacingRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn race_id_for_agent(&self, agent_id: Uuid) -> Option<Uuid> {
        self.agent_races.get(&agent_id).map(|e| *e.value())
    }

    fn register(&self, session: Arc<RacingSession>) {
        let race_id = session.session_id;
        self.sessions.insert(race_id, Arc::clone(&session));
        for c in &session.contestants {
            self.agent_races.insert(c.id, race_id);
        }
    }

    pub fn record_output(&self, race_id: Uuid, agent_id: Uuid, content: String) {
        if let Some(session) = self.sessions.get(&race_id) {
            session.outputs.insert(agent_id, content);
        }
    }

    pub fn finish(&self, race_id: Uuid) {
        if let Some((_, session)) = self.sessions.remove(&race_id) {
            for c in &session.contestants {
                self.agent_races.remove(&c.id);
            }
        }
    }

    /// Tag agent messages, emit racing UI events, and detect race completion.
    pub fn process_bus_event(
        &self,
        event: BusEvent,
        bus_tx: &broadcast::Sender<BusEvent>,
    ) -> BusEvent {
        match event {
            BusEvent::AgentMessage {
                id,
                tag,
                content,
                timestamp,
                race_session_id: None,
            } => {
                let Some(race_id) = self.race_id_for_agent(id) else {
                    return BusEvent::AgentMessage {
                        id,
                        tag,
                        content,
                        timestamp,
                        race_session_id: None,
                    };
                };

                let norm_tag = normalize_racing_tag(&tag);
                let chunk = if let Some(session) = self.sessions.get(&race_id) {
                    let prev_len = session.outputs.get(&id).map(|e| e.len()).unwrap_or(0);
                    session.outputs.insert(id, content.clone());
                    if content.len() > prev_len {
                        content[prev_len..].to_string()
                    } else {
                        content.clone()
                    }
                } else {
                    content.clone()
                };

                if !chunk.is_empty() {
                    let _ = bus_tx.send(BusEvent::RacingOutput {
                        session_id: race_id,
                        tag: norm_tag.clone(),
                        chunk,
                        timestamp,
                    });
                }

                if let Some(session) = self.sessions.get(&race_id) {
                    if session.completed.insert(id) {
                        let elapsed_ms = session.started_at.elapsed().as_millis() as u64;
                        let _ = bus_tx.send(BusEvent::RacingAgentComplete {
                            session_id: race_id,
                            tag: norm_tag.clone(),
                            elapsed_ms,
                            timestamp,
                        });

                        if session.all_contestants_done() {
                            let _ = bus_tx.send(BusEvent::RacingComplete {
                                session_id: race_id,
                                timestamp,
                            });
                        }
                    }
                }

                BusEvent::AgentMessage {
                    id,
                    tag,
                    content,
                    timestamp,
                    race_session_id: Some(race_id),
                }
            }
            other => other,
        }
    }
}

/// Outcome of attempting to dispatch a user line as an LLM race.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RacingDispatch {
    /// Input was not racing syntax; normal bus routing applies.
    NotRacing,
    /// Race started (PTY inject + SQLite when `db` is present).
    Started,
    /// Racing syntax was recognized but setup failed.
    Failed,
}

/// Detect multi-@ input, start a [`RacingSession`], and inject the prompt to all contestants.
#[must_use]
pub async fn try_dispatch_racing_user_message(
    hub_session_id: Uuid,
    state: Arc<ServerState>,
    registry: &RacingRegistry,
    db: Option<&DbClient>,
    content: &str,
    snapshot_id: Option<Uuid>,
    bus_tx: &broadcast::Sender<BusEvent>,
) -> RacingDispatch {
    if !is_racing_input(content) {
        return RacingDispatch::NotRacing;
    }

    let parsed = match parse_racing_input(content) {
        Ok(parsed) => parsed,
        Err(e) => {
            let _ = bus_tx.send(BusEvent::SystemMessage {
                content: format!("Racing: {e}"),
                timestamp: Utc::now(),
            });
            return RacingDispatch::Failed;
        }
    };

    match start_racing_session(RacingSessionStart {
        hub_session_id,
        state,
        registry,
        db,
        raw_input: content,
        parsed: &parsed,
        snapshot_id,
        bus_tx,
    })
    .await
    {
        Ok(_) => RacingDispatch::Started,
        Err(e) => {
            let _ = bus_tx.send(BusEvent::SystemMessage {
                content: format!("Racing failed: {e}"),
                timestamp: Utc::now(),
            });
            RacingDispatch::Failed
        }
    }
}

/// Parameters for [`start_racing_session`].
pub struct RacingSessionStart<'a> {
    pub hub_session_id: Uuid,
    pub state: Arc<ServerState>,
    pub registry: &'a RacingRegistry,
    pub db: Option<&'a DbClient>,
    pub raw_input: &'a str,
    pub parsed: &'a ParsedRacingInput,
    pub snapshot_id: Option<Uuid>,
    pub bus_tx: &'a broadcast::Sender<BusEvent>,
}

/// Start a race: SQLite log, registry, parallel PTY inject, bus notification.
pub async fn start_racing_session(params: RacingSessionStart<'_>) -> Result<Arc<RacingSession>> {
    let RacingSessionStart {
        hub_session_id,
        state,
        registry,
        db,
        raw_input,
        parsed,
        snapshot_id,
        bus_tx,
    } = params;
    let tag_map = agent_tag_registry(&state);
    let contestants = resolve_contestants_by_tag(&parsed.tags, &tag_map)?;
    let session = Arc::new(RacingSession::new(
        contestants.clone(),
        parsed.prompt.clone(),
    ));
    let race_id = session.session_id;

    let effective_snapshot = if snapshot_id.is_some() {
        snapshot_id
    } else if let Some(db) = db {
        take_pre_race_snapshot(db, hub_session_id, bus_tx).await
    } else {
        None
    };

    if let Some(db) = db {
        db.log_race_start(
            hub_session_id,
            race_id,
            raw_input,
            &parsed.tags,
            effective_snapshot,
        )
        .await?;

        db.log_message(&NewMessage {
            id: Uuid::new_v4(),
            session_id: hub_session_id,
            sender_type: "user".into(),
            sender_id: None,
            sender_tag: "User".into(),
            content: raw_input.to_string(),
            timestamp_ms: Utc::now().timestamp_millis(),
            pipeline_id: None,
            race_id: Some(race_id),
        })
        .await?;
    }

    registry.register(Arc::clone(&session));

    let _ = bus_tx.send(BusEvent::RacingStarted {
        session_id: race_id,
        tags: parsed.tags.clone(),
        prompt: parsed.prompt.clone(),
        timestamp: Utc::now(),
    });

    inject_racing_prompts_on_state(state, &session.contestants, &parsed.prompt).await?;

    Ok(session)
}

/// VFS snapshot before a race (blueprint §11 step 3 / §12.1).
async fn take_pre_race_snapshot(
    db: &DbClient,
    hub_session_id: Uuid,
    bus_tx: &broadcast::Sender<BusEvent>,
) -> Option<Uuid> {
    let session = db.get_session(hub_session_id).await.ok()??;
    if session.cwd.is_empty() {
        return None;
    }
    let cwd = std::path::PathBuf::from(&session.cwd);
    let config = AgentHubConfig::load().unwrap_or_default();
    let shadow = if config.shadow_dir.is_absolute() {
        config.shadow_dir.clone()
    } else {
        cwd.join(&config.shadow_dir)
    };
    ensure_session(&db.pool, hub_session_id, &cwd).await.ok()?;
    create_snapshot(
        &db.pool,
        &cwd,
        &shadow,
        hub_session_id,
        SnapshotTrigger::Racing,
        Some(bus_tx),
    )
    .await
    .ok()
    .map(|info| info.id)
}

fn count_leading_tags(input: &str) -> usize {
    collect_leading_tags(input.trim_start()).len()
}

fn collect_leading_tags(mut rest: &str) -> Vec<String> {
    let mut tags = Vec::new();
    rest = rest.trim_start();
    while rest.starts_with('@') {
        let after = &rest[1..];
        let tag_len = tag_char_len(after);
        if tag_len == 0 {
            break;
        }
        tags.push(after[..tag_len].to_string());
        rest = after[tag_len..].trim_start();
    }
    tags
}

fn prompt_after_tags(input: &str, tag_count: usize) -> String {
    let mut rest = input.trim_start();
    for _ in 0..tag_count {
        if !rest.starts_with('@') {
            break;
        }
        let after = &rest[1..];
        let tag_len = tag_char_len(after);
        if tag_len == 0 {
            break;
        }
        rest = after[tag_len..].trim_start();
    }
    rest.trim().to_string()
}

fn tag_char_len(s: &str) -> usize {
    s.chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .map(char::len_utf8)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[test]
    fn is_racing_input_requires_two_tags_no_pipe() {
        assert!(is_racing_input("@mock-1 @mock-2 hello"));
        assert!(!is_racing_input("@mock-1 hello"));
        assert!(!is_racing_input("@a @b | > echo"));
        assert!(!is_racing_input("@gemini go | @claude review"));
    }

    #[test]
    fn parse_racing_input_blueprint_example() {
        let parsed = parse_racing_input("@gemini @claude @codex write a binary search").unwrap();
        assert_eq!(
            parsed.tags,
            ["gemini", "claude", "codex"].map(String::from).to_vec()
        );
        assert_eq!(parsed.prompt, "write a binary search");
    }

    #[test]
    fn parse_rejects_single_tag() {
        let err = parse_racing_input("@only one").unwrap_err();
        assert!(matches!(err, AgentHubError::PipelineParse { .. }));
    }

    #[tokio::test]
    async fn inject_spread_within_50ms_with_mocks() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let contestants = vec![
            RacingContestant {
                id: a,
                tag: "mock-1".into(),
            },
            RacingContestant {
                id: b,
                tag: "mock-2".into(),
            },
        ];

        let times: Arc<Mutex<Vec<(Uuid, Instant)>>> = Arc::new(Mutex::new(Vec::new()));
        let times_c = Arc::clone(&times);

        let records = inject_racing_prompts(&contestants, "hello", move |id, _| {
            times_c
                .lock()
                .map_err(|_| AgentHubError::Pty("lock poisoned".into()))?
                .push((id, Instant::now()));
            Ok(())
        })
        .await
        .expect("inject");

        assert_eq!(records.len(), 2);
        let locked = times.lock().expect("lock");
        let spread = locked[1].1.duration_since(locked[0].1).as_millis();
        assert!(
            spread <= INJECT_SPREAD_MS as u128,
            "mock inject spread {spread}ms"
        );
    }

    #[test]
    fn normalize_racing_tag_strips_at() {
        assert_eq!(normalize_racing_tag("@gemini-1"), "gemini-1");
        assert_eq!(normalize_racing_tag("claude-1"), "claude-1");
    }

    #[test]
    fn registry_emits_racing_complete_when_all_contestants_finish() {
        let registry = RacingRegistry::new();
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let session = Arc::new(RacingSession::new(
            vec![
                RacingContestant {
                    id: id_a,
                    tag: "mock-1".into(),
                },
                RacingContestant {
                    id: id_b,
                    tag: "mock-2".into(),
                },
            ],
            "race".into(),
        ));
        let race_id = session.session_id;
        registry.register(session);

        let (bus_tx, mut bus_rx) = broadcast::channel(16);
        for (id, tag) in [(id_a, "mock-1"), (id_b, "mock-2")] {
            registry.process_bus_event(
                BusEvent::AgentMessage {
                    id,
                    tag: tag.into(),
                    content: format!("answer from {tag}"),
                    timestamp: Utc::now(),
                    race_session_id: None,
                },
                &bus_tx,
            );
        }

        let mut saw_complete = false;
        while let Ok(ev) = bus_rx.try_recv() {
            if matches!(ev, BusEvent::RacingComplete { session_id, .. } if session_id == race_id) {
                saw_complete = true;
            }
        }
        assert!(saw_complete, "expected RacingComplete for session");
        assert!(registry.race_id_for_agent(id_a).is_some());
        registry.finish(race_id);
        assert!(registry.race_id_for_agent(id_a).is_none());
    }

    #[test]
    fn registry_tags_agent_messages_with_race_id() {
        let registry = RacingRegistry::new();
        let agent_id = Uuid::new_v4();
        let session = Arc::new(RacingSession::new(
            vec![RacingContestant {
                id: agent_id,
                tag: "gemini-1".into(),
            }],
            "hi".into(),
        ));
        let race_id = session.session_id;
        registry.register(session);

        let (bus_tx, _) = broadcast::channel(8);
        let tagged = registry.process_bus_event(
            BusEvent::AgentMessage {
                id: agent_id,
                tag: "@gemini-1".into(),
                content: "answer".into(),
                timestamp: Utc::now(),
                race_session_id: None,
            },
            &bus_tx,
        );
        match tagged {
            BusEvent::AgentMessage {
                race_session_id: Some(rid),
                content,
                ..
            } => {
                assert_eq!(rid, race_id);
                assert_eq!(content, "answer");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn inject_rejects_slow_mock_when_forced_delay() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let contestants = vec![
            RacingContestant {
                id: a,
                tag: "slow-a".into(),
            },
            RacingContestant {
                id: b,
                tag: "slow-b".into(),
            },
        ];

        let err = inject_racing_prompts(&contestants, "hi", move |id, _| {
            if id == a {
                std::thread::sleep(Duration::from_millis(60));
            }
            Ok(())
        })
        .await
        .unwrap_err();

        assert!(matches!(err, AgentHubError::Pty(_)));
        let msg = err.to_string();
        assert!(msg.contains("spread"));
    }
}
