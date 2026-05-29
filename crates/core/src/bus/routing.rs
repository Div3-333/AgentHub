//! Pure message routing (blueprint §7.2). No PTY or server runtime dependencies.
use uuid::Uuid;

use super::event::{MessageTarget, WorkspaceModeRepr};

/// Stagger between broadcast injections when chaos heuristics apply (ms).
pub const STAGGER_STEP_MS: u64 = 150;

/// Display tag for user-originated injections (blueprint §7.2).
pub const USER_SENDER_TAG: &str = "User";

/// Agent status used for routing decisions (mirrors [`crate::pty::PtyStatus`] when `full` is on).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteAgentStatus {
    Initializing,
    Idle,
    Thinking,
    Muted,
    Deafened,
    Suspended,
    Dead,
    RateLimited,
}

impl RouteAgentStatus {
    #[must_use]
    pub const fn is_deliverable(self) -> bool {
        !matches!(self, Self::Dead | Self::Suspended)
    }
}

#[cfg(any(feature = "full", feature = "bus-tests"))]
impl From<crate::pty::PtyStatus> for RouteAgentStatus {
    fn from(status: crate::pty::PtyStatus) -> Self {
        use crate::pty::PtyStatus;
        match status {
            PtyStatus::Initializing => Self::Initializing,
            PtyStatus::Idle => Self::Idle,
            PtyStatus::Thinking => Self::Thinking,
            PtyStatus::Muted => Self::Muted,
            PtyStatus::Deafened => Self::Deafened,
            PtyStatus::Suspended => Self::Suspended,
            PtyStatus::Dead => Self::Dead,
            PtyStatus::RateLimited => Self::RateLimited,
        }
    }
}

/// Snapshot of one agent used for pure routing tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRouteInfo {
    pub id: Uuid,
    pub tag: String,
    pub status: RouteAgentStatus,
    pub receives_broadcast: bool,
}

/// Parses `@tag` mentions from user content; tags are matched without the `@` prefix.
#[must_use]
pub fn parse_mention_tags(content: &str) -> Vec<String> {
    // Manual scan to avoid fallible regex compilation in production paths.
    // Tags: first char must be alphanumeric; subsequent may include '_' or '-'.
    let bytes = content.as_bytes();
    let mut tags = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'@' {
            i += 1;
            continue;
        }
        let start = i + 1;
        if start >= bytes.len() {
            break;
        }
        let first = bytes[start];
        if !first.is_ascii_alphanumeric() {
            i = start;
            continue;
        }
        let mut end = start + 1;
        while end < bytes.len() {
            let c = bytes[end];
            if c.is_ascii_alphanumeric() || c == b'_' || c == b'-' {
                end += 1;
            } else {
                break;
            }
        }
        if let Ok(tag) = std::str::from_utf8(&bytes[start..end]) {
            tags.push(tag.to_string());
        }
        i = end;
    }
    tags
}

/// Resolves explicit or parsed mention targets to agent ids.
#[must_use]
pub fn resolve_mention_target(
    content: &str,
    explicit: &MessageTarget,
    agents: &[AgentRouteInfo],
) -> MessageTarget {
    match explicit {
        MessageTarget::Direct(_) | MessageTarget::Multi(_) => explicit.clone(),
        MessageTarget::Broadcast => {
            let tags = parse_mention_tags(content);
            if tags.is_empty() {
                return MessageTarget::Broadcast;
            }
            let mut ids = Vec::new();
            for tag in tags {
                if let Some(agent) = agents.iter().find(|a| a.tag.eq_ignore_ascii_case(&tag)) {
                    if !ids.contains(&agent.id) {
                        ids.push(agent.id);
                    }
                }
            }
            match ids.len() {
                0 => MessageTarget::Broadcast,
                1 => MessageTarget::Direct(ids[0]),
                _ => MessageTarget::Multi(ids),
            }
        }
    }
}

/// Whether this user delivery bypasses the deafen (`receives_broadcast`) gate.
#[must_use]
pub fn user_delivery_is_direct(target: &MessageTarget) -> bool {
    !matches!(target, MessageTarget::Broadcast)
}

/// Recipients for a routable message (blueprint §7.2).
#[must_use]
pub fn resolve_recipients(
    agents: &[AgentRouteInfo],
    mode: WorkspaceModeRepr,
    sender_id: Option<Uuid>,
    target: &MessageTarget,
    from_user: bool,
) -> Vec<Uuid> {
    let mut recipients: Vec<Uuid> = match target {
        MessageTarget::Direct(id) => {
            if agents
                .iter()
                .any(|a| a.id == *id && a.status.is_deliverable())
            {
                vec![*id]
            } else {
                Vec::new()
            }
        }
        MessageTarget::Multi(ids) => ids
            .iter()
            .filter(|id| {
                agents
                    .iter()
                    .any(|a| a.id == **id && a.status.is_deliverable())
            })
            .copied()
            .collect(),
        MessageTarget::Broadcast => agents
            .iter()
            .filter(|a| {
                a.status.is_deliverable() && a.receives_broadcast && sender_id != Some(a.id)
            })
            .map(|a| a.id)
            .collect(),
    };

    if from_user && user_delivery_is_direct(target) {
        recipients.retain(|id| {
            agents
                .iter()
                .any(|a| a.id == *id && a.status.is_deliverable())
        });
    } else if !from_user {
        recipients.retain(|id| {
            agents.iter().any(|a| {
                a.id == *id
                    && a.status.is_deliverable()
                    && a.receives_broadcast
                    && sender_id != Some(a.id)
            })
        });
    }

    if matches!(mode, WorkspaceModeRepr::Dm) {
        if let Some(only) = agents
            .iter()
            .find(|a| a.status.is_deliverable())
            .map(|a| a.id)
        {
            if matches!(target, MessageTarget::Broadcast) {
                recipients = vec![only];
            } else {
                recipients.retain(|id| *id == only);
            }
        } else {
            recipients.clear();
        }
    }

    recipients.sort_by_key(|id| *id);
    recipients.dedup();
    recipients
}

/// Injection prefix format (blueprint §7.2).
#[must_use]
pub fn format_injection(sender_tag: &str, content: &str) -> String {
    let line_end = if cfg!(windows) { "\r\n" } else { "\n" };
    format!("[{sender_tag} says]: {content}{line_end}")
}

/// True when user content should use LLM Racing instead of normal bus routing.
#[cfg(feature = "full")]
#[must_use]
pub fn should_route_as_racing(content: &str) -> bool {
    super::racing::is_racing_input(content)
}

/// True when staggered injection applies (blueprint §7.2 chaos heuristics).
#[must_use]
pub fn should_stagger(
    mode: WorkspaceModeRepr,
    thinking_count: usize,
    recipient_count: usize,
) -> bool {
    recipient_count > 1 && !matches!(mode, WorkspaceModeRepr::Dm) && thinking_count > 1
}

#[must_use]
pub fn stagger_delay_ms(index: usize) -> u64 {
    STAGGER_STEP_MS.saturating_mul(index as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn sample_id() -> Uuid {
        Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").expect("uuid")
    }

    fn agent(
        id: Uuid,
        tag: &str,
        status: RouteAgentStatus,
        receives_broadcast: bool,
    ) -> AgentRouteInfo {
        AgentRouteInfo {
            id,
            tag: tag.to_string(),
            status,
            receives_broadcast,
        }
    }

    #[test]
    fn parse_mention_tags_extracts_tags() {
        let tags = parse_mention_tags("@gemini-1 hello @claude-2");
        assert_eq!(tags, vec!["gemini-1", "claude-2"]);
    }

    #[test]
    fn resolve_mention_target_from_content() {
        let a = sample_id();
        let b = Uuid::new_v4();
        let agents = vec![
            agent(a, "gemini-1", RouteAgentStatus::Idle, true),
            agent(b, "claude-2", RouteAgentStatus::Idle, true),
        ];
        let target =
            resolve_mention_target("@gemini-1 fix this", &MessageTarget::Broadcast, &agents);
        assert_eq!(target, MessageTarget::Direct(a));
    }

    #[test]
    fn user_message_with_mention_routes_only_to_tagged_agent() {
        let gemini = sample_id();
        let claude = Uuid::new_v4();
        let agents = vec![
            agent(gemini, "gemini-1", RouteAgentStatus::Idle, true),
            agent(claude, "claude-2", RouteAgentStatus::Idle, true),
        ];
        let target =
            resolve_mention_target("@gemini-1 write tests", &MessageTarget::Broadcast, &agents);
        let recipients =
            resolve_recipients(&agents, WorkspaceModeRepr::GroupChat, None, &target, true);
        assert_eq!(recipients, vec![gemini]);
    }

    #[test]
    fn user_message_without_mention_broadcasts_to_non_deafened() {
        let gemini = sample_id();
        let claude = Uuid::new_v4();
        let agents = vec![
            agent(gemini, "gemini-1", RouteAgentStatus::Idle, true),
            agent(claude, "claude-2", RouteAgentStatus::Idle, true),
        ];
        let recipients = resolve_recipients(
            &agents,
            WorkspaceModeRepr::GroupChat,
            None,
            &MessageTarget::Broadcast,
            true,
        );
        let mut expected = vec![claude, gemini];
        expected.sort();
        assert_eq!(recipients, expected);
    }

    #[test]
    fn mention_routing_differs_from_broadcast_in_same_room() {
        let gemini = sample_id();
        let claude = Uuid::new_v4();
        let agents = vec![
            agent(gemini, "gemini-1", RouteAgentStatus::Idle, true),
            agent(claude, "claude-2", RouteAgentStatus::Idle, true),
        ];

        let broadcast = resolve_recipients(
            &agents,
            WorkspaceModeRepr::GroupChat,
            None,
            &MessageTarget::Broadcast,
            true,
        );
        assert_eq!(broadcast.len(), 2);

        let mention_target =
            resolve_mention_target("@claude-2 only you", &MessageTarget::Broadcast, &agents);
        let mention = resolve_recipients(
            &agents,
            WorkspaceModeRepr::GroupChat,
            None,
            &mention_target,
            true,
        );
        assert_eq!(mention, vec![claude]);
        assert_ne!(mention, broadcast);
    }

    #[test]
    fn deafened_agent_skips_broadcast_but_gets_direct_mention() {
        let gemini = sample_id();
        let claude = Uuid::new_v4();
        let agents = vec![
            agent(gemini, "gemini-1", RouteAgentStatus::Idle, false),
            agent(claude, "claude-2", RouteAgentStatus::Idle, true),
        ];
        let broadcast = resolve_recipients(
            &agents,
            WorkspaceModeRepr::GroupChat,
            None,
            &MessageTarget::Broadcast,
            true,
        );
        assert_eq!(broadcast, vec![claude]);

        let mention = resolve_recipients(
            &agents,
            WorkspaceModeRepr::GroupChat,
            None,
            &MessageTarget::Direct(gemini),
            true,
        );
        assert_eq!(mention, vec![gemini]);
    }

    #[test]
    fn agent_message_broadcasts_to_others_not_sender() {
        let gemini = sample_id();
        let claude = Uuid::new_v4();
        let agents = vec![
            agent(gemini, "gemini-1", RouteAgentStatus::Idle, true),
            agent(claude, "claude-2", RouteAgentStatus::Idle, true),
        ];
        let recipients = resolve_recipients(
            &agents,
            WorkspaceModeRepr::GroupChat,
            Some(gemini),
            &MessageTarget::Broadcast,
            false,
        );
        assert_eq!(recipients, vec![claude]);
    }

    #[test]
    fn agent_message_injection_format() {
        let payload = format_injection("gemini-1", "hello world");
        let expected_end = if cfg!(windows) { "\r\n" } else { "\n" };
        assert_eq!(
            payload,
            format!("[gemini-1 says]: hello world{expected_end}")
        );
    }

    #[test]
    fn suspended_agent_excluded_from_broadcast() {
        let active = sample_id();
        let suspended = Uuid::new_v4();
        let agents = vec![
            agent(active, "gemini-1", RouteAgentStatus::Idle, true),
            agent(suspended, "claude-2", RouteAgentStatus::Suspended, true),
        ];
        let recipients = resolve_recipients(
            &agents,
            WorkspaceModeRepr::GroupChat,
            None,
            &MessageTarget::Broadcast,
            true,
        );
        assert_eq!(recipients, vec![active]);
    }

    #[test]
    fn stagger_only_when_multiple_thinking_in_group_mode() {
        assert!(!should_stagger(WorkspaceModeRepr::Dm, 2, 3));
        assert!(!should_stagger(WorkspaceModeRepr::GroupChat, 1, 3));
        assert!(should_stagger(WorkspaceModeRepr::Server, 2, 2));
        assert_eq!(stagger_delay_ms(0), 0);
        assert_eq!(stagger_delay_ms(2), 300);
    }

    #[test]
    fn dm_mode_routes_broadcast_to_single_agent() {
        let only = sample_id();
        let other = Uuid::new_v4();
        let agents = vec![
            agent(only, "gemini-1", RouteAgentStatus::Idle, true),
            agent(other, "claude-2", RouteAgentStatus::Idle, true),
        ];
        let recipients = resolve_recipients(
            &agents,
            WorkspaceModeRepr::Dm,
            None,
            &MessageTarget::Broadcast,
            true,
        );
        assert_eq!(recipients, vec![only]);
    }

    #[test]
    fn unknown_mention_falls_back_to_broadcast() {
        let gemini = sample_id();
        let agents = vec![agent(gemini, "gemini-1", RouteAgentStatus::Idle, true)];
        let target = resolve_mention_target("@nobody hello", &MessageTarget::Broadcast, &agents);
        assert_eq!(target, MessageTarget::Broadcast);
        let recipients =
            resolve_recipients(&agents, WorkspaceModeRepr::GroupChat, None, &target, true);
        assert_eq!(recipients, vec![gemini]);
    }
}
