//! SQLite client and bus-event persistence (blueprint Part 14 + §7.2).

use std::str::FromStr;

use chrono::{DateTime, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

use crate::bus::{BusEvent, OfflineReason};
use crate::error::{AgentHubError, Result};

/// Session row for insert.
#[derive(Debug, Clone)]
pub struct NewSession {
    pub id: Uuid,
    pub mode: String,
    pub cwd: String,
}

/// Agent row for insert.
#[derive(Debug, Clone)]
pub struct NewAgent {
    pub id: Uuid,
    pub session_id: Uuid,
    pub tag: String,
    pub driver_name: String,
    pub role: String,
}

/// Chat message row for insert.
#[derive(Debug, Clone)]
pub struct NewMessage {
    pub id: Uuid,
    pub session_id: Uuid,
    pub sender_type: String,
    pub sender_id: Option<Uuid>,
    pub sender_tag: String,
    pub content: String,
    pub timestamp_ms: i64,
    pub pipeline_id: Option<Uuid>,
    pub race_id: Option<Uuid>,
}

/// Pipeline row for insert.
#[derive(Debug, Clone)]
pub struct NewPipeline {
    pub id: Uuid,
    pub session_id: Uuid,
    pub definition: String,
    pub snapshot_id: Option<Uuid>,
}

/// Pipeline stage row for insert.
#[derive(Debug, Clone)]
pub struct NewPipelineStage {
    pub id: Uuid,
    pub pipeline_id: Uuid,
    pub stage_index: i64,
    pub stage_type: String,
    pub target: String,
}

/// Snapshot row for insert.
#[derive(Debug, Clone)]
pub struct NewSnapshot {
    pub id: Uuid,
    pub session_id: Uuid,
    pub file_count: i64,
    pub size_bytes: i64,
    pub cwd: String,
    pub trigger: String,
}

/// Snapshot file row for insert.
#[derive(Debug, Clone)]
pub struct NewSnapshotFile {
    pub id: Uuid,
    pub snapshot_id: Uuid,
    pub rel_path: String,
    pub blake3_hash: String,
    pub status: String,
}

/// Custom role row for upsert (`custom_roles` table).
#[derive(Debug, Clone)]
pub struct NewCustomRole {
    pub name: String,
    pub permissions_mask: i64,
    pub induction_override: Option<String>,
}

/// PTY debug log row for insert (`pty_debug_log` table).
#[derive(Debug, Clone)]
pub struct NewPtyDebugEntry {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub timestamp: i64,
    pub raw_bytes: Vec<u8>,
}

/// Persisted session (blueprint `sessions` table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    pub id: Uuid,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub mode: String,
    pub cwd: String,
}

/// Persisted agent (`agents` table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRow {
    pub id: Uuid,
    pub session_id: Uuid,
    pub tag: String,
    pub driver_name: String,
    pub role: String,
    pub spawned_at: i64,
    pub killed_at: Option<i64>,
    pub kill_reason: Option<String>,
}

/// Persisted message (`messages` table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRow {
    pub id: Uuid,
    pub session_id: Uuid,
    pub sender_type: String,
    pub sender_id: Option<Uuid>,
    pub sender_tag: String,
    pub content: String,
    pub timestamp_ms: i64,
    pub pipeline_id: Option<Uuid>,
    pub race_id: Option<Uuid>,
}

/// Persisted pipeline (`pipelines` table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineRow {
    pub id: Uuid,
    pub session_id: Uuid,
    pub definition: String,
    pub status: String,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub snapshot_id: Option<Uuid>,
}

/// Persisted pipeline stage (`pipeline_stages` table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineStageRow {
    pub id: Uuid,
    pub pipeline_id: Uuid,
    pub stage_index: i64,
    pub stage_type: String,
    pub target: String,
    pub input_text: Option<String>,
    pub output_text: Option<String>,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub exit_code: Option<i64>,
}

/// Persisted snapshot (`snapshots` table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotRow {
    pub id: Uuid,
    pub session_id: Uuid,
    pub timestamp: i64,
    pub file_count: i64,
    pub size_bytes: i64,
    pub cwd: String,
    pub trigger: String,
}

/// Persisted snapshot file (`snapshot_files` table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotFileRow {
    pub id: Uuid,
    pub snapshot_id: Uuid,
    pub rel_path: String,
    pub blake3_hash: String,
    pub status: String,
}

/// Persisted custom role (`custom_roles` table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomRoleRow {
    pub name: String,
    pub permissions_mask: i64,
    pub induction_override: Option<String>,
}

/// Persisted PTY debug entry (`pty_debug_log` table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtyDebugRow {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub timestamp: i64,
    pub raw_bytes: Vec<u8>,
}

#[derive(Debug, FromRow)]
struct SessionRowRaw {
    id: String,
    started_at: i64,
    ended_at: Option<i64>,
    mode: String,
    cwd: String,
}

#[derive(Debug, FromRow)]
struct AgentRowRaw {
    id: String,
    session_id: String,
    tag: String,
    driver_name: String,
    role: String,
    spawned_at: i64,
    killed_at: Option<i64>,
    kill_reason: Option<String>,
}

#[derive(Debug, FromRow)]
struct MessageRowRaw {
    id: String,
    session_id: String,
    sender_type: String,
    sender_id: Option<String>,
    sender_tag: String,
    content: String,
    timestamp: i64,
    pipeline_id: Option<String>,
    race_id: Option<String>,
}

#[derive(Debug, FromRow)]
struct PipelineRowRaw {
    id: String,
    session_id: String,
    definition: String,
    status: String,
    started_at: i64,
    completed_at: Option<i64>,
    snapshot_id: Option<String>,
}

#[derive(Debug, FromRow)]
struct PipelineStageRowRaw {
    id: String,
    pipeline_id: String,
    stage_index: i64,
    stage_type: String,
    target: String,
    input_text: Option<String>,
    output_text: Option<String>,
    started_at: Option<i64>,
    completed_at: Option<i64>,
    exit_code: Option<i64>,
}

#[derive(Debug, FromRow)]
struct SnapshotRowRaw {
    id: String,
    session_id: String,
    timestamp: i64,
    file_count: i64,
    size_bytes: i64,
    cwd: String,
    trigger: String,
}

#[derive(Debug, FromRow)]
struct SnapshotFileRowRaw {
    id: String,
    snapshot_id: String,
    rel_path: String,
    blake3_hash: String,
    status: String,
}

#[derive(Debug, FromRow)]
struct CustomRoleRowRaw {
    name: String,
    permissions_mask: i64,
    induction_override: Option<String>,
}

#[derive(Debug, FromRow)]
struct PtyDebugRowRaw {
    id: String,
    agent_id: String,
    timestamp: i64,
    raw_bytes: Vec<u8>,
}

fn row_uuid(value: &str, field: &str) -> Result<Uuid> {
    Uuid::parse_str(value)
        .map_err(|e| AgentHubError::Database(sqlx::Error::Decode(format!("{field}: {e}").into())))
}

fn opt_row_uuid(value: Option<String>, field: &str) -> Result<Option<Uuid>> {
    value.map(|s| row_uuid(&s, field)).transpose()
}

fn session_from_raw(raw: SessionRowRaw) -> Result<SessionRow> {
    Ok(SessionRow {
        id: row_uuid(&raw.id, "sessions.id")?,
        started_at: raw.started_at,
        ended_at: raw.ended_at,
        mode: raw.mode,
        cwd: raw.cwd,
    })
}

fn agent_from_raw(raw: AgentRowRaw) -> Result<AgentRow> {
    Ok(AgentRow {
        id: row_uuid(&raw.id, "agents.id")?,
        session_id: row_uuid(&raw.session_id, "agents.session_id")?,
        tag: raw.tag,
        driver_name: raw.driver_name,
        role: raw.role,
        spawned_at: raw.spawned_at,
        killed_at: raw.killed_at,
        kill_reason: raw.kill_reason,
    })
}

fn message_from_raw(raw: MessageRowRaw) -> Result<MessageRow> {
    Ok(MessageRow {
        id: row_uuid(&raw.id, "messages.id")?,
        session_id: row_uuid(&raw.session_id, "messages.session_id")?,
        sender_type: raw.sender_type,
        sender_id: opt_row_uuid(raw.sender_id, "messages.sender_id")?,
        sender_tag: raw.sender_tag,
        content: raw.content,
        timestamp_ms: raw.timestamp,
        pipeline_id: opt_row_uuid(raw.pipeline_id, "messages.pipeline_id")?,
        race_id: opt_row_uuid(raw.race_id, "messages.race_id")?,
    })
}

fn pipeline_from_raw(raw: PipelineRowRaw) -> Result<PipelineRow> {
    Ok(PipelineRow {
        id: row_uuid(&raw.id, "pipelines.id")?,
        session_id: row_uuid(&raw.session_id, "pipelines.session_id")?,
        definition: raw.definition,
        status: raw.status,
        started_at: raw.started_at,
        completed_at: raw.completed_at,
        snapshot_id: opt_row_uuid(raw.snapshot_id, "pipelines.snapshot_id")?,
    })
}

fn pipeline_stage_from_raw(raw: PipelineStageRowRaw) -> Result<PipelineStageRow> {
    Ok(PipelineStageRow {
        id: row_uuid(&raw.id, "pipeline_stages.id")?,
        pipeline_id: row_uuid(&raw.pipeline_id, "pipeline_stages.pipeline_id")?,
        stage_index: raw.stage_index,
        stage_type: raw.stage_type,
        target: raw.target,
        input_text: raw.input_text,
        output_text: raw.output_text,
        started_at: raw.started_at,
        completed_at: raw.completed_at,
        exit_code: raw.exit_code,
    })
}

fn snapshot_from_raw(raw: SnapshotRowRaw) -> Result<SnapshotRow> {
    Ok(SnapshotRow {
        id: row_uuid(&raw.id, "snapshots.id")?,
        session_id: row_uuid(&raw.session_id, "snapshots.session_id")?,
        timestamp: raw.timestamp,
        file_count: raw.file_count,
        size_bytes: raw.size_bytes,
        cwd: raw.cwd,
        trigger: raw.trigger,
    })
}

fn snapshot_file_from_raw(raw: SnapshotFileRowRaw) -> Result<SnapshotFileRow> {
    Ok(SnapshotFileRow {
        id: row_uuid(&raw.id, "snapshot_files.id")?,
        snapshot_id: row_uuid(&raw.snapshot_id, "snapshot_files.snapshot_id")?,
        rel_path: raw.rel_path,
        blake3_hash: raw.blake3_hash,
        status: raw.status,
    })
}

fn pty_debug_from_raw(raw: PtyDebugRowRaw) -> Result<PtyDebugRow> {
    Ok(PtyDebugRow {
        id: row_uuid(&raw.id, "pty_debug_log.id")?,
        agent_id: row_uuid(&raw.agent_id, "pty_debug_log.agent_id")?,
        timestamp: raw.timestamp,
        raw_bytes: super::decompress_pty_bytes(&raw.raw_bytes)?,
    })
}

#[derive(Clone)]
pub struct DbClient {
    pub pool: SqlitePool,
}

impl DbClient {
    pub async fn init_pool(url: &str) -> Result<Self> {
        let options = SqliteConnectOptions::from_str(url)?
            .create_if_missing(true)
            .pragma("foreign_keys", "on")
            .pragma("journal_mode", "wal")
            .pragma("synchronous", "normal");
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;
        Ok(Self { pool })
    }

    pub async fn run_migrations(&self) -> Result<()> {
        sqlx::migrate!("./src/db/migrations")
            .run(&self.pool)
            .await
            .map_err(|e| AgentHubError::Database(e.into()))?;
        self.apply_pragmas().await
    }

    /// Blueprint Part 14 pragmas (must run outside sqlx migration transactions).
    async fn apply_pragmas(&self) -> Result<()> {
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&self.pool)
            .await?;
        sqlx::query("PRAGMA journal_mode = WAL")
            .execute(&self.pool)
            .await?;
        sqlx::query("PRAGMA synchronous = 1")
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ── Sessions ─────────────────────────────────────────────────────────────

    /// Insert a session row (`ON CONFLICT DO NOTHING`).
    pub async fn insert_session(&self, session: &NewSession) -> Result<()> {
        let started_at = Utc::now().timestamp();
        sqlx::query(
            r"
            INSERT INTO sessions (id, started_at, mode, cwd)
            VALUES (?, ?, ?, ?)
            ON CONFLICT(id) DO NOTHING
            ",
        )
        .bind(session.id.to_string())
        .bind(started_at)
        .bind(&session.mode)
        .bind(&session.cwd)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Load one session by id.
    pub async fn get_session(&self, id: Uuid) -> Result<Option<SessionRow>> {
        let raw = sqlx::query_as::<_, SessionRowRaw>(
            r"
            SELECT id, started_at, ended_at, mode, cwd
            FROM sessions
            WHERE id = ?
            ",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        raw.map(session_from_raw).transpose()
    }

    /// List sessions newest-first.
    pub async fn list_sessions(&self, limit: Option<i64>) -> Result<Vec<SessionRow>> {
        let limit = limit.unwrap_or(100);
        let rows = sqlx::query_as::<_, SessionRowRaw>(
            r"
            SELECT id, started_at, ended_at, mode, cwd
            FROM sessions
            ORDER BY started_at DESC
            LIMIT ?
            ",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(session_from_raw).collect()
    }

    /// Mark a session ended (sets `ended_at`).
    pub async fn end_session(&self, id: Uuid) -> Result<()> {
        let ended_at = Utc::now().timestamp();
        sqlx::query("UPDATE sessions SET ended_at = ? WHERE id = ?")
            .bind(ended_at)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Delete a session and all dependent rows for that session.
    pub async fn delete_session(&self, id: Uuid) -> Result<()> {
        let sid = id.to_string();
        sqlx::query(
            r"
            DELETE FROM pipeline_stages
            WHERE pipeline_id IN (SELECT id FROM pipelines WHERE session_id = ?)
            ",
        )
        .bind(&sid)
        .execute(&self.pool)
        .await?;
        sqlx::query("DELETE FROM pipelines WHERE session_id = ?")
            .bind(&sid)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM messages WHERE session_id = ?")
            .bind(&sid)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM agents WHERE session_id = ?")
            .bind(&sid)
            .execute(&self.pool)
            .await?;
        sqlx::query(
            r"
            DELETE FROM snapshot_files
            WHERE snapshot_id IN (SELECT id FROM snapshots WHERE session_id = ?)
            ",
        )
        .bind(&sid)
        .execute(&self.pool)
        .await?;
        sqlx::query("DELETE FROM snapshots WHERE session_id = ?")
            .bind(&sid)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(&sid)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ── Messages ─────────────────────────────────────────────────────────────

    /// Insert a chat message row.
    pub async fn insert_message(&self, message: &NewMessage) -> Result<()> {
        sqlx::query(
            r"
            INSERT INTO messages (
                id, session_id, sender_type, sender_id, sender_tag,
                content, timestamp, pipeline_id, race_id
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            ",
        )
        .bind(message.id.to_string())
        .bind(message.session_id.to_string())
        .bind(&message.sender_type)
        .bind(message.sender_id.map(|u| u.to_string()))
        .bind(&message.sender_tag)
        .bind(&message.content)
        .bind(message.timestamp_ms)
        .bind(message.pipeline_id.map(|u| u.to_string()))
        .bind(message.race_id.map(|u| u.to_string()))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Load one message by id.
    pub async fn get_message(&self, id: Uuid) -> Result<Option<MessageRow>> {
        let raw = sqlx::query_as::<_, MessageRowRaw>(
            r"
            SELECT id, session_id, sender_type, sender_id, sender_tag,
                   content, timestamp, pipeline_id, race_id
            FROM messages
            WHERE id = ?
            ",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        raw.map(message_from_raw).transpose()
    }

    /// List messages for a session in chronological order.
    pub async fn list_messages(
        &self,
        session_id: Uuid,
        limit: Option<i64>,
    ) -> Result<Vec<MessageRow>> {
        let limit = limit.unwrap_or(500);
        let rows = sqlx::query_as::<_, MessageRowRaw>(
            r"
            SELECT id, session_id, sender_type, sender_id, sender_tag,
                   content, timestamp, pipeline_id, race_id
            FROM messages
            WHERE session_id = ?
            ORDER BY timestamp ASC
            LIMIT ?
            ",
        )
        .bind(session_id.to_string())
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(message_from_raw).collect()
    }

    /// Delete one message row.
    pub async fn delete_message(&self, id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM messages WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ── Pipelines ────────────────────────────────────────────────────────────

    /// Insert a pipeline row with status `running`.
    pub async fn insert_pipeline(&self, pipeline: &NewPipeline) -> Result<()> {
        let started_at = Utc::now().timestamp();
        sqlx::query(
            r"
            INSERT INTO pipelines (id, session_id, definition, status, started_at, snapshot_id)
            VALUES (?, ?, ?, 'running', ?, ?)
            ON CONFLICT(id) DO NOTHING
            ",
        )
        .bind(pipeline.id.to_string())
        .bind(pipeline.session_id.to_string())
        .bind(&pipeline.definition)
        .bind(started_at)
        .bind(pipeline.snapshot_id.map(|u| u.to_string()))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Load one pipeline by id.
    pub async fn get_pipeline(&self, id: Uuid) -> Result<Option<PipelineRow>> {
        let raw = sqlx::query_as::<_, PipelineRowRaw>(
            r"
            SELECT id, session_id, definition, status, started_at, completed_at, snapshot_id
            FROM pipelines
            WHERE id = ?
            ",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        raw.map(pipeline_from_raw).transpose()
    }

    /// List pipelines for a session, newest first.
    pub async fn list_pipelines(&self, session_id: Uuid) -> Result<Vec<PipelineRow>> {
        let rows = sqlx::query_as::<_, PipelineRowRaw>(
            r"
            SELECT id, session_id, definition, status, started_at, completed_at, snapshot_id
            FROM pipelines
            WHERE session_id = ?
            ORDER BY started_at DESC
            ",
        )
        .bind(session_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(pipeline_from_raw).collect()
    }

    /// Update pipeline status and optional completion timestamp.
    pub async fn update_pipeline_status(
        &self,
        id: Uuid,
        status: &str,
        completed_at: Option<i64>,
    ) -> Result<()> {
        sqlx::query(
            r"
            UPDATE pipelines
            SET status = ?, completed_at = ?
            WHERE id = ?
            ",
        )
        .bind(status)
        .bind(completed_at)
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Mark a pipeline complete.
    pub async fn complete_pipeline(&self, id: Uuid) -> Result<()> {
        self.update_pipeline_status(id, "complete", Some(Utc::now().timestamp()))
            .await
    }

    /// Mark a pipeline failed.
    pub async fn fail_pipeline(&self, id: Uuid) -> Result<()> {
        self.update_pipeline_status(id, "failed", Some(Utc::now().timestamp()))
            .await
    }

    /// Delete a pipeline and its stages.
    pub async fn delete_pipeline(&self, id: Uuid) -> Result<()> {
        let pid = id.to_string();
        sqlx::query("DELETE FROM pipeline_stages WHERE pipeline_id = ?")
            .bind(&pid)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM pipelines WHERE id = ?")
            .bind(&pid)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Insert a pipeline stage row.
    pub async fn insert_pipeline_stage(&self, stage: &NewPipelineStage) -> Result<()> {
        sqlx::query(
            r"
            INSERT INTO pipeline_stages (
                id, pipeline_id, stage_index, stage_type, target
            )
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(id) DO NOTHING
            ",
        )
        .bind(stage.id.to_string())
        .bind(stage.pipeline_id.to_string())
        .bind(stage.stage_index)
        .bind(&stage.stage_type)
        .bind(&stage.target)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// List stages for a pipeline in stage order.
    pub async fn list_pipeline_stages(&self, pipeline_id: Uuid) -> Result<Vec<PipelineStageRow>> {
        let rows = sqlx::query_as::<_, PipelineStageRowRaw>(
            r"
            SELECT id, pipeline_id, stage_index, stage_type, target,
                   input_text, output_text, started_at, completed_at, exit_code
            FROM pipeline_stages
            WHERE pipeline_id = ?
            ORDER BY stage_index ASC
            ",
        )
        .bind(pipeline_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(pipeline_stage_from_raw).collect()
    }

    /// Record stage output and completion metadata.
    pub async fn update_pipeline_stage(
        &self,
        pipeline_id: Uuid,
        stage_index: i64,
        output_text: &str,
        exit_code: Option<i64>,
    ) -> Result<()> {
        let completed_at = Utc::now().timestamp();
        sqlx::query(
            r"
            UPDATE pipeline_stages
            SET output_text = ?, completed_at = ?, exit_code = ?
            WHERE pipeline_id = ? AND stage_index = ?
            ",
        )
        .bind(output_text)
        .bind(completed_at)
        .bind(exit_code)
        .bind(pipeline_id.to_string())
        .bind(stage_index)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Log an LLM race to `pipelines` + one `pipeline_stages` row per contestant tag.
    pub async fn log_race_start(
        &self,
        hub_session_id: Uuid,
        race_id: Uuid,
        definition: &str,
        tags: &[String],
        snapshot_id: Option<Uuid>,
    ) -> Result<()> {
        self.insert_pipeline(&NewPipeline {
            id: race_id,
            session_id: hub_session_id,
            definition: definition.to_string(),
            snapshot_id,
        })
        .await?;

        for (index, tag) in tags.iter().enumerate() {
            self.insert_pipeline_stage(&NewPipelineStage {
                id: Uuid::new_v4(),
                pipeline_id: race_id,
                stage_index: index as i64,
                stage_type: "agent".into(),
                target: format!("@{tag}"),
            })
            .await?;
        }
        Ok(())
    }

    /// Mark a race pipeline complete.
    pub async fn complete_race(&self, race_id: Uuid) -> Result<()> {
        self.complete_pipeline(race_id).await
    }

    // ── Agents ─────────────────────────────────────────────────────────────────

    /// Convenience wrapper used by racing and moderation callers.
    pub async fn log_message(&self, message: &NewMessage) -> Result<()> {
        self.insert_message(message).await
    }

    /// Register an agent spawn in the session registry.
    pub async fn insert_agent(
        &self,
        id: Uuid,
        session_id: Uuid,
        tag: &str,
        driver_name: &str,
        role: &str,
    ) -> Result<()> {
        self.insert_agent_record(&NewAgent {
            id,
            session_id,
            tag: tag.to_string(),
            driver_name: driver_name.to_string(),
            role: role.to_string(),
        })
        .await
    }

    /// Register an agent from a [`NewAgent`] row.
    pub async fn insert_agent_record(&self, agent: &NewAgent) -> Result<()> {
        let spawned_at = Utc::now().timestamp();
        sqlx::query(
            r"
            INSERT INTO agents (id, session_id, tag, driver_name, role, spawned_at)
            VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO NOTHING
            ",
        )
        .bind(agent.id.to_string())
        .bind(agent.session_id.to_string())
        .bind(&agent.tag)
        .bind(&agent.driver_name)
        .bind(&agent.role)
        .bind(spawned_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Load one agent by id.
    pub async fn get_agent(&self, id: Uuid) -> Result<Option<AgentRow>> {
        let raw = sqlx::query_as::<_, AgentRowRaw>(
            r"
            SELECT id, session_id, tag, driver_name, role, spawned_at, killed_at, kill_reason
            FROM agents
            WHERE id = ?
            ",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        raw.map(agent_from_raw).transpose()
    }

    /// List agents for a session; `active_only` skips rows with `killed_at` set.
    pub async fn list_agents(&self, session_id: Uuid, active_only: bool) -> Result<Vec<AgentRow>> {
        let rows = if active_only {
            sqlx::query_as::<_, AgentRowRaw>(
                r"
                SELECT id, session_id, tag, driver_name, role, spawned_at, killed_at, kill_reason
                FROM agents
                WHERE session_id = ? AND killed_at IS NULL
                ORDER BY spawned_at ASC
                ",
            )
            .bind(session_id.to_string())
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, AgentRowRaw>(
                r"
                SELECT id, session_id, tag, driver_name, role, spawned_at, killed_at, kill_reason
                FROM agents
                WHERE session_id = ?
                ORDER BY spawned_at ASC
                ",
            )
            .bind(session_id.to_string())
            .fetch_all(&self.pool)
            .await?
        };
        rows.into_iter().map(agent_from_raw).collect()
    }

    /// Delete one agent row.
    pub async fn delete_agent(&self, id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM agents WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ── Snapshots ──────────────────────────────────────────────────────────────

    /// Insert a snapshot row (`ON CONFLICT DO NOTHING`).
    pub async fn insert_snapshot(&self, snapshot: &NewSnapshot) -> Result<()> {
        let timestamp = Utc::now().timestamp();
        sqlx::query(
            r"
            INSERT INTO snapshots (
                id, session_id, timestamp, file_count, size_bytes, cwd, trigger
            )
            VALUES (?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO NOTHING
            ",
        )
        .bind(snapshot.id.to_string())
        .bind(snapshot.session_id.to_string())
        .bind(timestamp)
        .bind(snapshot.file_count)
        .bind(snapshot.size_bytes)
        .bind(&snapshot.cwd)
        .bind(&snapshot.trigger)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Load one snapshot by id.
    pub async fn get_snapshot(&self, id: Uuid) -> Result<Option<SnapshotRow>> {
        let raw = sqlx::query_as::<_, SnapshotRowRaw>(
            r"
            SELECT id, session_id, timestamp, file_count, size_bytes, cwd, trigger
            FROM snapshots
            WHERE id = ?
            ",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        raw.map(snapshot_from_raw).transpose()
    }

    /// List snapshots for a session, newest first.
    pub async fn list_snapshots(&self, session_id: Uuid) -> Result<Vec<SnapshotRow>> {
        let rows = sqlx::query_as::<_, SnapshotRowRaw>(
            r"
            SELECT id, session_id, timestamp, file_count, size_bytes, cwd, trigger
            FROM snapshots
            WHERE session_id = ?
            ORDER BY timestamp DESC
            ",
        )
        .bind(session_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(snapshot_from_raw).collect()
    }

    /// List snapshots for a workspace path, newest first.
    pub async fn list_snapshots_for_cwd(&self, cwd: &str) -> Result<Vec<SnapshotRow>> {
        let rows = sqlx::query_as::<_, SnapshotRowRaw>(
            r"
            SELECT id, session_id, timestamp, file_count, size_bytes, cwd, trigger
            FROM snapshots
            WHERE cwd = ?
            ORDER BY timestamp DESC
            ",
        )
        .bind(cwd)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(snapshot_from_raw).collect()
    }

    /// Delete a snapshot and its file manifest rows.
    pub async fn delete_snapshot(&self, id: Uuid) -> Result<()> {
        let sid = id.to_string();
        sqlx::query("DELETE FROM snapshot_files WHERE snapshot_id = ?")
            .bind(&sid)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM snapshots WHERE id = ?")
            .bind(&sid)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Insert one snapshot file manifest row.
    pub async fn insert_snapshot_file(&self, file: &NewSnapshotFile) -> Result<()> {
        sqlx::query(
            r"
            INSERT INTO snapshot_files (id, snapshot_id, rel_path, blake3_hash, status)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(id) DO NOTHING
            ",
        )
        .bind(file.id.to_string())
        .bind(file.snapshot_id.to_string())
        .bind(&file.rel_path)
        .bind(&file.blake3_hash)
        .bind(&file.status)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// List manifest rows for a snapshot.
    pub async fn list_snapshot_files(&self, snapshot_id: Uuid) -> Result<Vec<SnapshotFileRow>> {
        let rows = sqlx::query_as::<_, SnapshotFileRowRaw>(
            r"
            SELECT id, snapshot_id, rel_path, blake3_hash, status
            FROM snapshot_files
            WHERE snapshot_id = ?
            ORDER BY rel_path ASC
            ",
        )
        .bind(snapshot_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(snapshot_file_from_raw).collect()
    }

    /// Update an agent's role string (e.g. after `RoleAssigned`).
    pub async fn update_agent_role(&self, id: Uuid, role: &str) -> Result<()> {
        sqlx::query("UPDATE agents SET role = ? WHERE id = ?")
            .bind(role)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Mark an agent as killed with a blueprint kill_reason string.
    pub async fn mark_agent_offline(&self, id: Uuid, reason: OfflineReason) -> Result<()> {
        let killed_at = Utc::now().timestamp();
        let kill_reason = match reason {
            OfflineReason::Crashed => "crashed",
            OfflineReason::Kicked => "kicked",
            OfflineReason::Banned => "kicked",
            OfflineReason::Natural => "natural",
        };
        sqlx::query(
            r"
            UPDATE agents
            SET killed_at = ?, kill_reason = ?
            WHERE id = ?
            ",
        )
        .bind(killed_at)
        .bind(kill_reason)
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ── Custom roles ───────────────────────────────────────────────────────────

    /// Upsert a custom role (`ON CONFLICT` replaces mask and induction text).
    pub async fn upsert_custom_role(&self, role: &NewCustomRole) -> Result<()> {
        sqlx::query(
            r"
            INSERT INTO custom_roles (name, permissions_mask, induction_override)
            VALUES (?, ?, ?)
            ON CONFLICT(name) DO UPDATE SET
                permissions_mask = excluded.permissions_mask,
                induction_override = excluded.induction_override
            ",
        )
        .bind(&role.name)
        .bind(role.permissions_mask)
        .bind(&role.induction_override)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Load one custom role by name.
    pub async fn get_custom_role(&self, name: &str) -> Result<Option<CustomRoleRow>> {
        let raw = sqlx::query_as::<_, CustomRoleRowRaw>(
            r"
            SELECT name, permissions_mask, induction_override
            FROM custom_roles
            WHERE name = ?
            ",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;
        Ok(raw.map(|r| CustomRoleRow {
            name: r.name,
            permissions_mask: r.permissions_mask,
            induction_override: r.induction_override,
        }))
    }

    /// List all custom roles alphabetically.
    pub async fn list_custom_roles(&self) -> Result<Vec<CustomRoleRow>> {
        let rows = sqlx::query_as::<_, CustomRoleRowRaw>(
            r"
            SELECT name, permissions_mask, induction_override
            FROM custom_roles
            ORDER BY name ASC
            ",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| CustomRoleRow {
                name: r.name,
                permissions_mask: r.permissions_mask,
                induction_override: r.induction_override,
            })
            .collect())
    }

    /// Delete a custom role by name.
    pub async fn delete_custom_role(&self, name: &str) -> Result<()> {
        sqlx::query("DELETE FROM custom_roles WHERE name = ?")
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ── PTY debug log ──────────────────────────────────────────────────────────

    /// Insert one zstd-compressed PTY debug blob.
    pub async fn insert_pty_debug(&self, entry: &NewPtyDebugEntry) -> Result<()> {
        sqlx::query(
            r"
            INSERT INTO pty_debug_log (id, agent_id, timestamp, raw_bytes)
            VALUES (?, ?, ?, ?)
            ON CONFLICT(id) DO NOTHING
            ",
        )
        .bind(entry.id.to_string())
        .bind(entry.agent_id.to_string())
        .bind(entry.timestamp)
        .bind(&entry.raw_bytes)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// List debug entries for an agent, newest first.
    pub async fn list_pty_debug(
        &self,
        agent_id: Uuid,
        limit: Option<i64>,
    ) -> Result<Vec<PtyDebugRow>> {
        let limit = limit.unwrap_or(100);
        let rows = sqlx::query_as::<_, PtyDebugRowRaw>(
            r"
            SELECT id, agent_id, timestamp, raw_bytes
            FROM pty_debug_log
            WHERE agent_id = ?
            ORDER BY timestamp DESC
            LIMIT ?
            ",
        )
        .bind(agent_id.to_string())
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(pty_debug_from_raw).collect()
    }

    /// Delete debug rows older than 48 hours (blueprint rotation policy).
    pub async fn rotate_pty_debug_log(&self) -> Result<u64> {
        let cutoff = Utc::now().timestamp() - 48 * 3600;
        let result = sqlx::query("DELETE FROM pty_debug_log WHERE timestamp < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    /// Delete all PTY debug rows for one agent.
    pub async fn delete_pty_debug_for_agent(&self, agent_id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM pty_debug_log WHERE agent_id = ?")
            .bind(agent_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Persists bus events to the Part 14 schema (blueprint §7.2).
    pub async fn log_bus_event(&self, session_id: Uuid, event: &BusEvent) -> Result<()> {
        match event {
            BusEvent::UserMessage {
                content, timestamp, ..
            } => {
                self.insert_message(&NewMessage {
                    id: Uuid::new_v4(),
                    session_id,
                    sender_type: "user".into(),
                    sender_id: None,
                    sender_tag: "User".into(),
                    content: content.clone(),
                    timestamp_ms: timestamp_to_ms(timestamp),
                    pipeline_id: None,
                    race_id: None,
                })
                .await?;
            }
            BusEvent::AgentMessage {
                id,
                tag,
                content,
                timestamp,
                race_session_id,
            } => {
                self.insert_message(&NewMessage {
                    id: Uuid::new_v4(),
                    session_id,
                    sender_type: "agent".into(),
                    sender_id: Some(*id),
                    sender_tag: tag.clone(),
                    content: content.clone(),
                    timestamp_ms: timestamp_to_ms(timestamp),
                    pipeline_id: None,
                    race_id: *race_session_id,
                })
                .await?;
            }
            BusEvent::SystemMessage { content, timestamp } => {
                self.insert_message(&NewMessage {
                    id: Uuid::new_v4(),
                    session_id,
                    sender_type: "system".into(),
                    sender_id: None,
                    sender_tag: "System".into(),
                    content: content.clone(),
                    timestamp_ms: timestamp_to_ms(timestamp),
                    pipeline_id: None,
                    race_id: None,
                })
                .await?;
            }
            BusEvent::AgentOnline { id, tag, role } => {
                let driver = tag
                    .trim_start_matches('@')
                    .split('-')
                    .next()
                    .unwrap_or("unknown");
                self.insert_agent(*id, session_id, tag, driver, role)
                    .await?;
                self.insert_audit_message(session_id, &format!("{tag} is online ({role})"))
                    .await?;
            }
            BusEvent::AgentOffline { id, tag, reason } => {
                self.mark_agent_offline(*id, reason.clone()).await?;
                self.insert_audit_message(session_id, &format!("{tag} went offline ({reason:?})"))
                    .await?;
            }
            BusEvent::PipelineStarted {
                pipeline_id,
                definition,
            } => {
                self.insert_pipeline(&NewPipeline {
                    id: *pipeline_id,
                    session_id,
                    definition: definition.clone(),
                    snapshot_id: None,
                })
                .await?;
                self.insert_audit_message(session_id, &format!("Pipeline started: {definition}"))
                    .await?;
            }
            BusEvent::PipelineStageComplete {
                pipeline_id,
                stage,
                output_preview,
            } => {
                self.update_pipeline_stage(*pipeline_id, *stage as i64, output_preview, None)
                    .await?;
                self.insert_audit_message(
                    session_id,
                    &format!("Pipeline {pipeline_id} stage {stage} complete: {output_preview}"),
                )
                .await?;
            }
            BusEvent::PipelineComplete { pipeline_id } => {
                self.complete_pipeline(*pipeline_id).await?;
                self.insert_audit_message(session_id, "Pipeline complete")
                    .await?;
            }
            BusEvent::PipelineFailed {
                pipeline_id,
                stage,
                error,
            } => {
                self.fail_pipeline(*pipeline_id).await?;
                self.insert_audit_message(
                    session_id,
                    &format!("Pipeline failed at stage {stage}: {error}"),
                )
                .await?;
            }
            BusEvent::RoleAssigned { agent_id, role, .. } => {
                self.update_agent_role(*agent_id, role).await?;
                self.insert_audit_message(session_id, &format!("Role assigned: {role}"))
                    .await?;
            }
            BusEvent::SnapshotCreated { file_count, .. } => {
                // VFS `create_snapshot` writes the full row; bus path only audits.
                self.insert_audit_message(
                    session_id,
                    &format!("Snapshot created ({file_count} files)"),
                )
                .await?;
            }
            BusEvent::AgentSpawnStarted { tag, driver, .. } => {
                self.insert_audit_message(session_id, &format!("Spawn started @{tag} ({driver})"))
                    .await?;
            }
            BusEvent::SpawnTrace { .. } | BusEvent::PtyIoTrace { .. } => {}
            other => {
                self.insert_audit_message(session_id, &format!("{other:?}"))
                    .await?;
            }
        }
        Ok(())
    }

    async fn insert_audit_message(&self, session_id: Uuid, content: &str) -> Result<()> {
        self.insert_message(&NewMessage {
            id: Uuid::new_v4(),
            session_id,
            sender_type: "system".into(),
            sender_id: None,
            sender_tag: "System".into(),
            content: content.to_string(),
            timestamp_ms: Utc::now().timestamp_millis(),
            pipeline_id: None,
            race_id: None,
        })
        .await
    }
}

fn timestamp_to_ms(ts: &DateTime<Utc>) -> i64 {
    ts.timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{BusEvent, MessageTarget};
    use crate::db::schema::{INDEXES, TABLES, TABLE_COLUMNS};
    use chrono::TimeZone;
    use sqlx::Row;
    use tempfile::NamedTempFile;

    async fn fresh_db() -> (DbClient, Uuid, tempfile::TempPath) {
        let file = NamedTempFile::new().expect("temp db");
        let path = file.path().to_path_buf();
        let url = format!("sqlite://{}", path.display());
        let db = DbClient::init_pool(&url).await.expect("pool");
        db.run_migrations().await.expect("migrate");
        let session_id = Uuid::new_v4();
        db.insert_session(&NewSession {
            id: session_id,
            mode: "group_chat".into(),
            cwd: "/tmp/agenthub".into(),
        })
        .await
        .expect("session");
        (db, session_id, file.into_temp_path())
    }

    #[tokio::test]
    async fn db_migrations_apply_on_fresh_database() {
        let (_db, _session, _guard) = fresh_db().await;
    }

    #[tokio::test]
    async fn db_schema_matches_blueprint_part14_tables_and_indexes() {
        let (db, _session, _guard) = fresh_db().await;

        let tables: Vec<String> = sqlx::query_scalar(
            r"
            SELECT name FROM sqlite_master
            WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name != '_sqlx_migrations'
            ORDER BY name
            ",
        )
        .fetch_all(&db.pool)
        .await
        .expect("tables");

        for expected in TABLES {
            assert!(
                tables.iter().any(|t| t == expected),
                "missing table {expected}: got {tables:?}"
            );
        }
        assert_eq!(tables.len(), TABLES.len(), "unexpected extra tables");

        let indexes: Vec<String> = sqlx::query_scalar(
            r"
            SELECT name FROM sqlite_master
            WHERE type = 'index' AND name NOT LIKE 'sqlite_%'
            ORDER BY name
            ",
        )
        .fetch_all(&db.pool)
        .await
        .expect("indexes");

        for expected in INDEXES {
            assert!(
                indexes.iter().any(|i| i == expected),
                "missing index {expected}: got {indexes:?}"
            );
        }
        assert_eq!(indexes.len(), INDEXES.len());
    }

    #[tokio::test]
    async fn db_schema_columns_match_blueprint_part14() {
        let (db, _session, _guard) = fresh_db().await;

        for (table, expected_cols) in TABLE_COLUMNS {
            let cols: Vec<String> = sqlx::query_scalar(
                r"
                SELECT name FROM pragma_table_info(?)
                ORDER BY cid
                ",
            )
            .bind(table)
            .fetch_all(&db.pool)
            .await
            .unwrap_or_else(|e| panic!("pragma_table_info({table}): {e}"));

            assert_eq!(
                cols,
                expected_cols
                    .iter()
                    .map(|c| (*c).to_string())
                    .collect::<Vec<_>>(),
                "column mismatch for table {table}"
            );
        }
    }

    #[tokio::test]
    async fn db_bus_logs_user_message_to_messages_table() {
        let (db, session_id, _guard) = fresh_db().await;
        let ts = Utc
            .with_ymd_and_hms(2026, 5, 29, 12, 0, 0)
            .single()
            .expect("ts");

        db.log_bus_event(
            session_id,
            &BusEvent::UserMessage {
                content: "hello agents".into(),
                timestamp: ts,
                target: MessageTarget::Broadcast,
            },
        )
        .await
        .expect("log");

        let row = sqlx::query(
            r"
            SELECT sender_type, sender_tag, content
            FROM messages
            WHERE session_id = ?
            ",
        )
        .bind(session_id.to_string())
        .fetch_one(&db.pool)
        .await
        .expect("row");

        assert_eq!(row.get::<String, _>("sender_type"), "user");
        assert_eq!(row.get::<String, _>("sender_tag"), "User");
        assert_eq!(row.get::<String, _>("content"), "hello agents");
    }

    #[tokio::test]
    async fn db_bus_logs_agent_message_with_sender_id() {
        let (db, session_id, _guard) = fresh_db().await;
        let agent_id = Uuid::new_v4();
        let ts = Utc::now();

        db.log_bus_event(
            session_id,
            &BusEvent::AgentMessage {
                id: agent_id,
                tag: "@gemini-1".into(),
                content: "response body".into(),
                timestamp: ts,
                race_session_id: None,
            },
        )
        .await
        .expect("log");

        let row = sqlx::query(
            r"
            SELECT sender_type, sender_id, sender_tag, content
            FROM messages
            WHERE session_id = ?
            ",
        )
        .bind(session_id.to_string())
        .fetch_one(&db.pool)
        .await
        .expect("row");

        assert_eq!(row.get::<String, _>("sender_type"), "agent");
        assert_eq!(row.get::<String, _>("sender_id"), agent_id.to_string());
        assert_eq!(row.get::<String, _>("sender_tag"), "@gemini-1");
        assert_eq!(row.get::<String, _>("content"), "response body");
    }

    #[tokio::test]
    async fn db_bus_logs_agent_online_to_agents_table() {
        let (db, session_id, _guard) = fresh_db().await;
        let agent_id = Uuid::new_v4();

        db.log_bus_event(
            session_id,
            &BusEvent::AgentOnline {
                id: agent_id,
                tag: "@gemini-1".into(),
                role: "Builder".into(),
            },
        )
        .await
        .expect("log");

        let count: i64 = sqlx::query_scalar(r"SELECT COUNT(*) FROM agents WHERE id = ?")
            .bind(agent_id.to_string())
            .fetch_one(&db.pool)
            .await
            .expect("count");

        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn db_bus_logs_system_message() {
        let (db, session_id, _guard) = fresh_db().await;
        let ts = Utc::now();

        db.log_bus_event(
            session_id,
            &BusEvent::SystemMessage {
                content: "snapshot complete".into(),
                timestamp: ts,
            },
        )
        .await
        .expect("log");

        let row = sqlx::query(
            r"
            SELECT sender_type, sender_tag, content
            FROM messages
            WHERE session_id = ? AND sender_type = 'system'
            ",
        )
        .bind(session_id.to_string())
        .fetch_one(&db.pool)
        .await
        .expect("row");

        assert_eq!(row.get::<String, _>("sender_tag"), "System");
        assert_eq!(row.get::<String, _>("content"), "snapshot complete");
    }

    #[tokio::test]
    async fn db_bus_logs_pipeline_started_to_pipelines_table() {
        let (db, session_id, _guard) = fresh_db().await;
        let pipeline_id = Uuid::new_v4();

        db.log_bus_event(
            session_id,
            &BusEvent::PipelineStarted {
                pipeline_id,
                definition: "@gemini-1 hello | > echo world".into(),
            },
        )
        .await
        .expect("log");

        let status: String = sqlx::query_scalar(r"SELECT status FROM pipelines WHERE id = ?")
            .bind(pipeline_id.to_string())
            .fetch_one(&db.pool)
            .await
            .expect("status");

        assert_eq!(status, "running");
    }

    #[tokio::test]
    async fn db_wal_mode_enabled_after_migration() {
        let (db, _session, _guard) = fresh_db().await;
        let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&db.pool)
            .await
            .expect("pragma");
        assert_eq!(mode.to_lowercase(), "wal");
    }

    #[tokio::test]
    async fn db_bus_logs_pipeline_complete() {
        let (db, session_id, _guard) = fresh_db().await;
        let pipeline_id = Uuid::new_v4();

        db.log_bus_event(
            session_id,
            &BusEvent::PipelineStarted {
                pipeline_id,
                definition: "test".into(),
            },
        )
        .await
        .expect("start");

        db.log_bus_event(session_id, &BusEvent::PipelineComplete { pipeline_id })
            .await
            .expect("complete");

        let status: String = sqlx::query_scalar(r"SELECT status FROM pipelines WHERE id = ?")
            .bind(pipeline_id.to_string())
            .fetch_one(&db.pool)
            .await
            .expect("status");

        assert_eq!(status, "complete");
    }

    #[tokio::test]
    async fn db_bus_logs_agent_offline_marks_killed() {
        let (db, session_id, _guard) = fresh_db().await;
        let agent_id = Uuid::new_v4();

        db.log_bus_event(
            session_id,
            &BusEvent::AgentOnline {
                id: agent_id,
                tag: "@gemini-1".into(),
                role: "Builder".into(),
            },
        )
        .await
        .expect("online");

        db.log_bus_event(
            session_id,
            &BusEvent::AgentOffline {
                id: agent_id,
                tag: "@gemini-1".into(),
                reason: OfflineReason::Natural,
            },
        )
        .await
        .expect("offline");

        let row = db.get_agent(agent_id).await.expect("get").expect("row");
        assert!(row.killed_at.is_some());
        assert_eq!(row.kill_reason.as_deref(), Some("natural"));
    }

    #[tokio::test]
    async fn db_foreign_keys_and_synchronous_pragmas_after_migration() {
        let (db, _session, _guard) = fresh_db().await;
        let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&db.pool)
            .await
            .expect("fk");
        assert_eq!(fk, 1);

        // SQLite returns 1 for NORMAL synchronous mode.
        let sync: i64 = sqlx::query_scalar("PRAGMA synchronous")
            .fetch_one(&db.pool)
            .await
            .expect("sync");
        assert_eq!(sync, 1);
    }

    #[tokio::test]
    async fn db_custom_roles_crud_roundtrip() {
        let (db, _session, _guard) = fresh_db().await;

        db.upsert_custom_role(&NewCustomRole {
            name: "Reviewer".into(),
            permissions_mask: 0b1010,
            induction_override: Some("You review code.".into()),
        })
        .await
        .expect("upsert");

        let row = db
            .get_custom_role("Reviewer")
            .await
            .expect("get")
            .expect("row");
        assert_eq!(row.permissions_mask, 0b1010);
        assert_eq!(row.induction_override.as_deref(), Some("You review code."));

        db.delete_custom_role("Reviewer").await.expect("delete");
        assert!(db.get_custom_role("Reviewer").await.expect("get").is_none());
    }

    #[tokio::test]
    async fn db_pty_debug_insert_list_and_rotate() {
        let (db, _session, _guard) = fresh_db().await;
        let agent_id = Uuid::new_v4();
        let entry_id = Uuid::new_v4();
        let old_ts = Utc::now().timestamp() - 49 * 3600;

        db.insert_pty_debug(&NewPtyDebugEntry {
            id: entry_id,
            agent_id,
            timestamp: old_ts,
            raw_bytes: crate::db::compress_pty_bytes(&[1, 2, 3]).expect("compress"),
        })
        .await
        .expect("insert old");

        db.insert_pty_debug(&NewPtyDebugEntry {
            id: Uuid::new_v4(),
            agent_id,
            timestamp: Utc::now().timestamp(),
            raw_bytes: crate::db::compress_pty_bytes(&[4, 5]).expect("compress"),
        })
        .await
        .expect("insert new");

        let deleted = db.rotate_pty_debug_log().await.expect("rotate");
        assert_eq!(deleted, 1);

        let rows = db.list_pty_debug(agent_id, None).await.expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].raw_bytes, vec![4, 5]);
    }

    #[tokio::test]
    async fn db_insert_session_and_message_roundtrip() {
        let (db, session_id, _guard) = fresh_db().await;
        let msg_id = Uuid::new_v4();
        let ts = 1_706_000_000_000_i64;

        db.insert_message(&NewMessage {
            id: msg_id,
            session_id,
            sender_type: "user".into(),
            sender_id: None,
            sender_tag: "User".into(),
            content: "direct insert".into(),
            timestamp_ms: ts,
            pipeline_id: None,
            race_id: None,
        })
        .await
        .expect("insert");

        let row = sqlx::query(
            r"
            SELECT id, content, timestamp
            FROM messages
            WHERE id = ?
            ",
        )
        .bind(msg_id.to_string())
        .fetch_one(&db.pool)
        .await
        .expect("row");

        assert_eq!(row.get::<String, _>("id"), msg_id.to_string());
        assert_eq!(row.get::<String, _>("content"), "direct insert");
        assert_eq!(row.get::<i64, _>("timestamp"), ts);
    }
}
