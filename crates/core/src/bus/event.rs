//! Bus event types (blueprint §7.1).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Every event that flows through the AgentHub system.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BusEvent {
    // ── Agent Lifecycle ──────────────────────────────────────────────────
    /// An agent has completed induction and is now online.
    AgentOnline {
        id: Uuid,
        tag: String,
        role: String,
    },
    /// An agent's process has died (crash, kick, or natural exit).
    AgentOffline {
        id: Uuid,
        tag: String,
        reason: OfflineReason,
    },
    /// An agent's status has changed (e.g., Idle → Thinking).
    AgentStatusChanged {
        id: Uuid,
        old: u8,
        new: u8,
    },
    /// A subagent was detected and registered.
    SubagentDetected {
        parent_id: Uuid,
        child_id: Uuid,
        child_tag: String,
    },

    // ── Messages ─────────────────────────────────────────────────────────
    /// A message from a human user.
    UserMessage {
        content: String,
        timestamp: DateTime<Utc>,
        target: MessageTarget,
    },
    /// A sanitized, complete message from an agent after turn detection.
    AgentMessage {
        id: Uuid,
        tag: String,
        content: String,
        timestamp: DateTime<Utc>,
        /// Set when this turn belongs to an active LLM Racing session.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        race_session_id: Option<Uuid>,
    },
    /// A system notification displayed in the chat (italicized, grey).
    SystemMessage {
        content: String,
        timestamp: DateTime<Utc>,
    },

    // ── Moderation ───────────────────────────────────────────────────────
    AgentMuted {
        id: Uuid,
        by: String,
    },
    AgentUnmuted {
        id: Uuid,
        by: String,
    },
    AgentDeafened {
        id: Uuid,
        by: String,
    },
    AgentUndeafened {
        id: Uuid,
        by: String,
    },
    AgentTimedOut {
        id: Uuid,
        duration_secs: u64,
        by: String,
    },
    AgentKicked {
        id: Uuid,
        reason: Option<String>,
        by: String,
    },
    AgentBanned {
        id: Uuid,
        driver_name: String,
        by: String,
    },
    RoleAssigned {
        agent_id: Uuid,
        role: String,
        by: String,
    },

    // ── Pipelines ────────────────────────────────────────────────────────
    PipelineStarted {
        pipeline_id: Uuid,
        definition: String,
    },
    PipelineStageComplete {
        pipeline_id: Uuid,
        stage: usize,
        output_preview: String,
    },
    PipelineFailed {
        pipeline_id: Uuid,
        stage: usize,
        error: String,
    },
    PipelineComplete {
        pipeline_id: Uuid,
    },

    // ── VFS / Time-Travel ────────────────────────────────────────────────
    SnapshotCreated {
        snapshot_id: Uuid,
        file_count: usize,
    },
    RevertInitiated {
        snapshot_id: Uuid,
    },
    RevertComplete {
        snapshot_id: Uuid,
    },

    // ── LLM Racing ───────────────────────────────────────────────────────
    /// Multi-@ race started; TUI splits into side-by-side columns.
    RacingStarted {
        session_id: Uuid,
        tags: Vec<String>,
        prompt: String,
        timestamp: DateTime<Utc>,
    },
    /// Streamed output chunk for one racing contestant.
    RacingOutput {
        session_id: Uuid,
        tag: String,
        chunk: String,
        timestamp: DateTime<Utc>,
    },
    /// One contestant finished its turn.
    RacingAgentComplete {
        session_id: Uuid,
        tag: String,
        elapsed_ms: u64,
        timestamp: DateTime<Utc>,
    },
    /// All contestants have completed.
    RacingComplete {
        session_id: Uuid,
        timestamp: DateTime<Utc>,
    },
    /// Race dismissed without selecting a winner.
    RacingCancelled {
        session_id: Uuid,
        timestamp: DateTime<Utc>,
    },

    // ── System ───────────────────────────────────────────────────────────
    ModeChanged {
        old: WorkspaceModeRepr,
        new: WorkspaceModeRepr,
    },
    RateLimitDetected {
        id: Uuid,
        tag: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MessageTarget {
    /// Message goes to all non-deafened agents.
    Broadcast,
    /// Message goes to a specific agent only.
    Direct(Uuid),
    /// Message goes to multiple specific agents.
    Multi(Vec<Uuid>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OfflineReason {
    Crashed,
    Kicked,
    Banned,
    /// Process exited on its own (e.g., user typed "exit").
    Natural,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorkspaceModeRepr {
    Dm,
    GroupChat,
    Server,
}

/// `ServerState.mode` encoding (must match workspace mode setup).
pub const MODE_DM: u8 = 0;
pub const MODE_GROUP_CHAT: u8 = 1;
pub const MODE_SERVER: u8 = 2;

impl WorkspaceModeRepr {
    #[must_use]
    pub const fn from_atomic(value: u8) -> Self {
        match value {
            MODE_DM => Self::Dm,
            MODE_SERVER => Self::Server,
            _ => Self::GroupChat,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde::de::DeserializeOwned;

    fn roundtrip<T>(value: &T)
    where
        T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_string(value).expect("serialize");
        let back: T = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(*value, back);
    }

    fn sample_ts() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 29, 12, 0, 0)
            .single()
            .expect("valid timestamp")
    }

    fn sample_id() -> Uuid {
        Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").expect("uuid")
    }

    #[test]
    fn offline_reason_roundtrip() {
        for reason in [
            OfflineReason::Crashed,
            OfflineReason::Kicked,
            OfflineReason::Banned,
            OfflineReason::Natural,
        ] {
            roundtrip(&reason);
        }
    }

    #[test]
    fn message_target_roundtrip() {
        let id = sample_id();
        for target in [
            MessageTarget::Broadcast,
            MessageTarget::Direct(id),
            MessageTarget::Multi(vec![id, Uuid::new_v4()]),
        ] {
            roundtrip(&target);
        }
    }

    #[test]
    fn workspace_mode_repr_roundtrip() {
        for mode in [
            WorkspaceModeRepr::Dm,
            WorkspaceModeRepr::GroupChat,
            WorkspaceModeRepr::Server,
        ] {
            roundtrip(&mode);
        }
    }

    #[test]
    fn bus_event_lifecycle_roundtrip() {
        let id = sample_id();
        roundtrip(&BusEvent::AgentOnline {
            id,
            tag: "gemini".into(),
            role: "default".into(),
        });
        roundtrip(&BusEvent::AgentOffline {
            id,
            tag: "gemini".into(),
            reason: OfflineReason::Natural,
        });
        roundtrip(&BusEvent::AgentStatusChanged { id, old: 1, new: 2 });
        roundtrip(&BusEvent::SubagentDetected {
            parent_id: id,
            child_id: Uuid::new_v4(),
            child_tag: "sub".into(),
        });
    }

    #[test]
    fn bus_event_messages_roundtrip() {
        let id = sample_id();
        let ts = sample_ts();
        roundtrip(&BusEvent::UserMessage {
            content: "hello".into(),
            timestamp: ts,
            target: MessageTarget::Broadcast,
        });
        roundtrip(&BusEvent::AgentMessage {
            id,
            tag: "claude".into(),
            content: "response".into(),
            timestamp: ts,
            race_session_id: None,
        });
        roundtrip(&BusEvent::AgentMessage {
            id,
            tag: "claude".into(),
            content: "racing turn".into(),
            timestamp: ts,
            race_session_id: Some(Uuid::new_v4()),
        });
        roundtrip(&BusEvent::RacingStarted {
            session_id: id,
            tags: vec!["gemini".into(), "claude".into()],
            prompt: "write code".into(),
            timestamp: ts,
        });
        roundtrip(&BusEvent::RacingOutput {
            session_id: id,
            tag: "gemini".into(),
            chunk: "fn main() {}".into(),
            timestamp: ts,
        });
        roundtrip(&BusEvent::RacingAgentComplete {
            session_id: id,
            tag: "gemini".into(),
            elapsed_ms: 1200,
            timestamp: ts,
        });
        roundtrip(&BusEvent::RacingComplete {
            session_id: id,
            timestamp: ts,
        });
        roundtrip(&BusEvent::RacingCancelled {
            session_id: id,
            timestamp: ts,
        });
        roundtrip(&BusEvent::SystemMessage {
            content: "notice".into(),
            timestamp: ts,
        });
    }

    #[test]
    fn bus_event_moderation_roundtrip() {
        let id = sample_id();
        roundtrip(&BusEvent::AgentMuted {
            id,
            by: "admin".into(),
        });
        roundtrip(&BusEvent::AgentUnmuted {
            id,
            by: "admin".into(),
        });
        roundtrip(&BusEvent::AgentDeafened {
            id,
            by: "admin".into(),
        });
        roundtrip(&BusEvent::AgentUndeafened {
            id,
            by: "admin".into(),
        });
        roundtrip(&BusEvent::AgentTimedOut {
            id,
            duration_secs: 300,
            by: "admin".into(),
        });
        roundtrip(&BusEvent::AgentKicked {
            id,
            reason: Some("spam".into()),
            by: "admin".into(),
        });
        roundtrip(&BusEvent::AgentBanned {
            id,
            driver_name: "gemini".into(),
            by: "admin".into(),
        });
        roundtrip(&BusEvent::RoleAssigned {
            agent_id: id,
            role: "moderator".into(),
            by: "admin".into(),
        });
    }

    #[test]
    fn bus_event_pipeline_roundtrip() {
        let pipeline_id = sample_id();
        roundtrip(&BusEvent::PipelineStarted {
            pipeline_id,
            definition: "a | b".into(),
        });
        roundtrip(&BusEvent::PipelineStageComplete {
            pipeline_id,
            stage: 1,
            output_preview: "preview".into(),
        });
        roundtrip(&BusEvent::PipelineFailed {
            pipeline_id,
            stage: 0,
            error: "timeout".into(),
        });
        roundtrip(&BusEvent::PipelineComplete { pipeline_id });
    }

    #[test]
    fn bus_event_vfs_roundtrip() {
        let snapshot_id = sample_id();
        roundtrip(&BusEvent::SnapshotCreated {
            snapshot_id,
            file_count: 42,
        });
        roundtrip(&BusEvent::RevertInitiated { snapshot_id });
        roundtrip(&BusEvent::RevertComplete { snapshot_id });
    }

    #[test]
    fn agent_message_deserializes_without_race_session_field() {
        let id = sample_id();
        let ts = sample_ts();
        let json = serde_json::json!({
            "AgentMessage": {
                "id": id,
                "tag": "legacy",
                "content": "body",
                "timestamp": ts
            }
        });
        let event: BusEvent = serde_json::from_value(json).expect("deserialize");
        assert_eq!(
            event,
            BusEvent::AgentMessage {
                id,
                tag: "legacy".into(),
                content: "body".into(),
                timestamp: ts,
                race_session_id: None,
            }
        );
    }

    #[test]
    fn bus_event_system_roundtrip() {
        let id = sample_id();
        roundtrip(&BusEvent::ModeChanged {
            old: WorkspaceModeRepr::Dm,
            new: WorkspaceModeRepr::Server,
        });
        roundtrip(&BusEvent::RateLimitDetected {
            id,
            tag: "codex".into(),
        });
    }
}
