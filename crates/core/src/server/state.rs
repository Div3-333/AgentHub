use std::sync::atomic::AtomicU8;
#[cfg(feature = "full")]
use std::sync::atomic::Ordering;
use std::sync::Arc;

use dashmap::{DashMap, DashSet};
use uuid::Uuid;

use crate::server::rbac::{
    agent_state_for_role, check_permission as check_agent_permission_inner, permissions_for_role,
    populate_session_roles, AgentState, Permissions,
};

#[cfg(any(feature = "full", feature = "bus-tests"))]
use crate::pty::AgentPty;
#[cfg(feature = "full")]
use crate::server::modes::WorkspaceModeId;
#[cfg(feature = "full")]
use crate::server::modes::MODE_GROUP_CHAT;
#[cfg(all(feature = "bus-tests", not(feature = "full")))]
use crate::server::modes::MODE_GROUP_CHAT;

/// Group chat mode id when the `modes` module is not linked.
#[cfg(not(any(feature = "full", feature = "bus-tests")))]
const MODE_GROUP_CHAT_DEFAULT: u8 = 1;

/// Full session runtime state for the server / workspace (blueprint §8.2).
pub struct ServerState {
    /// Active PTY handles. Key: agent Uuid.
    #[cfg(any(feature = "full", feature = "bus-tests"))]
    pub agents: DashMap<Uuid, Arc<AgentPty>>,
    /// RBAC metadata per agent. Key: agent Uuid.
    pub agent_states: DashMap<Uuid, Arc<AgentState>>,
    /// Role name per agent (used before [`AgentState`] is fully online).
    pub agent_roles: DashMap<Uuid, String>,
    /// Roles available in this session (default + custom).
    pub roles: DashMap<String, Permissions>,
    /// Role induction prompt overrides. Key: role name.
    pub role_induction_overrides: DashMap<String, String>,
    /// Banned driver names for this session.
    pub banned_drivers: DashSet<String>,
    /// Named logical channels → member agent ids (blueprint §9.3, Server mode only).
    pub channels: DashMap<String, Vec<Uuid>>,
    /// Current workspace mode (`modes` module encoding when `full` is enabled).
    pub mode: AtomicU8,
}

impl ServerState {
    pub fn new() -> Self {
        let state = Self {
            #[cfg(any(feature = "full", feature = "bus-tests"))]
            agents: DashMap::new(),
            agent_states: DashMap::new(),
            agent_roles: DashMap::new(),
            roles: DashMap::new(),
            role_induction_overrides: DashMap::new(),
            banned_drivers: DashSet::new(),
            channels: DashMap::new(),
            mode: AtomicU8::new(
                #[cfg(any(feature = "full", feature = "bus-tests"))]
                MODE_GROUP_CHAT,
                #[cfg(not(any(feature = "full", feature = "bus-tests")))]
                MODE_GROUP_CHAT_DEFAULT,
            ),
        };
        if let Err(err) = populate_session_roles(&state.roles, &state.role_induction_overrides) {
            tracing::warn!(%err, "failed to load custom roles; using built-in defaults only");
            for (name, perms) in crate::server::rbac::default_roles() {
                state.roles.insert(name, perms);
            }
        }
        state
    }

    /// Reload role definitions from `~/.agenthub/roles.json` into this session.
    pub fn reload_roles(&self) -> crate::error::Result<()> {
        populate_session_roles(&self.roles, &self.role_induction_overrides)
    }

    #[cfg(feature = "full")]
    #[must_use]
    pub fn mode(&self) -> WorkspaceModeId {
        WorkspaceModeId::from_u8(self.mode.load(Ordering::Acquire))
            .unwrap_or(WorkspaceModeId::GroupChat)
    }

    #[must_use]
    pub fn permissions_for_role(&self, role: &str) -> Option<Permissions> {
        permissions_for_role(&self.roles, role)
    }

    /// Register RBAC metadata for a spawned agent.
    pub fn register_agent_state(
        &self,
        id: Uuid,
        tag: String,
        driver_name: String,
        role: &str,
        instance_number: u8,
    ) -> crate::error::Result<()> {
        self.agent_roles.insert(id, role.to_string());
        self.agent_states.insert(
            id,
            agent_state_for_role(id, tag, driver_name, role, instance_number, &self.roles)?,
        );
        Ok(())
    }

    /// Enforce a permission on an agent's [`AgentState`].
    pub fn check_agent_permission(
        &self,
        agent_id: Uuid,
        required: Permissions,
    ) -> crate::error::Result<()> {
        let agent = self
            .agent_states
            .get(&agent_id)
            .ok_or(crate::error::AgentHubError::AgentNotFound(agent_id))?;
        check_agent_permission_inner(agent.value(), required)
    }

    /// Resolve an active agent by its `@tag` (blueprint §8 moderation commands).
    #[cfg(feature = "full")]
    #[must_use]
    pub fn find_agent_by_tag(&self, tag: &str) -> Option<(Uuid, Arc<AgentPty>)> {
        let needle = tag.strip_prefix('@').unwrap_or(tag);
        self.agents.iter().find_map(|entry| {
            let agent = entry.value();
            if agent.tag.eq_ignore_ascii_case(needle) {
                Some((*entry.key(), Arc::clone(agent)))
            } else {
                None
            }
        })
    }
}

impl Default for ServerState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_loads_seven_builtin_roles() {
        let state = ServerState::new();
        assert_eq!(state.roles.len(), 7);
        let leader = state.roles.get("Leader").expect("Leader");
        assert!(leader.contains(Permissions::MODIFY_ROLES));
    }
}
