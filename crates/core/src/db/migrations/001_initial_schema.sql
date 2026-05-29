-- @disable-transaction
-- WAL / synchronous / foreign_keys pragmas are applied in DbClient::run_migrations
-- after this file runs (sqlx migrations execute inside a transaction).

-- ─────────────────────────────────────────────────────────────────────────────
-- Session Log: One row per AgentHub launch.
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS sessions (
    id          TEXT PRIMARY KEY NOT NULL,  -- UUID
    started_at  INTEGER NOT NULL,           -- Unix timestamp (seconds)
    ended_at    INTEGER,                    -- NULL if still running
    mode        TEXT NOT NULL,             -- 'dm', 'group_chat', 'server'
    cwd         TEXT NOT NULL              -- Absolute path of working directory
);

-- ─────────────────────────────────────────────────────────────────────────────
-- Agent Registry: One row per spawned agent instance (per session).
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS agents (
    id           TEXT PRIMARY KEY NOT NULL, -- UUID
    session_id   TEXT NOT NULL REFERENCES sessions(id),
    tag          TEXT NOT NULL,             -- '@gemini-1'
    driver_name  TEXT NOT NULL,             -- 'gemini'
    role         TEXT NOT NULL,             -- 'Builder'
    spawned_at   INTEGER NOT NULL,
    killed_at    INTEGER,
    kill_reason  TEXT                       -- 'kicked', 'crashed', 'natural'
);

-- ─────────────────────────────────────────────────────────────────────────────
-- Chat Log: Every message visible in any chat pane.
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS messages (
    id           TEXT PRIMARY KEY NOT NULL, -- UUID
    session_id   TEXT NOT NULL REFERENCES sessions(id),
    sender_type  TEXT NOT NULL,             -- 'user', 'agent', 'system'
    sender_id    TEXT,                      -- Agent UUID if sender_type='agent'
    sender_tag   TEXT NOT NULL,             -- '@gemini-1' or 'User' or 'System'
    content      TEXT NOT NULL,
    timestamp    INTEGER NOT NULL,          -- Unix timestamp (milliseconds)
    pipeline_id  TEXT,                      -- UUID if part of a pipeline
    race_id      TEXT                       -- UUID if part of an LLM race
);

-- ─────────────────────────────────────────────────────────────────────────────
-- Pipeline Log: One row per pipeline execution.
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS pipelines (
    id           TEXT PRIMARY KEY NOT NULL, -- UUID
    session_id   TEXT NOT NULL REFERENCES sessions(id),
    definition   TEXT NOT NULL,             -- Raw pipeline string typed by user
    status       TEXT NOT NULL,             -- 'running', 'complete', 'failed'
    started_at   INTEGER NOT NULL,
    completed_at INTEGER,
    snapshot_id  TEXT                       -- VFS snapshot taken before execution
);

CREATE TABLE IF NOT EXISTS pipeline_stages (
    id           TEXT PRIMARY KEY NOT NULL,
    pipeline_id  TEXT NOT NULL REFERENCES pipelines(id),
    stage_index  INTEGER NOT NULL,
    stage_type   TEXT NOT NULL,             -- 'agent', 'unix'
    target       TEXT NOT NULL,             -- agent tag or unix command
    input_text   TEXT,
    output_text  TEXT,
    started_at   INTEGER,
    completed_at INTEGER,
    exit_code    INTEGER                    -- NULL for agent stages
);

-- ─────────────────────────────────────────────────────────────────────────────
-- VFS Snapshots: Workspace checkpoints.
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS snapshots (
    id           TEXT PRIMARY KEY NOT NULL, -- UUID
    session_id   TEXT NOT NULL REFERENCES sessions(id),
    timestamp    INTEGER NOT NULL,
    file_count   INTEGER NOT NULL,
    size_bytes   INTEGER NOT NULL,
    cwd          TEXT NOT NULL,
    trigger      TEXT NOT NULL             -- 'pipeline', 'race', 'spar', 'manual'
);

CREATE TABLE IF NOT EXISTS snapshot_files (
    id           TEXT PRIMARY KEY NOT NULL,
    snapshot_id  TEXT NOT NULL REFERENCES snapshots(id),
    rel_path     TEXT NOT NULL,            -- Relative to CWD
    blake3_hash  TEXT NOT NULL,
    status       TEXT NOT NULL            -- 'copied', 'unchanged'
);

-- ─────────────────────────────────────────────────────────────────────────────
-- Role Registry: Custom roles persisted across sessions.
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS custom_roles (
    name                 TEXT PRIMARY KEY NOT NULL,
    permissions_mask     INTEGER NOT NULL,          -- Bitflags as u64
    induction_override   TEXT                        -- NULL = use default template
);

-- ─────────────────────────────────────────────────────────────────────────────
-- Debug Log: Raw PTY byte streams for driver profile debugging.
-- Stored as compressed blobs. Rotated after 48 hours.
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS pty_debug_log (
    id           TEXT PRIMARY KEY NOT NULL,
    agent_id     TEXT NOT NULL,
    timestamp    INTEGER NOT NULL,
    raw_bytes    BLOB NOT NULL             -- zstd-compressed raw PTY output
);

-- Indices for common query patterns
CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, timestamp);
CREATE INDEX IF NOT EXISTS idx_messages_sender  ON messages(sender_id);
CREATE INDEX IF NOT EXISTS idx_pipeline_stages  ON pipeline_stages(pipeline_id, stage_index);
CREATE INDEX IF NOT EXISTS idx_snapshot_files   ON snapshot_files(snapshot_id);
CREATE INDEX IF NOT EXISTS idx_pty_debug_agent  ON pty_debug_log(agent_id, timestamp);
