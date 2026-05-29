//! Workspace mode logic: DM, Group Chat, and Server (blueprint §9.1–9.3).

use std::sync::atomic::Ordering;

use uuid::Uuid;

use crate::bus::WorkspaceModeRepr;
use crate::config::WorkspaceMode;
use crate::error::{AgentHubError, Result};
use crate::server::rbac::Permissions;
use crate::server::state::ServerState;

#[cfg(feature = "full")]
use crate::pty::AgentPty;

/// Atomic encoding for [`WorkspaceMode::DirectMessage`].
pub const MODE_DIRECT_MESSAGE: u8 = 0;
/// Atomic encoding for [`WorkspaceMode::GroupChat`].
pub const MODE_GROUP_CHAT: u8 = 1;
/// Atomic encoding for [`WorkspaceMode::Server`].
pub const MODE_SERVER: u8 = 2;

/// Compact mode id stored in [`ServerState::mode`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceModeId {
    Dm = 0,
    GroupChat = 1,
    Server = 2,
}

impl WorkspaceModeId {
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            MODE_DIRECT_MESSAGE => Some(Self::Dm),
            MODE_GROUP_CHAT => Some(Self::GroupChat),
            MODE_SERVER => Some(Self::Server),
            _ => None,
        }
    }

    #[must_use]
    pub fn to_repr(self) -> WorkspaceModeRepr {
        match self {
            Self::Dm => WorkspaceModeRepr::Dm,
            Self::GroupChat => WorkspaceModeRepr::GroupChat,
            Self::Server => WorkspaceModeRepr::Server,
        }
    }

    #[must_use]
    pub fn from_config(mode: WorkspaceMode) -> Self {
        match mode {
            WorkspaceMode::DirectMessage => Self::Dm,
            WorkspaceMode::GroupChat => Self::GroupChat,
            WorkspaceMode::Server => Self::Server,
        }
    }

    #[must_use]
    pub fn to_config(self) -> WorkspaceMode {
        match self {
            Self::Dm => WorkspaceMode::DirectMessage,
            Self::GroupChat => WorkspaceMode::GroupChat,
            Self::Server => WorkspaceMode::Server,
        }
    }
}

/// Default permissive bitmask for agents in group chat (§9.2).
pub const GROUP_CHAT_DEFAULT_PERMISSIONS: Permissions = Permissions::from_bits_truncate(
    Permissions::VIEW_CHANNEL.bits()
        | Permissions::SEND_MESSAGES.bits()
        | Permissions::BROADCAST_OUTPUT.bits()
        | Permissions::RECEIVE_BROADCAST.bits()
        | Permissions::EXECUTE_UNIX.bits()
        | Permissions::WRITE_FILES.bits()
        | Permissions::TRIGGER_PIPELINE.bits(),
);

/// Implicit full permissions for the lone agent in DM mode (§9.1).
pub const DM_FULL_PERMISSIONS: Permissions = Permissions::from_bits_truncate(
    Permissions::VIEW_CHANNEL.bits()
        | Permissions::SEND_MESSAGES.bits()
        | Permissions::BROADCAST_OUTPUT.bits()
        | Permissions::RECEIVE_BROADCAST.bits()
        | Permissions::EXECUTE_UNIX.bits()
        | Permissions::WRITE_FILES.bits()
        | Permissions::TRIGGER_PIPELINE.bits()
        | Permissions::OVERRIDE_OTHERS.bits()
        | Permissions::MODIFY_ROLES.bits()
        | Permissions::SPAWN_AGENTS.bits(),
);

/// Result of a successful [`set_mode`] / [`cycle_mode`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeChange {
    pub old: WorkspaceModeId,
    pub new: WorkspaceModeId,
}

/// Conflict when spawning a second agent in DM mode (§9.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmSpawnConflict {
    pub existing_id: Uuid,
    pub existing_tag: String,
    pub new_tag: String,
}

#[must_use]
pub const fn mode_to_u8(mode: WorkspaceModeId) -> u8 {
    mode.as_u8()
}

#[must_use]
pub fn get_mode(state: &ServerState) -> WorkspaceModeId {
    WorkspaceModeId::from_u8(state.mode.load(Ordering::Acquire))
        .unwrap_or(WorkspaceModeId::GroupChat)
}

#[must_use]
pub fn mode_repr(mode: WorkspaceModeId) -> WorkspaceModeRepr {
    mode.to_repr()
}

/// Whether agent-to-agent broadcast is enabled in this mode (§9.1, §9.2).
#[must_use]
pub const fn broadcast_enabled(mode: WorkspaceModeId) -> bool {
    !matches!(mode, WorkspaceModeId::Dm)
}

/// Whether RBAC checks are strictly enforced (§9.3).
#[must_use]
pub const fn rbac_enforced(mode: WorkspaceModeId) -> bool {
    matches!(mode, WorkspaceModeId::Server)
}

/// Whether staggered broadcast injection applies (§7.2).
#[must_use]
pub const fn chaos_heuristics_enabled(mode: WorkspaceModeId) -> bool {
    matches!(mode, WorkspaceModeId::GroupChat | WorkspaceModeId::Server)
}

/// Whether admin slash commands (`/ban`, `/addrole`, …) are available (§9.3).
#[must_use]
pub const fn admin_commands_enabled(mode: WorkspaceModeId) -> bool {
    matches!(mode, WorkspaceModeId::Server)
}

/// Whether induction omits multi-agent context (§9.1).
#[must_use]
pub const fn simplified_induction(mode: WorkspaceModeId) -> bool {
    matches!(mode, WorkspaceModeId::Dm)
}

/// Maximum concurrent agents allowed in the current mode.
#[must_use]
pub const fn max_agents_for_mode(mode: WorkspaceModeId, config_max: u8) -> usize {
    match mode {
        WorkspaceModeId::Dm => 1,
        WorkspaceModeId::GroupChat | WorkspaceModeId::Server => config_max as usize,
    }
}

#[must_use]
pub fn active_agent_count(state: &ServerState) -> usize {
    #[cfg(feature = "full")]
    {
        let n = state.agents.len();
        if n > 0 {
            return n;
        }
    }
    state.agent_states.len()
}

/// Switch workspace mode, updating [`ServerState::mode`] and PTY broadcast flags.
pub fn set_mode(state: &ServerState, new: WorkspaceModeId) -> Result<ModeChange> {
    let old = get_mode(state);
    if old == new {
        return Ok(ModeChange { old, new });
    }

    if new == WorkspaceModeId::Dm && active_agent_count(state) > 1 {
        return Err(AgentHubError::DmModeTransition {
            count: active_agent_count(state),
        });
    }

    if old == WorkspaceModeId::Server && new != WorkspaceModeId::Server {
        state.channels.clear();
    }

    state.mode.store(mode_to_u8(new), Ordering::Release);
    #[cfg(feature = "full")]
    apply_mode_to_agents(state, new);

    Ok(ModeChange { old, new })
}

/// Cycle DM → GroupChat → Server → DM.
pub fn cycle_mode(state: &ServerState) -> Result<ModeChange> {
    let next = match get_mode(state) {
        WorkspaceModeId::Dm => WorkspaceModeId::GroupChat,
        WorkspaceModeId::GroupChat => WorkspaceModeId::Server,
        WorkspaceModeId::Server => WorkspaceModeId::Dm,
    };
    set_mode(state, next)
}

/// Validate that another agent may be spawned under the current mode.
pub fn validate_spawn(state: &ServerState, max_agents: u8, new_tag: &str) -> Result<()> {
    validate_spawn_for_mode(
        get_mode(state),
        active_agent_count(state),
        max_agents,
        first_active_agent(state),
        new_tag,
    )
}

/// Core spawn validation (testable without live PTYs).
pub fn validate_spawn_for_mode(
    mode: WorkspaceModeId,
    active_count: usize,
    max_agents: u8,
    existing: Option<(Uuid, String)>,
    new_tag: &str,
) -> Result<()> {
    if mode == WorkspaceModeId::Dm {
        if active_count >= 1 {
            let (existing_id, existing_tag) = existing.ok_or_else(|| {
                AgentHubError::Config("DM spawn validation requires existing agent metadata".into())
            })?;
            return Err(AgentHubError::DmAgentLimit {
                existing_id,
                existing_tag,
                new_tag: new_tag.to_string(),
            });
        }
        return Ok(());
    }

    let limit = max_agents_for_mode(mode, max_agents);
    if active_count >= limit {
        return Err(AgentHubError::Config(format!(
            "maximum of {limit} agents already active"
        )));
    }
    Ok(())
}

/// If DM mode already has an agent, returns details for the replacement prompt (§9.1).
#[must_use]
pub fn dm_spawn_conflict(state: &ServerState, new_tag: &str) -> Option<DmSpawnConflict> {
    if get_mode(state) != WorkspaceModeId::Dm {
        return None;
    }
    let (existing_id, existing_tag) = first_active_agent(state)?;
    Some(DmSpawnConflict {
        existing_id,
        existing_tag,
        new_tag: new_tag.to_string(),
    })
}

/// Whether named channels are active (§9.3).
#[must_use]
pub const fn channels_enabled(mode: WorkspaceModeId) -> bool {
    matches!(mode, WorkspaceModeId::Server)
}

/// Enforce RBAC for an action. Strict only in Server mode (§9.3).
pub fn check_permission(state: &ServerState, agent_id: Uuid, required: Permissions) -> Result<()> {
    if !rbac_enforced(get_mode(state)) {
        return Ok(());
    }

    let agent = state
        .agent_states
        .get(&agent_id)
        .ok_or(AgentHubError::AgentNotFound(agent_id))?;
    agent.require_permission(required)
}

/// Whether an agent may emit output onto the bus (§9.3).
pub fn enforce_send_messages(state: &ServerState, agent_id: Uuid) -> Result<()> {
    check_permission(state, agent_id, Permissions::SEND_MESSAGES)
}

/// Whether an agent output may be broadcast to peers (§9.3).
pub fn enforce_broadcast_output(state: &ServerState, agent_id: Uuid) -> Result<()> {
    check_permission(state, agent_id, Permissions::BROADCAST_OUTPUT)
}

/// Whether an agent may run Unix pipeline stages (§9.3).
pub fn enforce_execute_unix(state: &ServerState, agent_id: Uuid) -> Result<()> {
    check_permission(state, agent_id, Permissions::EXECUTE_UNIX)
}

/// Whether an agent may trigger pipeline handoffs to other agents (§9.3).
pub fn enforce_trigger_pipeline(state: &ServerState, agent_id: Uuid) -> Result<()> {
    check_permission(state, agent_id, Permissions::TRIGGER_PIPELINE)
}

/// Whether agent output may be published onto the bus (§9.3).
pub fn gate_agent_bus_output(state: &ServerState, agent_id: Uuid, content: &str) -> Result<()> {
    enforce_send_messages(state, agent_id)?;
    enforce_write_files(state, agent_id, content)?;
    Ok(())
}

/// Whether an agent's output may use broadcast routing (§9.3 `BROADCAST_OUTPUT`).
#[must_use]
pub fn agent_may_broadcast(state: &ServerState, sender_id: Uuid) -> bool {
    !rbac_enforced(get_mode(state)) || enforce_broadcast_output(state, sender_id).is_ok()
}

/// Best-effort detection of shell/tool filesystem writes in agent text (§9.3).
#[must_use]
pub fn detect_filesystem_write_attempt(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    const PATTERNS: &[&str] = &[
        " > ",
        " >> ",
        "cat >",
        "echo >",
        "tee ",
        "touch ",
        "cp ",
        "mv ",
        "install ",
        "write_file",
        "writefile",
    ];
    PATTERNS.iter().any(|p| lower.contains(p))
}

/// Blocks agent output when WRITE_FILES is missing in Server mode (§9.3).
pub fn enforce_write_files(state: &ServerState, agent_id: Uuid, content: &str) -> Result<()> {
    if !rbac_enforced(get_mode(state)) || !detect_filesystem_write_attempt(content) {
        return Ok(());
    }
    check_permission(state, agent_id, Permissions::WRITE_FILES)
        .map_err(|_| AgentHubError::WriteFilesBlocked { agent_id })
}

/// Normalizes a channel name (strips `#`, lowercases).
#[must_use]
pub fn normalize_channel_name(raw: &str) -> String {
    raw.trim().trim_start_matches('#').to_ascii_lowercase()
}

/// Parses the first `#channel` tag from message content.
#[must_use]
pub fn parse_channel_tag(content: &str) -> Option<String> {
    let bytes = content.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'#' {
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
        if let Ok(name) = std::str::from_utf8(&bytes[start..end]) {
            return Some(normalize_channel_name(name));
        }
        i = end;
    }
    None
}

/// Creates a logical channel (Server mode only).
pub fn create_channel(state: &ServerState, name: &str) -> Result<()> {
    if !channels_enabled(get_mode(state)) {
        return Err(AgentHubError::Config(
            "channels are only available in Server mode".into(),
        ));
    }
    let key = normalize_channel_name(name);
    if key.is_empty() {
        return Err(AgentHubError::Config("channel name cannot be empty".into()));
    }
    if state.channels.contains_key(&key) {
        return Err(AgentHubError::ChannelAlreadyExists(key));
    }
    state.channels.insert(key, Vec::new());
    Ok(())
}

/// Removes a channel and its membership list.
pub fn delete_channel(state: &ServerState, name: &str) -> Result<()> {
    if !channels_enabled(get_mode(state)) {
        return Err(AgentHubError::Config(
            "channels are only available in Server mode".into(),
        ));
    }
    let key = normalize_channel_name(name);
    state
        .channels
        .remove(&key)
        .ok_or(AgentHubError::ChannelNotFound(key))?;
    Ok(())
}

/// Adds an agent to a channel membership list.
pub fn assign_agent_to_channel(state: &ServerState, channel: &str, agent_id: Uuid) -> Result<()> {
    if !channels_enabled(get_mode(state)) {
        return Err(AgentHubError::Config(
            "channels are only available in Server mode".into(),
        ));
    }
    if !state.agent_states.contains_key(&agent_id) {
        return Err(AgentHubError::AgentNotFound(agent_id));
    }
    let key = normalize_channel_name(channel);
    let mut entry = state
        .channels
        .get_mut(&key)
        .ok_or_else(|| AgentHubError::ChannelNotFound(key.clone()))?;
    if !entry.contains(&agent_id) {
        entry.push(agent_id);
    }
    Ok(())
}

/// Removes an agent from a channel.
pub fn remove_agent_from_channel(state: &ServerState, channel: &str, agent_id: Uuid) -> Result<()> {
    if !channels_enabled(get_mode(state)) {
        return Err(AgentHubError::Config(
            "channels are only available in Server mode".into(),
        ));
    }
    let key = normalize_channel_name(channel);
    let mut entry = state
        .channels
        .get_mut(&key)
        .ok_or_else(|| AgentHubError::ChannelNotFound(key.clone()))?;
    let before = entry.len();
    entry.retain(|id| *id != agent_id);
    if entry.len() == before {
        return Err(AgentHubError::AgentNotFound(agent_id));
    }
    Ok(())
}

/// Member ids for a channel (empty if channel does not exist).
#[must_use]
pub fn channel_members(state: &ServerState, channel: &str) -> Vec<Uuid> {
    let key = normalize_channel_name(channel);
    state
        .channels
        .get(&key)
        .map(|entry| entry.clone())
        .unwrap_or_default()
}

/// Restricts broadcast recipients to a `#channel` tag in Server mode (§9.3).
#[must_use]
pub fn filter_recipients_by_channel(
    state: &ServerState,
    content: &str,
    mut recipients: Vec<Uuid>,
) -> Vec<Uuid> {
    if !channels_enabled(get_mode(state)) {
        return recipients;
    }
    let Some(channel) = parse_channel_tag(content) else {
        return recipients;
    };
    let members = channel_members(state, &channel);
    if members.is_empty() {
        recipients.clear();
        return recipients;
    }
    let member_set: std::collections::HashSet<Uuid> = members.into_iter().collect();
    recipients.retain(|id| member_set.contains(id));
    recipients
}

/// Effective permissions for an agent under the current workspace mode.
pub fn effective_permissions(state: &ServerState, agent_id: Uuid) -> Result<Permissions> {
    let mode = get_mode(state);
    match mode {
        WorkspaceModeId::Dm => Ok(DM_FULL_PERMISSIONS),
        WorkspaceModeId::GroupChat => {
            let agent = state
                .agent_states
                .get(&agent_id)
                .ok_or(AgentHubError::AgentNotFound(agent_id))?;
            Ok(agent.permissions | GROUP_CHAT_DEFAULT_PERMISSIONS)
        }
        WorkspaceModeId::Server => {
            let agent = state
                .agent_states
                .get(&agent_id)
                .ok_or(AgentHubError::AgentNotFound(agent_id))?;
            Ok(agent.permissions)
        }
    }
}

/// Sync PTY-side flags from RBAC + workspace mode after role or mode changes.
#[cfg(feature = "full")]
pub fn sync_agent_pty(state: &ServerState, agent_id: Uuid) -> Result<()> {
    let Some(pty) = state.agents.get(&agent_id) else {
        return Ok(());
    };
    let perms = effective_permissions(state, agent_id)?;
    apply_permissions_to_pty(pty.value(), get_mode(state), perms);
    Ok(())
}

#[cfg(feature = "full")]
fn apply_permissions_to_pty(pty: &AgentPty, mode: WorkspaceModeId, perms: Permissions) {
    pty.set_permissions(perms);

    let receives = match mode {
        WorkspaceModeId::Dm => false,
        WorkspaceModeId::GroupChat => true,
        WorkspaceModeId::Server => perms.contains(Permissions::RECEIVE_BROADCAST),
    };
    pty.receives_broadcast.store(receives, Ordering::Release);
}

#[cfg(feature = "full")]
fn apply_mode_to_agents(state: &ServerState, mode: WorkspaceModeId) {
    for entry in state.agents.iter() {
        let id = *entry.key();
        if let Ok(perms) = effective_permissions(state, id) {
            apply_permissions_to_pty(entry.value(), mode, perms);
        }
    }
}

fn first_active_agent(state: &ServerState) -> Option<(Uuid, String)> {
    #[cfg(feature = "full")]
    {
        if let Some(first) = state.agents.iter().next() {
            return Some((*first.key(), first.value().tag.clone()));
        }
    }
    state
        .agent_states
        .iter()
        .next()
        .map(|entry| (*entry.key(), entry.value().tag.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::server::rbac::{default_roles, permission_label, AgentState};

    fn server_with_mode(mode: WorkspaceModeId) -> ServerState {
        let state = ServerState::new();
        state.mode.store(mode_to_u8(mode), Ordering::Release);
        state
    }

    fn register_agent_state(state: &ServerState, role: &str) -> Uuid {
        let id = Uuid::new_v4();
        let perms = default_roles()[role];
        state.agent_states.insert(
            id,
            Arc::new(AgentState::new(
                id,
                format!("{role}-1"),
                "mock".into(),
                role.into(),
                perms,
                1,
            )),
        );
        id
    }

    fn register_n_agents(state: &ServerState, n: usize) {
        for i in 0..n {
            let id = Uuid::new_v4();
            state.agent_states.insert(
                id,
                Arc::new(AgentState::new(
                    id,
                    format!("agent-{i}"),
                    "mock".into(),
                    "Builder".into(),
                    default_roles()["Builder"],
                    1,
                )),
            );
        }
    }

    #[test]
    fn mode_atomic_roundtrip() {
        for mode in [
            WorkspaceModeId::Dm,
            WorkspaceModeId::GroupChat,
            WorkspaceModeId::Server,
        ] {
            assert_eq!(WorkspaceModeId::from_u8(mode.as_u8()), Some(mode));
        }
        assert_eq!(WorkspaceModeId::from_u8(99), None);
    }

    #[test]
    fn get_mode_reads_server_state_atomic() {
        let state = server_with_mode(WorkspaceModeId::Server);
        assert_eq!(get_mode(&state), WorkspaceModeId::Server);
    }

    #[test]
    fn dm_mode_rules() {
        assert!(!broadcast_enabled(WorkspaceModeId::Dm));
        assert!(!rbac_enforced(WorkspaceModeId::Dm));
        assert!(!chaos_heuristics_enabled(WorkspaceModeId::Dm));
        assert!(simplified_induction(WorkspaceModeId::Dm));
        assert_eq!(max_agents_for_mode(WorkspaceModeId::Dm, 16), 1);
    }

    #[test]
    fn group_chat_mode_rules() {
        assert!(broadcast_enabled(WorkspaceModeId::GroupChat));
        assert!(!rbac_enforced(WorkspaceModeId::GroupChat));
        assert!(chaos_heuristics_enabled(WorkspaceModeId::GroupChat));
        assert!(!simplified_induction(WorkspaceModeId::GroupChat));
        assert!(!admin_commands_enabled(WorkspaceModeId::GroupChat));
    }

    #[test]
    fn server_mode_rules() {
        assert!(broadcast_enabled(WorkspaceModeId::Server));
        assert!(rbac_enforced(WorkspaceModeId::Server));
        assert!(chaos_heuristics_enabled(WorkspaceModeId::Server));
        assert!(!simplified_induction(WorkspaceModeId::Server));
        assert!(admin_commands_enabled(WorkspaceModeId::Server));
    }

    #[test]
    fn transition_dm_to_groupchat() {
        let state = server_with_mode(WorkspaceModeId::Dm);
        let change = set_mode(&state, WorkspaceModeId::GroupChat).expect("transition");
        assert_eq!(change.old, WorkspaceModeId::Dm);
        assert_eq!(change.new, WorkspaceModeId::GroupChat);
        assert_eq!(get_mode(&state), WorkspaceModeId::GroupChat);
        assert!(broadcast_enabled(get_mode(&state)));
    }

    #[test]
    fn transition_groupchat_to_server() {
        let state = server_with_mode(WorkspaceModeId::GroupChat);
        let change = set_mode(&state, WorkspaceModeId::Server).expect("transition");
        assert_eq!(change.old, WorkspaceModeId::GroupChat);
        assert_eq!(change.new, WorkspaceModeId::Server);
        assert!(rbac_enforced(get_mode(&state)));
    }

    #[test]
    fn transition_server_to_groupchat() {
        let state = server_with_mode(WorkspaceModeId::Server);
        let change = set_mode(&state, WorkspaceModeId::GroupChat).expect("transition");
        assert_eq!(change.old, WorkspaceModeId::Server);
        assert_eq!(change.new, WorkspaceModeId::GroupChat);
        assert!(!rbac_enforced(get_mode(&state)));
    }

    #[test]
    fn transition_groupchat_to_dm_with_no_agents() {
        let state = server_with_mode(WorkspaceModeId::GroupChat);
        set_mode(&state, WorkspaceModeId::Dm).expect("transition");
        assert_eq!(get_mode(&state), WorkspaceModeId::Dm);
    }

    #[test]
    fn transition_groupchat_to_dm_fails_with_two_agents() {
        let state = server_with_mode(WorkspaceModeId::GroupChat);
        register_n_agents(&state, 2);
        let err = set_mode(&state, WorkspaceModeId::Dm).expect_err("blocked");
        assert!(matches!(err, AgentHubError::DmModeTransition { count: 2 }));
    }

    #[test]
    fn transition_server_to_dm_succeeds_with_one_agent() {
        let state = server_with_mode(WorkspaceModeId::Server);
        register_n_agents(&state, 1);
        set_mode(&state, WorkspaceModeId::Dm).expect("ok");
        assert_eq!(get_mode(&state), WorkspaceModeId::Dm);
    }

    #[test]
    fn transition_dm_to_server_with_no_agents() {
        let state = server_with_mode(WorkspaceModeId::Dm);
        set_mode(&state, WorkspaceModeId::Server).expect("transition");
        assert_eq!(get_mode(&state), WorkspaceModeId::Server);
    }

    #[test]
    fn set_mode_noop_when_unchanged() {
        let state = server_with_mode(WorkspaceModeId::GroupChat);
        let change = set_mode(&state, WorkspaceModeId::GroupChat).expect("noop");
        assert_eq!(change.old, WorkspaceModeId::GroupChat);
        assert_eq!(change.new, WorkspaceModeId::GroupChat);
    }

    #[test]
    fn cycle_mode_visits_all_three() {
        let state = server_with_mode(WorkspaceModeId::Dm);
        assert_eq!(
            cycle_mode(&state).expect("cycle").new,
            WorkspaceModeId::GroupChat
        );
        assert_eq!(
            cycle_mode(&state).expect("cycle").new,
            WorkspaceModeId::Server
        );
        assert_eq!(cycle_mode(&state).expect("cycle").new, WorkspaceModeId::Dm);
    }

    #[test]
    fn dm_rejects_second_agent_spawn() {
        let existing_id = Uuid::new_v4();
        let err = validate_spawn_for_mode(
            WorkspaceModeId::Dm,
            1,
            16,
            Some((existing_id, "gemini-1".into())),
            "claude-1",
        )
        .expect_err("dm limit");
        assert!(matches!(
            err,
            AgentHubError::DmAgentLimit {
                existing_id: id,
                existing_tag,
                new_tag,
            } if id == existing_id && existing_tag == "gemini-1" && new_tag == "claude-1"
        ));
    }

    #[test]
    fn dm_allows_first_agent_spawn() {
        validate_spawn_for_mode(WorkspaceModeId::Dm, 0, 16, None, "gemini-1").expect("first ok");
    }

    #[test]
    fn group_chat_allows_multiple_up_to_config_max() {
        validate_spawn_for_mode(WorkspaceModeId::GroupChat, 15, 16, None, "agent-16").expect("ok");
        assert!(
            validate_spawn_for_mode(WorkspaceModeId::GroupChat, 16, 16, None, "overflow").is_err()
        );
    }

    #[test]
    fn server_enforces_permissions() {
        let state = server_with_mode(WorkspaceModeId::Server);
        let id = register_agent_state(&state, "Observer");
        check_permission(&state, id, Permissions::VIEW_CHANNEL).expect("view ok");
        let err = check_permission(&state, id, Permissions::EXECUTE_UNIX).expect_err("denied");
        assert!(matches!(err, AgentHubError::PermissionDenied { .. }));
        assert_eq!(permission_label(Permissions::EXECUTE_UNIX), "EXECUTE_UNIX");
    }

    #[test]
    fn group_chat_skips_permission_denial() {
        let state = server_with_mode(WorkspaceModeId::GroupChat);
        let id = register_agent_state(&state, "Observer");
        check_permission(&state, id, Permissions::EXECUTE_UNIX).expect("permissive");
    }

    #[test]
    fn dm_skips_permission_denial() {
        let state = server_with_mode(WorkspaceModeId::Dm);
        let id = register_agent_state(&state, "Observer");
        check_permission(&state, id, Permissions::EXECUTE_UNIX).expect("implicit full");
    }

    #[test]
    fn effective_permissions_dm_is_full() {
        let state = server_with_mode(WorkspaceModeId::Dm);
        let id = register_agent_state(&state, "Observer");
        let perms = effective_permissions(&state, id).expect("perms");
        assert_eq!(perms, DM_FULL_PERMISSIONS);
    }

    #[test]
    fn effective_permissions_group_chat_is_permissive() {
        let state = server_with_mode(WorkspaceModeId::GroupChat);
        let id = register_agent_state(&state, "Observer");
        let perms = effective_permissions(&state, id).expect("perms");
        assert!(perms.contains(Permissions::EXECUTE_UNIX));
        assert!(perms.contains(Permissions::RECEIVE_BROADCAST));
    }

    #[test]
    fn effective_permissions_server_matches_role() {
        let state = server_with_mode(WorkspaceModeId::Server);
        let id = register_agent_state(&state, "Observer");
        let perms = effective_permissions(&state, id).expect("perms");
        assert_eq!(perms, default_roles()["Observer"]);
    }

    #[test]
    fn dm_spawn_conflict_reports_existing_agent() {
        let state = server_with_mode(WorkspaceModeId::Dm);
        let id = register_agent_state(&state, "Builder");
        let conflict = dm_spawn_conflict(&state, "claude-1").expect("conflict");
        assert_eq!(conflict.existing_id, id);
        assert_eq!(conflict.existing_tag, "Builder-1");
        assert_eq!(conflict.new_tag, "claude-1");
    }

    #[test]
    fn parse_channel_tag_extracts_name() {
        assert_eq!(
            parse_channel_tag("post to #backend team"),
            Some("backend".into())
        );
        assert_eq!(parse_channel_tag("no channel here"), None);
    }

    #[test]
    fn server_channel_lifecycle() {
        let state = server_with_mode(WorkspaceModeId::Server);
        create_channel(&state, "backend").expect("create");
        let id = register_agent_state(&state, "Builder");
        assign_agent_to_channel(&state, "backend", id).expect("assign");
        assert_eq!(channel_members(&state, "backend"), vec![id]);
        let gemini = Uuid::new_v4();
        state.agent_states.insert(
            gemini,
            Arc::new(AgentState::new(
                gemini,
                "gemini-1".into(),
                "mock".into(),
                "Builder".into(),
                default_roles()["Builder"],
                1,
            )),
        );
        let filtered = filter_recipients_by_channel(&state, "update #backend", vec![id, gemini]);
        assert_eq!(filtered, vec![id]);
        delete_channel(&state, "backend").expect("delete");
        assert!(create_channel(&state, "backend").is_ok());
    }

    #[test]
    fn channels_rejected_outside_server_mode() {
        let state = server_with_mode(WorkspaceModeId::GroupChat);
        assert!(create_channel(&state, "general").is_err());
    }

    #[test]
    fn enforce_write_files_blocks_observer_in_server_mode() {
        let state = server_with_mode(WorkspaceModeId::Server);
        let id = register_agent_state(&state, "Observer");
        assert!(detect_filesystem_write_attempt("echo hello > out.txt"));
        let err = enforce_write_files(&state, id, "echo hello > out.txt").expect_err("blocked");
        assert!(matches!(err, AgentHubError::WriteFilesBlocked { agent_id } if agent_id == id));
    }

    #[test]
    fn enforce_write_files_permissive_in_group_chat() {
        let state = server_with_mode(WorkspaceModeId::GroupChat);
        let id = register_agent_state(&state, "Observer");
        enforce_write_files(&state, id, "echo hello > out.txt").expect("allowed");
    }

    #[test]
    fn enforce_trigger_pipeline_denied_for_observer_in_server() {
        let state = server_with_mode(WorkspaceModeId::Server);
        let id = register_agent_state(&state, "Observer");
        assert!(enforce_trigger_pipeline(&state, id).is_err());
        let builder = register_agent_state(&state, "Builder");
        enforce_trigger_pipeline(&state, builder).expect("builder may pipeline");
    }

    #[test]
    fn enforce_broadcast_output_denied_for_auditor_in_server() {
        let state = server_with_mode(WorkspaceModeId::Server);
        let id = register_agent_state(&state, "Auditor");
        assert!(enforce_broadcast_output(&state, id).is_err());
    }

    #[test]
    fn workspace_mode_id_config_roundtrip() {
        use crate::config::WorkspaceMode;
        assert_eq!(
            WorkspaceModeId::from_config(WorkspaceMode::DirectMessage),
            WorkspaceModeId::Dm
        );
        assert_eq!(
            WorkspaceModeId::Dm.to_config(),
            WorkspaceMode::DirectMessage
        );
    }

    #[test]
    fn group_chat_default_permissions_cover_blueprint_minimum() {
        assert!(GROUP_CHAT_DEFAULT_PERMISSIONS.contains(Permissions::SEND_MESSAGES));
        assert!(GROUP_CHAT_DEFAULT_PERMISSIONS.contains(Permissions::RECEIVE_BROADCAST));
    }

    #[test]
    fn channels_only_enabled_in_server_mode() {
        assert!(!channels_enabled(WorkspaceModeId::Dm));
        assert!(!channels_enabled(WorkspaceModeId::GroupChat));
        assert!(channels_enabled(WorkspaceModeId::Server));
    }

    #[test]
    fn set_mode_leaving_server_clears_channels() {
        let state = server_with_mode(WorkspaceModeId::Server);
        create_channel(&state, "backend").expect("create");
        let id = register_agent_state(&state, "Builder");
        assign_agent_to_channel(&state, "backend", id).expect("assign");
        set_mode(&state, WorkspaceModeId::GroupChat).expect("transition");
        assert!(state.channels.is_empty());
    }

    #[test]
    fn normalize_channel_name_strips_hash_and_lowercases() {
        assert_eq!(normalize_channel_name("  #Backend-Room  "), "backend-room");
    }

    #[test]
    fn filter_recipients_unknown_channel_tag_clears_recipients() {
        let state = server_with_mode(WorkspaceModeId::Server);
        create_channel(&state, "backend").expect("create");
        let id = Uuid::new_v4();
        let filtered = filter_recipients_by_channel(&state, "ping #frontend", vec![id]);
        assert!(filtered.is_empty());
    }

    #[test]
    fn filter_recipients_without_tag_passes_through() {
        let state = server_with_mode(WorkspaceModeId::Server);
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let filtered = filter_recipients_by_channel(&state, "general update", vec![a, b]);
        assert_eq!(filtered, vec![a, b]);
    }

    #[test]
    fn filter_recipients_ignored_in_group_chat() {
        let state = server_with_mode(WorkspaceModeId::GroupChat);
        create_channel(&state, "backend").expect_err("no channels");
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let filtered = filter_recipients_by_channel(&state, "update #backend", vec![a, b]);
        assert_eq!(filtered, vec![a, b]);
    }

    #[test]
    fn dm_spawn_conflict_none_outside_dm() {
        let state = server_with_mode(WorkspaceModeId::GroupChat);
        register_agent_state(&state, "Builder");
        assert!(dm_spawn_conflict(&state, "claude-1").is_none());
    }

    #[test]
    fn enforce_send_messages_blocks_observer_in_server() {
        let state = server_with_mode(WorkspaceModeId::Server);
        let id = register_agent_state(&state, "Observer");
        let err = enforce_send_messages(&state, id).expect_err("no send");
        assert!(matches!(err, AgentHubError::PermissionDenied { .. }));
    }

    #[test]
    fn enforce_trigger_pipeline_permissive_in_group_chat() {
        let state = server_with_mode(WorkspaceModeId::GroupChat);
        let id = register_agent_state(&state, "Observer");
        enforce_trigger_pipeline(&state, id).expect("permissive");
    }

    #[test]
    fn gate_agent_bus_output_blocks_send_in_server() {
        let state = server_with_mode(WorkspaceModeId::Server);
        let id = register_agent_state(&state, "Observer");
        assert!(gate_agent_bus_output(&state, id, "hello").is_err());
    }

    #[test]
    fn gate_agent_bus_output_allows_builder_in_server() {
        let state = server_with_mode(WorkspaceModeId::Server);
        let id = register_agent_state(&state, "Builder");
        gate_agent_bus_output(&state, id, "cargo build").expect("ok");
    }

    #[test]
    fn agent_may_broadcast_false_for_observer_in_server() {
        let state = server_with_mode(WorkspaceModeId::Server);
        let id = register_agent_state(&state, "Reviewer");
        assert!(agent_may_broadcast(&state, id));
        let id = register_agent_state(&state, "Observer");
        assert!(!agent_may_broadcast(&state, id));
    }

    #[test]
    fn agent_may_broadcast_always_true_in_group_chat() {
        let state = server_with_mode(WorkspaceModeId::GroupChat);
        let id = register_agent_state(&state, "Observer");
        assert!(agent_may_broadcast(&state, id));
    }

    #[test]
    fn cycle_mode_blocked_when_entering_dm_with_multiple_agents() {
        let state = server_with_mode(WorkspaceModeId::GroupChat);
        register_n_agents(&state, 2);
        set_mode(&state, WorkspaceModeId::Server).expect("to server");
        let err = cycle_mode(&state).expect_err("dm blocked");
        assert!(matches!(err, AgentHubError::DmModeTransition { count: 2 }));
        assert_eq!(get_mode(&state), WorkspaceModeId::Server);
    }

    #[test]
    fn detect_filesystem_write_patterns() {
        assert!(detect_filesystem_write_attempt("echo hello > out.txt"));
        assert!(detect_filesystem_write_attempt("use write_file tool"));
        assert!(!detect_filesystem_write_attempt("read the config file"));
    }

    #[test]
    fn validate_spawn_dm_requires_existing_metadata_when_count_one() {
        let err = validate_spawn_for_mode(WorkspaceModeId::Dm, 1, 16, None, "new")
            .expect_err("needs existing");
        assert!(matches!(err, AgentHubError::Config(_)));
    }

    #[test]
    fn remove_agent_from_channel_errors_when_not_member() {
        let state = server_with_mode(WorkspaceModeId::Server);
        create_channel(&state, "ops").expect("create");
        let id = register_agent_state(&state, "Builder");
        assert!(remove_agent_from_channel(&state, "ops", id).is_err());
    }
}
