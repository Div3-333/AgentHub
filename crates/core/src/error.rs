use thiserror::Error;

#[derive(Error, Debug)]
pub enum AgentHubError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("PTY error: {0}")]
    Pty(String),

    #[error("Sanitizer error: {0}")]
    Sanitizer(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("RBAC error: agent {agent_id} lacks permission {permission}")]
    PermissionDenied {
        agent_id: uuid::Uuid,
        permission: String,
    },

    #[error("Agent not found: {0}")]
    AgentNotFound(uuid::Uuid),

    #[error("Role not found: {0}")]
    RoleNotFound(String),

    #[error("Pipeline parse error at position {pos}: {msg}")]
    PipelineParse { pos: usize, msg: String },

    #[error("Pipeline execution error in stage {stage}: {msg}")]
    PipelineExecution { stage: usize, msg: String },

    #[error("VFS snapshot error: {0}")]
    Snapshot(String),

    #[error("VFS revert error: {0}")]
    Revert(String),

    #[error("Context injection error: {0}")]
    Context(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Driver profile error for CLI '{driver}': {msg}")]
    DriverProfile { driver: String, msg: String },

    #[error("Induction protocol timed out for agent {0}")]
    InductionTimeout(uuid::Uuid),

    #[error("DM mode supports only one agent; kick @{existing_tag} to spawn @{new_tag}")]
    DmAgentLimit {
        existing_id: uuid::Uuid,
        existing_tag: String,
        new_tag: String,
    },

    #[error("Cannot switch to DM mode while {count} agents are active")]
    DmModeTransition { count: usize },

    #[error("Channel not found: {0}")]
    ChannelNotFound(String),

    #[error("Channel already exists: {0}")]
    ChannelAlreadyExists(String),

    #[error("Agent output blocked: filesystem write detected without WRITE_FILES permission")]
    WriteFilesBlocked { agent_id: uuid::Uuid },

    #[error("Rate limit detected for agent {0}")]
    RateLimit(uuid::Uuid),

    #[error("Graph error: {0}")]
    Graph(#[from] GraphError),
}

#[derive(Error, Debug)]
pub enum GraphError {
    #[error("Cycle detected in pipeline graph")]
    CycleDetected,
    #[error("Node not found: {0}")]
    NodeNotFound(String),
}

impl AgentHubError {
    /// POSIX exit code for deterministic shell integration (blueprint §3).
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        u8::from(self)
    }
}

// POSIX exit code mapping for deterministic shell integration
impl From<&AgentHubError> for u8 {
    fn from(e: &AgentHubError) -> u8 {
        match e {
            AgentHubError::Io(_) => 1,
            AgentHubError::Pty(_) => 2,
            AgentHubError::Database(_) => 3,
            AgentHubError::AgentNotFound(_) => 4,
            AgentHubError::PipelineParse { .. } => 5,
            AgentHubError::PipelineExecution { .. } => 6,
            AgentHubError::Snapshot(_) => 7,
            AgentHubError::Revert(_) => 8,
            AgentHubError::InductionTimeout(_) => 9,
            AgentHubError::RateLimit(_) => 10,
            AgentHubError::PermissionDenied { .. } => 13, // EACCES
            // Blueprint §3: all other variants map to 255
            AgentHubError::Sanitizer(_)
            | AgentHubError::Serialization(_)
            | AgentHubError::RoleNotFound(_)
            | AgentHubError::Context(_)
            | AgentHubError::Config(_)
            | AgentHubError::DriverProfile { .. }
            | AgentHubError::Graph(_)
            | AgentHubError::DmAgentLimit { .. }
            | AgentHubError::DmModeTransition { .. }
            | AgentHubError::ChannelNotFound(_)
            | AgentHubError::ChannelAlreadyExists(_)
            | AgentHubError::WriteFilesBlocked { .. } => 255,
        }
    }
}

pub type Result<T> = std::result::Result<T, AgentHubError>;

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    const MAPPED_CODES: &[u8] = &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 13, 255];

    fn exit_code(err: AgentHubError) -> u8 {
        err.exit_code()
    }

    #[test]
    fn exit_io() {
        let err = AgentHubError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "missing file",
        ));
        assert_eq!(exit_code(err), 1);
    }

    #[test]
    fn exit_pty() {
        assert_eq!(exit_code(AgentHubError::Pty("pty failed".into())), 2);
    }

    #[test]
    fn exit_database() {
        let err = AgentHubError::Database(sqlx::Error::RowNotFound);
        assert_eq!(exit_code(err), 3);
    }

    #[test]
    fn exit_agent_not_found() {
        let id = Uuid::new_v4();
        assert_eq!(exit_code(AgentHubError::AgentNotFound(id)), 4);
    }

    #[test]
    fn exit_pipeline_parse() {
        let err = AgentHubError::PipelineParse {
            pos: 12,
            msg: "unexpected token".into(),
        };
        assert_eq!(exit_code(err), 5);
    }

    #[test]
    fn exit_pipeline_execution() {
        let err = AgentHubError::PipelineExecution {
            stage: 2,
            msg: "stage failed".into(),
        };
        assert_eq!(exit_code(err), 6);
    }

    #[test]
    fn exit_snapshot() {
        assert_eq!(
            exit_code(AgentHubError::Snapshot("snapshot failed".into())),
            7
        );
    }

    #[test]
    fn exit_revert() {
        assert_eq!(exit_code(AgentHubError::Revert("revert failed".into())), 8);
    }

    #[test]
    fn exit_induction_timeout() {
        let id = Uuid::new_v4();
        assert_eq!(exit_code(AgentHubError::InductionTimeout(id)), 9);
    }

    #[test]
    fn exit_rate_limit() {
        let id = Uuid::new_v4();
        assert_eq!(exit_code(AgentHubError::RateLimit(id)), 10);
    }

    #[test]
    fn exit_permission_denied() {
        let err = AgentHubError::PermissionDenied {
            agent_id: Uuid::new_v4(),
            permission: "write".into(),
        };
        assert_eq!(exit_code(err), 13);
    }

    #[test]
    fn exit_sanitizer() {
        assert_eq!(exit_code(AgentHubError::Sanitizer("blocked".into())), 255);
    }

    #[test]
    fn exit_serialization() {
        let json_err: serde_json::Error =
            serde_json::from_str::<i32>("not-json").expect_err("invalid json");
        assert_eq!(exit_code(AgentHubError::Serialization(json_err)), 255);
    }

    #[test]
    fn exit_role_not_found() {
        assert_eq!(exit_code(AgentHubError::RoleNotFound("admin".into())), 255);
    }

    #[test]
    fn exit_context() {
        assert_eq!(
            exit_code(AgentHubError::Context("inject failed".into())),
            255
        );
    }

    #[test]
    fn exit_config() {
        assert_eq!(exit_code(AgentHubError::Config("bad config".into())), 255);
    }

    #[test]
    fn exit_driver_profile() {
        let err = AgentHubError::DriverProfile {
            driver: "cursor".into(),
            msg: "invalid profile".into(),
        };
        assert_eq!(exit_code(err), 255);
    }

    #[test]
    fn exit_graph_cycle_detected() {
        let err = AgentHubError::Graph(GraphError::CycleDetected);
        assert_eq!(exit_code(err), 255);
    }

    #[test]
    fn exit_graph_node_not_found() {
        let err = AgentHubError::Graph(GraphError::NodeNotFound("node-1".into()));
        assert_eq!(exit_code(err), 255);
    }

    #[test]
    fn exit_dm_agent_limit() {
        let err = AgentHubError::DmAgentLimit {
            existing_id: Uuid::new_v4(),
            existing_tag: "gemini".into(),
            new_tag: "claude".into(),
        };
        assert_eq!(exit_code(err), 255);
    }

    #[test]
    fn exit_dm_mode_transition() {
        assert_eq!(exit_code(AgentHubError::DmModeTransition { count: 2 }), 255);
    }

    #[test]
    fn exit_channel_not_found() {
        assert_eq!(
            exit_code(AgentHubError::ChannelNotFound("backend".into())),
            255
        );
    }

    #[test]
    fn exit_channel_already_exists() {
        assert_eq!(
            exit_code(AgentHubError::ChannelAlreadyExists("general".into())),
            255
        );
    }

    #[test]
    fn exit_write_files_blocked() {
        let id = Uuid::new_v4();
        assert_eq!(
            exit_code(AgentHubError::WriteFilesBlocked { agent_id: id }),
            255
        );
    }

    #[test]
    fn from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::Other, "io");
        let err: AgentHubError = io_err.into();
        assert!(matches!(err, AgentHubError::Io(_)));
        assert_eq!(exit_code(err), 1);
    }

    #[test]
    fn from_database_error() {
        let err: AgentHubError = sqlx::Error::RowNotFound.into();
        assert!(matches!(err, AgentHubError::Database(_)));
        assert_eq!(exit_code(err), 3);
    }

    #[test]
    fn from_serialization_error() {
        let json_err: serde_json::Error =
            serde_json::from_str::<i32>("not-json").expect_err("invalid json");
        let err: AgentHubError = json_err.into();
        assert!(matches!(err, AgentHubError::Serialization(_)));
        assert_eq!(exit_code(err), 255);
    }

    #[test]
    fn from_graph_error_cycle() {
        let err: AgentHubError = GraphError::CycleDetected.into();
        assert!(matches!(
            err,
            AgentHubError::Graph(GraphError::CycleDetected)
        ));
        assert_eq!(exit_code(err), 255);
    }

    #[test]
    fn from_graph_error_node_not_found() {
        let err: AgentHubError = GraphError::NodeNotFound("stage-a".into()).into();
        assert!(matches!(
            err,
            AgentHubError::Graph(GraphError::NodeNotFound(ref id)) if id == "stage-a"
        ));
        assert_eq!(exit_code(err), 255);
    }

    #[test]
    fn exit_code_via_u8_from_matches_method() {
        let err = AgentHubError::Pty("x".into());
        assert_eq!(u8::from(&err), err.exit_code());
    }

    #[test]
    fn all_exit_codes_are_blueprint_defined() {
        let samples = [
            AgentHubError::Io(std::io::Error::other("io")),
            AgentHubError::Pty("p".into()),
            AgentHubError::Database(sqlx::Error::RowNotFound),
            AgentHubError::AgentNotFound(Uuid::nil()),
            AgentHubError::PipelineParse {
                pos: 0,
                msg: "m".into(),
            },
            AgentHubError::PipelineExecution {
                stage: 0,
                msg: "m".into(),
            },
            AgentHubError::Snapshot("s".into()),
            AgentHubError::Revert("r".into()),
            AgentHubError::InductionTimeout(Uuid::nil()),
            AgentHubError::RateLimit(Uuid::nil()),
            AgentHubError::PermissionDenied {
                agent_id: Uuid::nil(),
                permission: "x".into(),
            },
            AgentHubError::Sanitizer("s".into()),
            AgentHubError::Serialization(serde_json::from_str::<i32>("x").expect_err("json")),
            AgentHubError::RoleNotFound("r".into()),
            AgentHubError::Context("c".into()),
            AgentHubError::Config("c".into()),
            AgentHubError::DriverProfile {
                driver: "d".into(),
                msg: "m".into(),
            },
            AgentHubError::Graph(GraphError::CycleDetected),
            AgentHubError::DmAgentLimit {
                existing_id: Uuid::nil(),
                existing_tag: "a".into(),
                new_tag: "b".into(),
            },
            AgentHubError::DmModeTransition { count: 1 },
            AgentHubError::ChannelNotFound("ch".into()),
            AgentHubError::ChannelAlreadyExists("ch".into()),
            AgentHubError::WriteFilesBlocked {
                agent_id: Uuid::nil(),
            },
        ];
        for err in samples {
            assert!(
                MAPPED_CODES.contains(&err.exit_code()),
                "unexpected exit code {} for {err}",
                err.exit_code()
            );
        }
    }

    #[test]
    fn display_messages_include_context() {
        let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").expect("uuid");
        let err = AgentHubError::PermissionDenied {
            agent_id: id,
            permission: "WRITE_FILES".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("550e8400"));
        assert!(msg.contains("WRITE_FILES"));

        let dm = AgentHubError::DmAgentLimit {
            existing_id: id,
            existing_tag: "gemini".into(),
            new_tag: "claude".into(),
        };
        let dm_msg = dm.to_string();
        assert!(dm_msg.contains("gemini"));
        assert!(dm_msg.contains("claude"));

        let blocked = AgentHubError::WriteFilesBlocked { agent_id: id };
        assert!(blocked.to_string().contains("WRITE_FILES"));
    }

    #[test]
    fn graph_error_display() {
        assert_eq!(
            GraphError::CycleDetected.to_string(),
            "Cycle detected in pipeline graph"
        );
        assert_eq!(
            GraphError::NodeNotFound("n1".into()).to_string(),
            "Node not found: n1"
        );
    }
}
