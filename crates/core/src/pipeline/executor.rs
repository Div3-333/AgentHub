//! Pipeline execution engine (blueprint §10.2).

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use sqlx::SqlitePool;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::broadcast;
use tokio::time::timeout;
use uuid::Uuid;

use crate::bus::BusEvent;
use crate::config::AgentHubConfig;
use crate::db::DbClient;
use crate::error::{AgentHubError, Result};
use crate::pty::AgentPty;
use crate::server::modes;
use crate::server::rbac::{default_roles, AgentState, Permissions};
use crate::server::ServerState;
use crate::vfs::{create_snapshot, ensure_session, SnapshotTrigger};

use super::parser::{parse, AgentStage, PipelineStage, UnixStage};

/// Agent response wait (blueprint §10.2).
pub const AGENT_STAGE_TIMEOUT: Duration = Duration::from_secs(300);
/// Unix stage wait (blueprint §10.2).
pub const UNIX_STAGE_TIMEOUT: Duration = Duration::from_secs(60);

/// Outcome of a completed pipeline run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineResult {
    pub pipeline_id: Uuid,
    pub final_output: String,
    pub snapshot_id: Option<Uuid>,
}

/// Runs Frankenstein pipelines against live agents and optional shell stages.
pub struct PipelineExecutor {
    state: Arc<ServerState>,
    bus_tx: broadcast::Sender<BusEvent>,
    cwd: PathBuf,
    session_id: Uuid,
    config: AgentHubConfig,
    db: Option<Arc<DbClient>>,
}

impl PipelineExecutor {
    #[must_use]
    pub fn new(
        state: Arc<ServerState>,
        bus_tx: broadcast::Sender<BusEvent>,
        cwd: impl Into<PathBuf>,
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

    /// Parse and execute a pipeline definition string.
    pub async fn execute(&self, definition: &str) -> Result<PipelineResult> {
        let stages = parse(definition)?;
        let pipeline_id = Uuid::new_v4();

        let _ = self.bus_tx.send(BusEvent::PipelineStarted {
            pipeline_id,
            definition: definition.to_string(),
        });

        let snapshot_id = self.maybe_snapshot().await?;

        if let Some(db) = &self.db {
            insert_pipeline_row(
                &db.pool,
                pipeline_id,
                self.session_id,
                definition,
                snapshot_id,
            )
            .await?;
        }

        let mut bus_rx = self.bus_tx.subscribe();
        let mut current_output = String::new();
        let mut last_agent: Option<(Uuid, String)> = None;

        for (index, stage) in stages.iter().enumerate() {
            let stage_result = match stage {
                PipelineStage::Agent(agent) => {
                    let result = self
                        .run_agent_stage(index, agent, &mut current_output, &mut bus_rx)
                        .await;
                    if let Ok(Some(responder_id)) = &result {
                        if let Some(pty) = self.state.agents.get(responder_id) {
                            last_agent = Some((*responder_id, pty.tag.clone()));
                        }
                    }
                    result.map(|_| ())
                }
                PipelineStage::Unix(unix) => {
                    self.run_unix_stage(
                        index,
                        unix,
                        &mut current_output,
                        pipeline_id,
                        last_agent.as_ref().map(|(id, _)| *id),
                    )
                    .await
                }
            };

            if let Err(err) = stage_result {
                let msg = err.to_string();
                let _ = self.bus_tx.send(BusEvent::PipelineFailed {
                    pipeline_id,
                    stage: index,
                    error: msg.clone(),
                });
                if let Some(db) = &self.db {
                    mark_pipeline_failed(&db.pool, pipeline_id, &msg).await;
                }
                return Err(err);
            }

            let preview = preview_output(&current_output);
            let _ = self.bus_tx.send(BusEvent::PipelineStageComplete {
                pipeline_id,
                stage: index,
                output_preview: preview,
            });

            if let Some(db) = &self.db {
                log_stage_complete(&db.pool, pipeline_id, index, stage, &current_output).await;
            }
        }

        let _ = self.bus_tx.send(BusEvent::PipelineComplete { pipeline_id });

        if let Some((id, tag)) = last_agent {
            if modes::gate_agent_bus_output(&self.state, id, &current_output).is_ok() {
                let _ = self.bus_tx.send(BusEvent::AgentMessage {
                    id,
                    tag,
                    content: current_output.clone(),
                    timestamp: Utc::now(),
                    race_session_id: None,
                });
            }
        }

        if let Some(db) = &self.db {
            mark_pipeline_complete(&db.pool, pipeline_id).await;
        }

        Ok(PipelineResult {
            pipeline_id,
            final_output: current_output,
            snapshot_id,
        })
    }

    async fn maybe_snapshot(&self) -> Result<Option<Uuid>> {
        let Some(db) = &self.db else {
            return Ok(None);
        };
        ensure_session(&db.pool, self.session_id, &self.cwd).await?;
        let shadow = if self.config.shadow_dir.is_absolute() {
            self.config.shadow_dir.clone()
        } else {
            self.cwd.join(&self.config.shadow_dir)
        };
        let info = create_snapshot(
            &db.pool,
            &self.cwd,
            &shadow,
            self.session_id,
            SnapshotTrigger::Pipeline,
            Some(&self.bus_tx),
        )
        .await?;
        Ok(Some(info.id))
    }

    async fn run_agent_stage(
        &self,
        index: usize,
        stage: &AgentStage,
        current_output: &mut String,
        bus_rx: &mut broadcast::Receiver<BusEvent>,
    ) -> Result<Option<Uuid>> {
        let inject = if index == 0 {
            stage.prompt.clone()
        } else {
            format!(
                "[Pipeline context from previous stage]:\n{current_output}\n\n{}",
                stage.prompt
            )
        };

        let line_end = line_ending();
        let payload = format!("{inject}{line_end}");

        let (agent_id, content) = match &stage.tag {
            Some(tag) => {
                let (agent_id, agent) = find_agent_by_tag(&self.state, tag, index)?;
                ensure_agent_rbac(&self.state, &agent);
                modes::enforce_send_messages(&self.state, agent_id)?;
                agent
                    .write_stdin(payload.as_bytes())
                    .map_err(|e| stage_error(index, e.to_string()))?;
                let content = wait_for_agent_message(bus_rx, agent_id, index, AGENT_STAGE_TIMEOUT)
                    .await
                    .map_err(|e| stage_error(index, e.to_string()))?;
                (agent_id, content)
            }
            None => {
                let targets = all_active_agents(&self.state, index)?;
                for (id, agent) in &targets {
                    ensure_agent_rbac(&self.state, agent);
                    modes::enforce_send_messages(&self.state, *id)?;
                    agent
                        .write_stdin(payload.as_bytes())
                        .map_err(|e| stage_error(index, e.to_string()))?;
                }
                let ids: Vec<Uuid> = targets.iter().map(|(id, _)| *id).collect();
                wait_for_any_agent_message(bus_rx, &ids, index, AGENT_STAGE_TIMEOUT)
                    .await
                    .map_err(|e| stage_error(index, e.to_string()))?
            }
        };

        *current_output = content;
        Ok(Some(agent_id))
    }

    async fn run_unix_stage(
        &self,
        index: usize,
        stage: &UnixStage,
        current_output: &mut String,
        pipeline_id: Uuid,
        prior_agent: Option<Uuid>,
    ) -> Result<()> {
        if let Some(agent_id) = prior_agent {
            modes::enforce_execute_unix(&self.state, agent_id)?;
        }
        let mut cmd = shell_command(&stage.command);
        cmd.current_dir(&self.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| stage_error(index, format!("failed to spawn shell: {e}")))?;

        if let Some(mut stdin) = child.stdin.take() {
            if !current_output.is_empty() {
                stdin
                    .write_all(current_output.as_bytes())
                    .await
                    .map_err(|e| stage_error(index, format!("stdin write failed: {e}")))?;
            }
            drop(stdin);
        }

        let output = timeout(UNIX_STAGE_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| {
                stage_error(
                    index,
                    format!(
                        "unix stage timed out after {}s",
                        UNIX_STAGE_TIMEOUT.as_secs()
                    ),
                )
            })?
            .map_err(|e| stage_error(index, format!("unix stage failed: {e}")))?;

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let code = output.status.code().unwrap_or(-1);

        if code != 0 {
            let msg = format!("[Pipeline]: Unix command failed (exit {code}):\n{stderr}");
            let _ = self.bus_tx.send(BusEvent::SystemMessage {
                content: msg.clone(),
                timestamp: Utc::now(),
            });
            let _ = self.bus_tx.send(BusEvent::PipelineFailed {
                pipeline_id,
                stage: index,
                error: msg.clone(),
            });
            *current_output = stderr;
            return Err(stage_error(index, msg));
        }

        *current_output = stdout;
        Ok(())
    }
}

pub(crate) fn find_agent_by_tag(
    state: &ServerState,
    tag: &str,
    stage: usize,
) -> Result<(Uuid, Arc<AgentPty>)> {
    state
        .find_agent_by_tag(tag)
        .ok_or_else(|| AgentHubError::PipelineExecution {
            stage,
            msg: format!("agent not found: @{tag}"),
        })
}

fn all_active_agents(state: &ServerState, stage: usize) -> Result<Vec<(Uuid, Arc<AgentPty>)>> {
    let agents: Vec<_> = state
        .agents
        .iter()
        .map(|entry| (*entry.key(), Arc::clone(entry.value())))
        .collect();
    if agents.is_empty() {
        return Err(AgentHubError::PipelineExecution {
            stage,
            msg: "no active agents for broadcast pipeline stage".into(),
        });
    }
    Ok(agents)
}

pub(crate) async fn wait_for_agent_message(
    bus_rx: &mut broadcast::Receiver<BusEvent>,
    agent_id: Uuid,
    stage: usize,
    wait: Duration,
) -> Result<String> {
    wait_for_any_agent_message(bus_rx, &[agent_id], stage, wait)
        .await
        .map(|(_, content)| content)
}

pub(crate) async fn wait_for_any_agent_message(
    bus_rx: &mut broadcast::Receiver<BusEvent>,
    agent_ids: &[Uuid],
    stage: usize,
    wait: Duration,
) -> Result<(Uuid, String)> {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(AgentHubError::PipelineExecution {
                stage,
                msg: format!(
                    "timed out waiting for agent message after {}s",
                    wait.as_secs()
                ),
            });
        }

        match timeout(remaining, bus_rx.recv()).await {
            Ok(Ok(BusEvent::AgentMessage { id, content, .. })) if agent_ids.contains(&id) => {
                return Ok((id, content));
            }
            Ok(Ok(BusEvent::RateLimitDetected { id, tag: _ })) if agent_ids.contains(&id) => {
                return Err(AgentHubError::RateLimit(id));
            }
            Ok(Ok(BusEvent::AgentOffline { id, .. })) if agent_ids.contains(&id) => {
                return Err(AgentHubError::PipelineExecution {
                    stage,
                    msg: "agent went offline during pipeline stage".into(),
                });
            }
            Ok(Ok(_)) => continue,
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) => {
                return Err(AgentHubError::PipelineExecution {
                    stage,
                    msg: "bus closed while waiting for agent message".into(),
                });
            }
            Err(_) => {
                return Err(AgentHubError::PipelineExecution {
                    stage,
                    msg: format!(
                        "timed out waiting for agent message after {}s",
                        wait.as_secs()
                    ),
                });
            }
        }
    }
}

fn shell_command(command: &str) -> Command {
    if cfg!(windows) {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", command]);
        cmd
    } else {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", command]);
        cmd
    }
}

pub(crate) fn line_ending() -> &'static str {
    if cfg!(windows) {
        "\r\n"
    } else {
        "\n"
    }
}

fn stage_error(stage: usize, msg: String) -> AgentHubError {
    AgentHubError::PipelineExecution { stage, msg }
}

fn preview_output(output: &str) -> String {
    const LIMIT: usize = 200;
    if output.chars().count() <= LIMIT {
        output.to_string()
    } else {
        output.chars().take(LIMIT).collect()
    }
}

async fn insert_pipeline_row(
    pool: &SqlitePool,
    pipeline_id: Uuid,
    session_id: Uuid,
    definition: &str,
    snapshot_id: Option<Uuid>,
) -> Result<()> {
    let now = Utc::now().timestamp();
    sqlx::query(
        r"
        INSERT INTO pipelines (id, session_id, definition, status, started_at, snapshot_id)
        VALUES (?, ?, ?, 'running', ?, ?)
        ",
    )
    .bind(pipeline_id.to_string())
    .bind(session_id.to_string())
    .bind(definition)
    .bind(now)
    .bind(snapshot_id.map(|id| id.to_string()))
    .execute(pool)
    .await?;
    Ok(())
}

async fn mark_pipeline_complete(pool: &SqlitePool, pipeline_id: Uuid) {
    let now = Utc::now().timestamp();
    let _ = sqlx::query("UPDATE pipelines SET status = 'complete', completed_at = ? WHERE id = ?")
        .bind(now)
        .bind(pipeline_id.to_string())
        .execute(pool)
        .await;
}

async fn mark_pipeline_failed(pool: &SqlitePool, pipeline_id: Uuid, _error: &str) {
    let now = Utc::now().timestamp();
    let _ = sqlx::query("UPDATE pipelines SET status = 'failed', completed_at = ? WHERE id = ?")
        .bind(now)
        .bind(pipeline_id.to_string())
        .execute(pool)
        .await;
}

async fn log_stage_complete(
    pool: &SqlitePool,
    pipeline_id: Uuid,
    stage_index: usize,
    stage: &PipelineStage,
    output: &str,
) {
    let (stage_type, target) = match stage {
        PipelineStage::Agent(a) => ("agent", a.tag.clone().unwrap_or_else(|| "broadcast".into())),
        PipelineStage::Unix(u) => ("unix", u.command.clone()),
    };
    let now = Utc::now().timestamp();
    let _ = sqlx::query(
        r"
        INSERT INTO pipeline_stages
            (id, pipeline_id, stage_index, stage_type, target, output_text, completed_at)
        VALUES (?, ?, ?, ?, ?, ?, ?)
        ",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(pipeline_id.to_string())
    .bind(i64::try_from(stage_index).unwrap_or(i64::MAX))
    .bind(stage_type)
    .bind(target)
    .bind(output)
    .bind(now)
    .execute(pool)
    .await;
}

/// Registers a Builder-role [`AgentState`] for integration tests when missing.
pub fn ensure_agent_rbac(state: &ServerState, agent: &AgentPty) {
    if state.agent_states.contains_key(&agent.id) {
        return;
    }
    let perms = default_roles()
        .get("Builder")
        .copied()
        .unwrap_or(Permissions::SEND_MESSAGES | Permissions::EXECUTE_UNIX);
    state.agent_states.insert(
        agent.id,
        Arc::new(AgentState::new(
            agent.id,
            agent.tag.clone(),
            agent.driver_name.clone(),
            "Builder".into(),
            perms,
            1,
        )),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::parser::parse;

    #[test]
    fn preview_truncates_to_200_chars() {
        let long = "x".repeat(250);
        let preview = preview_output(&long);
        assert_eq!(preview.chars().count(), 200);
    }

    #[tokio::test]
    async fn unix_stage_echo_pipes_stdin() {
        let mut output = String::from("hello world");
        let stage = UnixStage {
            command: if cfg!(windows) {
                "more".to_string()
            } else {
                "cat".to_string()
            },
        };
        let state = Arc::new(ServerState::new());
        let (bus_tx, _) = broadcast::channel(8);
        let exec = PipelineExecutor::new(
            state,
            bus_tx,
            std::env::temp_dir(),
            Uuid::new_v4(),
            AgentHubConfig::default(),
            None,
        );
        exec.run_unix_stage(0, &stage, &mut output, Uuid::new_v4(), None)
            .await
            .expect("unix stage");
        assert!(output.contains("hello"));
    }

    #[tokio::test]
    async fn unix_stage_nonzero_exit_fails() {
        let mut output = String::new();
        let stage = UnixStage {
            command: if cfg!(windows) {
                "cmd /C exit 1".to_string()
            } else {
                "false".to_string()
            },
        };
        let state = Arc::new(ServerState::new());
        let (bus_tx, _) = broadcast::channel(8);
        let exec = PipelineExecutor::new(
            state,
            bus_tx,
            std::env::temp_dir(),
            Uuid::new_v4(),
            AgentHubConfig::default(),
            None,
        );
        let err = exec
            .run_unix_stage(0, &stage, &mut output, Uuid::new_v4(), None)
            .await
            .expect_err("should fail");
        assert!(matches!(err, AgentHubError::PipelineExecution { .. }));
    }

    #[test]
    fn parse_blueprint_pipeline_three_stages() {
        let input = "@mock-1 hello | > echo world | @mock-2 repeat";
        let stages = parse(input).expect("parse");
        assert_eq!(stages.len(), 3);
    }
}
