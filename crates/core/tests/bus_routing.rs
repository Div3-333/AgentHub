//! Integration-style routing tests for blueprint §7.2 (Phase 4 DoD).

use agenthub_core::bus::{
    format_injection, parse_mention_tags, resolve_mention_target, resolve_recipients,
    AgentRouteInfo, MessageTarget, WorkspaceModeRepr,
};
use agenthub_core::pty::PtyStatus;
use uuid::Uuid;

fn agent(id: Uuid, tag: &str, status: PtyStatus, receives_broadcast: bool) -> AgentRouteInfo {
    AgentRouteInfo {
        id,
        tag: tag.to_string(),
        status: status.into(),
        receives_broadcast,
    }
}

#[test]
fn bus_mention_in_content_routes_to_single_agent() {
    let gemini = Uuid::new_v4();
    let claude = Uuid::new_v4();
    let agents = vec![
        agent(gemini, "gemini-1", PtyStatus::Idle, true),
        agent(claude, "claude-2", PtyStatus::Idle, true),
    ];

    let target = resolve_mention_target(
        "@gemini-1 please review",
        &MessageTarget::Broadcast,
        &agents,
    );
    let recipients = resolve_recipients(&agents, WorkspaceModeRepr::GroupChat, None, &target, true);

    assert_eq!(recipients, vec![gemini]);
}

#[test]
fn bus_no_mention_broadcasts_to_all_non_deafened() {
    let gemini = Uuid::new_v4();
    let claude = Uuid::new_v4();
    let agents = vec![
        agent(gemini, "gemini-1", PtyStatus::Idle, true),
        agent(claude, "claude-2", PtyStatus::Idle, true),
    ];

    let recipients = resolve_recipients(
        &agents,
        WorkspaceModeRepr::GroupChat,
        None,
        &MessageTarget::Broadcast,
        true,
    );

    assert_eq!(recipients.len(), 2);
    assert!(recipients.contains(&gemini));
    assert!(recipients.contains(&claude));
}

#[test]
fn bus_agent_broadcast_excludes_sender_and_uses_prefix_format() {
    let gemini = Uuid::new_v4();
    let claude = Uuid::new_v4();
    let agents = vec![
        agent(gemini, "gemini-1", PtyStatus::Idle, true),
        agent(claude, "claude-2", PtyStatus::Idle, true),
    ];

    let recipients = resolve_recipients(
        &agents,
        WorkspaceModeRepr::GroupChat,
        Some(gemini),
        &MessageTarget::Broadcast,
        false,
    );

    assert_eq!(recipients, vec![claude]);
    let expected_end = if cfg!(windows) { "\r\n" } else { "\n" };
    assert_eq!(
        format_injection("gemini-1", "done"),
        format!("[gemini-1 says]: done{expected_end}")
    );
}

#[test]
fn bus_parse_mention_tags_finds_multiple() {
    assert_eq!(
        parse_mention_tags("@a-1 x @b-2"),
        vec!["a-1".to_string(), "b-2".to_string()]
    );
}
