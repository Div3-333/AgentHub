//! Blueprint Part 14 table, index, and column names (for migration verification).

pub const TABLES: &[&str] = &[
    "sessions",
    "agents",
    "messages",
    "pipelines",
    "pipeline_stages",
    "snapshots",
    "snapshot_files",
    "custom_roles",
    "pty_debug_log",
];

pub const INDEXES: &[&str] = &[
    "idx_messages_session",
    "idx_messages_sender",
    "idx_pipeline_stages",
    "idx_snapshot_files",
    "idx_pty_debug_agent",
];

/// `(table_name, column_names in blueprint order)`
pub const TABLE_COLUMNS: &[(&str, &[&str])] = &[
    ("sessions", &["id", "started_at", "ended_at", "mode", "cwd"]),
    (
        "agents",
        &[
            "id",
            "session_id",
            "tag",
            "driver_name",
            "role",
            "spawned_at",
            "killed_at",
            "kill_reason",
        ],
    ),
    (
        "messages",
        &[
            "id",
            "session_id",
            "sender_type",
            "sender_id",
            "sender_tag",
            "content",
            "timestamp",
            "pipeline_id",
            "race_id",
        ],
    ),
    (
        "pipelines",
        &[
            "id",
            "session_id",
            "definition",
            "status",
            "started_at",
            "completed_at",
            "snapshot_id",
        ],
    ),
    (
        "pipeline_stages",
        &[
            "id",
            "pipeline_id",
            "stage_index",
            "stage_type",
            "target",
            "input_text",
            "output_text",
            "started_at",
            "completed_at",
            "exit_code",
        ],
    ),
    (
        "snapshots",
        &[
            "id",
            "session_id",
            "timestamp",
            "file_count",
            "size_bytes",
            "cwd",
            "trigger",
        ],
    ),
    (
        "snapshot_files",
        &["id", "snapshot_id", "rel_path", "blake3_hash", "status"],
    ),
    (
        "custom_roles",
        &["name", "permissions_mask", "induction_override"],
    ),
    (
        "pty_debug_log",
        &["id", "agent_id", "timestamp", "raw_bytes"],
    ),
];
