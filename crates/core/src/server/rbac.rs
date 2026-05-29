//! Roles, permissions, and per-agent RBAC state (blueprint §8.1–8.2).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use bitflags::bitflags;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AgentHubError, Result};

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(transparent)]
    pub struct Permissions: u64 {
        /// Can see messages in the chat.
        const VIEW_CHANNEL          = 1 << 0;
        /// Can generate and send messages.
        const SEND_MESSAGES         = 1 << 1;
        /// Output is broadcast to other agents.
        const BROADCAST_OUTPUT      = 1 << 2;
        /// Can receive broadcasts from other agents.
        const RECEIVE_BROADCAST     = 1 << 3;
        /// Can execute Unix commands via the pipeline `>` syntax.
        const EXECUTE_UNIX          = 1 << 4;
        /// Can write files to the workspace.
        const WRITE_FILES           = 1 << 5;
        /// Can trigger pipeline handoffs to other agents.
        const TRIGGER_PIPELINE      = 1 << 6;
        /// Can override or veto outputs from other agents (Leader role).
        const OVERRIDE_OTHERS       = 1 << 7;
        /// Can call /promote and /demote (Moderator role).
        const MODIFY_ROLES          = 1 << 8;
        /// Can spawn additional agent instances.
        const SPAWN_AGENTS          = 1 << 9;
    }
}

/// Built-in role names (cannot be deleted from disk; may be overridden in memory).
pub const BUILTIN_ROLES: &[&str] = &[
    "Leader",
    "Builder",
    "Reviewer",
    "Auditor",
    "Moderator",
    "Subagent",
    "Observer",
];

/// Built-in role definitions. These cannot be deleted, only overridden.
#[must_use]
pub fn default_roles() -> HashMap<String, Permissions> {
    use Permissions as P;
    [
        (
            "Leader",
            P::VIEW_CHANNEL
                | P::SEND_MESSAGES
                | P::BROADCAST_OUTPUT
                | P::RECEIVE_BROADCAST
                | P::EXECUTE_UNIX
                | P::WRITE_FILES
                | P::TRIGGER_PIPELINE
                | P::OVERRIDE_OTHERS
                | P::MODIFY_ROLES,
        ),
        (
            "Builder",
            P::VIEW_CHANNEL
                | P::SEND_MESSAGES
                | P::BROADCAST_OUTPUT
                | P::RECEIVE_BROADCAST
                | P::EXECUTE_UNIX
                | P::WRITE_FILES
                | P::TRIGGER_PIPELINE,
        ),
        (
            "Reviewer",
            P::VIEW_CHANNEL | P::SEND_MESSAGES | P::BROADCAST_OUTPUT | P::RECEIVE_BROADCAST,
        ),
        (
            "Auditor",
            P::VIEW_CHANNEL | P::SEND_MESSAGES | P::RECEIVE_BROADCAST,
        ),
        (
            "Moderator",
            P::VIEW_CHANNEL
                | P::SEND_MESSAGES
                | P::BROADCAST_OUTPUT
                | P::RECEIVE_BROADCAST
                | P::MODIFY_ROLES,
        ),
        (
            "Subagent",
            P::VIEW_CHANNEL
                | P::SEND_MESSAGES
                | P::BROADCAST_OUTPUT
                | P::EXECUTE_UNIX
                | P::WRITE_FILES,
        ),
        ("Observer", P::VIEW_CHANNEL),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), *v))
    .collect()
}

/// Full runtime RBAC state for one agent. Stored in [`super::state::ServerState::agent_states`].
#[derive(Debug)]
pub struct AgentState {
    pub id: Uuid,
    pub tag: String,
    pub driver_name: String,
    pub role: String,
    pub permissions: Permissions,
    /// Instance number. If this is the 2nd Gemini, instance_number = 2.
    pub instance_number: u8,
    /// Timestamp of when the agent came online.
    pub online_since: DateTime<Utc>,
    /// Unix timestamp until which the agent is timed out. 0 = not timed out.
    pub timeout_until: AtomicI64,
    /// Whether this agent is permanently banned (driver blocklisted for this session).
    pub banned: bool,
}

impl AgentState {
    #[must_use]
    pub fn new(
        id: Uuid,
        tag: String,
        driver_name: String,
        role: String,
        permissions: Permissions,
        instance_number: u8,
    ) -> Self {
        Self {
            id,
            tag,
            driver_name,
            role,
            permissions,
            instance_number,
            online_since: Utc::now(),
            timeout_until: AtomicI64::new(0),
            banned: false,
        }
    }

    /// Returns `true` when the agent holds all bits in `required`.
    #[must_use]
    pub fn has_permission(&self, required: Permissions) -> bool {
        !self.banned && self.permissions.contains(required)
    }

    /// Enforces a permission, returning [`AgentHubError::PermissionDenied`] on failure.
    pub fn require_permission(&self, required: Permissions) -> Result<()> {
        if self.has_permission(required) {
            Ok(())
        } else {
            Err(AgentHubError::PermissionDenied {
                agent_id: self.id,
                permission: permission_label(required),
            })
        }
    }

    /// `true` when `timeout_until` is a future Unix timestamp.
    #[must_use]
    pub fn is_timed_out(&self) -> bool {
        let until = self.timeout_until.load(Ordering::Acquire);
        until > 0 && until > Utc::now().timestamp()
    }

    pub fn assign_role(&mut self, role: String, permissions: Permissions) {
        self.role = role;
        self.permissions = permissions;
    }

    /// Reassign role/permissions while preserving timeout, ban, and online timestamp.
    #[must_use]
    pub fn with_role(&self, role: String, permissions: Permissions) -> Self {
        Self {
            id: self.id,
            tag: self.tag.clone(),
            driver_name: self.driver_name.clone(),
            role,
            permissions,
            instance_number: self.instance_number,
            online_since: self.online_since,
            timeout_until: AtomicI64::new(self.timeout_until.load(Ordering::Acquire)),
            banned: self.banned,
        }
    }

    pub fn set_timeout_until(&self, unix_ts: i64) {
        self.timeout_until.store(unix_ts, Ordering::Release);
    }

    pub fn clear_timeout(&self) {
        self.timeout_until.store(0, Ordering::Release);
    }
}

/// Custom role entry in `~/.agenthub/roles.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomRoleDefinition {
    pub name: String,
    pub permissions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub induction_prompt_override: Option<String>,
}

#[must_use]
pub fn is_builtin_role(name: &str) -> bool {
    BUILTIN_ROLES.iter().any(|r| r.eq_ignore_ascii_case(name))
}

/// AgentHub config directory (`~/.agenthub` on Unix, `%USERPROFILE%\.agenthub` on Windows).
///
/// Override with `AGENTHUB_CONFIG_DIR` (used by integration tests for isolated `roles.json`).
#[must_use]
pub fn agenthub_config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("AGENTHUB_CONFIG_DIR") {
        let path = PathBuf::from(dir);
        if !path.as_os_str().is_empty() {
            return path;
        }
    }
    home_dir().join(".agenthub")
}

/// Path to persisted custom roles: `~/.agenthub/roles.json`.
#[must_use]
pub fn roles_json_path() -> PathBuf {
    agenthub_config_dir().join("roles.json")
}

fn home_dir() -> PathBuf {
    if let Some(dir) = dirs::home_dir() {
        return dir;
    }
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(profile);
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home);
    }
    PathBuf::from(".")
}

/// Parse permission flag names into a bitmask.
pub fn parse_permission_names(names: &[impl AsRef<str>]) -> Result<Permissions> {
    let mut combined = Permissions::empty();
    for name in names {
        combined |= permission_from_name(name.as_ref())?;
    }
    Ok(combined)
}

/// Resolves a single permission flag name (e.g. `VIEW_CHANNEL`).
pub fn permission_from_name(name: &str) -> Result<Permissions> {
    all_permission_flags()
        .iter()
        .find(|(label, _)| label.eq_ignore_ascii_case(name))
        .map(|(_, flag)| *flag)
        .ok_or_else(|| AgentHubError::Config(format!("unknown permission flag: {name}")))
}

/// Permission flag names present in `perm`.
#[must_use]
pub fn permission_names(perm: Permissions) -> Vec<String> {
    let mut names = Vec::new();
    for (name, flag) in all_permission_flags() {
        if perm.contains(*flag) {
            names.push((*name).to_string());
        }
    }
    names
}

/// Human-readable label for a permission set (used in errors).
#[must_use]
pub fn permission_label(perm: Permissions) -> String {
    if perm.is_empty() {
        return "NONE".to_string();
    }
    let mut names = Vec::new();
    for (name, flag) in all_permission_flags() {
        if perm.contains(*flag) {
            names.push(*name);
        }
    }
    names.join("|")
}

fn all_permission_flags() -> &'static [(&'static str, Permissions)] {
    &[
        ("VIEW_CHANNEL", Permissions::VIEW_CHANNEL),
        ("SEND_MESSAGES", Permissions::SEND_MESSAGES),
        ("BROADCAST_OUTPUT", Permissions::BROADCAST_OUTPUT),
        ("RECEIVE_BROADCAST", Permissions::RECEIVE_BROADCAST),
        ("EXECUTE_UNIX", Permissions::EXECUTE_UNIX),
        ("WRITE_FILES", Permissions::WRITE_FILES),
        ("TRIGGER_PIPELINE", Permissions::TRIGGER_PIPELINE),
        ("OVERRIDE_OTHERS", Permissions::OVERRIDE_OTHERS),
        ("MODIFY_ROLES", Permissions::MODIFY_ROLES),
        ("SPAWN_AGENTS", Permissions::SPAWN_AGENTS),
    ]
}

/// Load custom role definitions from disk. Missing file yields an empty list.
pub fn load_custom_roles(path: &Path) -> Result<Vec<CustomRoleDefinition>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let data = std::fs::read_to_string(path)?;
    if data.trim().is_empty() {
        return Ok(Vec::new());
    }
    Ok(serde_json::from_str(&data)?)
}

/// Upsert one custom role into `roles.json` (built-in names are not persisted).
pub fn upsert_custom_role(path: &Path, def: &CustomRoleDefinition) -> Result<()> {
    let mut custom = load_custom_roles(path)?;
    custom.retain(|r| !r.name.eq_ignore_ascii_case(&def.name));
    if !is_builtin_role(&def.name) {
        custom.push(def.clone());
    }
    save_custom_roles(path, &custom)
}

/// Remove a custom role from `roles.json` by name.
pub fn remove_custom_role(path: &Path, name: &str) -> Result<()> {
    let custom: Vec<_> = load_custom_roles(path)?
        .into_iter()
        .filter(|r| !r.name.eq_ignore_ascii_case(name))
        .collect();
    save_custom_roles(path, &custom)
}

/// Persist custom (non-built-in) roles to `roles.json`.
pub fn save_custom_roles(path: &Path, roles: &[CustomRoleDefinition]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let custom: Vec<_> = roles
        .iter()
        .filter(|r| !is_builtin_role(&r.name))
        .cloned()
        .collect();
    let json = serde_json::to_string_pretty(&custom)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Merge built-in defaults with custom roles from disk into the session maps.
pub fn populate_session_roles(
    roles: &DashMap<String, Permissions>,
    induction_overrides: &DashMap<String, String>,
) -> Result<()> {
    roles.clear();
    induction_overrides.clear();

    for (name, perms) in default_roles() {
        roles.insert(name, perms);
    }

    let path = roles_json_path();
    for def in load_custom_roles(&path)? {
        let perms = parse_permission_names(&def.permissions)?;
        roles.insert(def.name.clone(), perms);
        if let Some(prompt) = def.induction_prompt_override {
            induction_overrides.insert(def.name, prompt);
        }
    }

    Ok(())
}

/// Look up permissions for a role name in the session role table.
#[must_use]
pub fn permissions_for_role(
    roles: &DashMap<String, Permissions>,
    role: &str,
) -> Option<Permissions> {
    roles
        .iter()
        .find(|entry| entry.key().eq_ignore_ascii_case(role))
        .map(|entry| *entry.value())
}

/// Create an [`AgentState`] for a newly spawned agent using the session role table.
pub fn agent_state_for_role(
    id: Uuid,
    tag: String,
    driver_name: String,
    role: &str,
    instance_number: u8,
    roles: &DashMap<String, Permissions>,
) -> Result<Arc<AgentState>> {
    let permissions = permissions_for_role(roles, role)
        .ok_or_else(|| AgentHubError::RoleNotFound(role.to_string()))?;
    Ok(Arc::new(AgentState::new(
        id,
        tag,
        driver_name,
        role.to_string(),
        permissions,
        instance_number,
    )))
}

/// Checks whether `state` holds `permission`.
pub fn check_permission(state: &AgentState, permission: Permissions) -> Result<()> {
    state.require_permission(permission)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_roles_has_seven_builtin_roles() {
        let roles = default_roles();
        assert_eq!(roles.len(), 7);
        for name in BUILTIN_ROLES {
            assert!(roles.contains_key(*name), "missing builtin role {name}");
        }
    }

    #[test]
    fn leader_has_admin_permissions() {
        let roles = default_roles();
        let leader = roles["Leader"];
        assert!(leader.contains(Permissions::OVERRIDE_OTHERS));
        assert!(leader.contains(Permissions::MODIFY_ROLES));
        assert!(leader.contains(Permissions::VIEW_CHANNEL));
        assert!(!leader.contains(Permissions::SPAWN_AGENTS));
    }

    #[test]
    fn observer_is_view_only() {
        let roles = default_roles();
        assert_eq!(roles["Observer"], Permissions::VIEW_CHANNEL);
    }

    #[test]
    fn reviewer_and_auditor_broadcast_rules() {
        let roles = default_roles();
        let reviewer = roles["Reviewer"];
        assert!(reviewer.contains(Permissions::BROADCAST_OUTPUT));
        assert!(!reviewer.contains(Permissions::EXECUTE_UNIX));

        let auditor = roles["Auditor"];
        assert!(auditor.contains(Permissions::RECEIVE_BROADCAST));
        assert!(!auditor.contains(Permissions::BROADCAST_OUTPUT));
    }

    #[test]
    fn moderator_can_modify_roles_not_execute() {
        let roles = default_roles();
        let moderator = roles["Moderator"];
        assert!(moderator.contains(Permissions::MODIFY_ROLES));
        assert!(!moderator.contains(Permissions::EXECUTE_UNIX));
    }

    #[test]
    fn subagent_execute_without_broadcast_receive() {
        let roles = default_roles();
        let subagent = roles["Subagent"];
        assert!(subagent.contains(Permissions::EXECUTE_UNIX));
        assert!(!subagent.contains(Permissions::RECEIVE_BROADCAST));
    }

    #[test]
    fn builder_can_execute_and_pipeline() {
        let roles = default_roles();
        let builder = roles["Builder"];
        assert!(builder.contains(Permissions::EXECUTE_UNIX));
        assert!(builder.contains(Permissions::TRIGGER_PIPELINE));
        assert!(!builder.contains(Permissions::MODIFY_ROLES));
    }

    #[test]
    fn parse_permission_names_roundtrip() {
        let parsed = parse_permission_names(&["view_channel", "SEND_MESSAGES"]).unwrap();
        assert_eq!(
            parsed,
            Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES
        );
    }

    #[test]
    fn parse_unknown_permission_errors() {
        let err = parse_permission_names(&["NOT_A_FLAG"]).unwrap_err();
        assert!(matches!(err, AgentHubError::Config(_)));
    }

    #[test]
    fn agent_has_and_require_permission() {
        let agent = AgentState::new(
            Uuid::new_v4(),
            "gemini-1".into(),
            "gemini".into(),
            "Builder".into(),
            default_roles()["Builder"],
            1,
        );
        assert!(agent.has_permission(Permissions::WRITE_FILES));
        assert!(!agent.has_permission(Permissions::MODIFY_ROLES));
        agent
            .require_permission(Permissions::SEND_MESSAGES)
            .unwrap();
        assert!(matches!(
            agent.require_permission(Permissions::MODIFY_ROLES),
            Err(AgentHubError::PermissionDenied { .. })
        ));
    }

    #[test]
    fn banned_agent_denies_all_permissions() {
        let mut agent = AgentState::new(
            Uuid::new_v4(),
            "gemini-1".into(),
            "gemini".into(),
            "Leader".into(),
            default_roles()["Leader"],
            1,
        );
        agent.banned = true;
        assert!(!agent.has_permission(Permissions::VIEW_CHANNEL));
        assert!(check_permission(&agent, Permissions::VIEW_CHANNEL).is_err());
    }

    #[test]
    fn custom_roles_load_save_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roles.json");

        let defs = vec![CustomRoleDefinition {
            name: "SecurityAuditor".into(),
            permissions: vec![
                "VIEW_CHANNEL".into(),
                "SEND_MESSAGES".into(),
                "RECEIVE_BROADCAST".into(),
            ],
            induction_prompt_override: Some("security only".into()),
        }];
        save_custom_roles(&path, &defs).unwrap();

        let loaded = load_custom_roles(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "SecurityAuditor");

        let perms = parse_permission_names(&loaded[0].permissions).unwrap();
        assert!(perms.contains(Permissions::RECEIVE_BROADCAST));
        assert!(!perms.contains(Permissions::EXECUTE_UNIX));
    }

    #[test]
    fn save_custom_roles_strips_builtin_names() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roles.json");

        let defs = vec![
            CustomRoleDefinition {
                name: "Leader".into(),
                permissions: vec!["VIEW_CHANNEL".into()],
                induction_prompt_override: None,
            },
            CustomRoleDefinition {
                name: "Custom".into(),
                permissions: vec!["VIEW_CHANNEL".into()],
                induction_prompt_override: None,
            },
        ];
        save_custom_roles(&path, &defs).unwrap();
        let loaded = load_custom_roles(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "Custom");
    }

    #[test]
    fn populate_session_roles_merges_custom_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roles.json");
        save_custom_roles(
            &path,
            &[CustomRoleDefinition {
                name: "SecurityAuditor".into(),
                permissions: vec!["VIEW_CHANNEL".into(), "RECEIVE_BROADCAST".into()],
                induction_prompt_override: Some("audit".into()),
            }],
        )
        .unwrap();

        let roles_map = DashMap::new();
        let overrides_map = DashMap::new();
        for (name, perms) in default_roles() {
            roles_map.insert(name, perms);
        }
        for def in load_custom_roles(&path).unwrap() {
            let perms = parse_permission_names(&def.permissions).unwrap();
            roles_map.insert(def.name.clone(), perms);
            if let Some(p) = def.induction_prompt_override {
                overrides_map.insert(def.name, p);
            }
        }

        assert!(roles_map.contains_key("SecurityAuditor"));
        assert_eq!(roles_map.len(), 8);
        assert!(roles_map
            .get("SecurityAuditor")
            .unwrap()
            .contains(Permissions::RECEIVE_BROADCAST));
        assert_eq!(
            overrides_map.get("SecurityAuditor").unwrap().value(),
            "audit"
        );
    }

    #[test]
    fn permissions_for_role_case_insensitive() {
        let roles = DashMap::new();
        roles.insert("Builder".into(), default_roles()["Builder"]);
        let perms = permissions_for_role(&roles, "builder").unwrap();
        assert!(perms.contains(Permissions::WRITE_FILES));
    }

    #[test]
    fn agent_state_for_role_unknown_errors() {
        let roles = DashMap::new();
        roles.insert("Observer".into(), Permissions::VIEW_CHANNEL);
        let err = agent_state_for_role(
            Uuid::new_v4(),
            "x".into(),
            "mock".into(),
            "NoSuchRole",
            1,
            &roles,
        )
        .unwrap_err();
        assert!(matches!(err, AgentHubError::RoleNotFound(_)));
    }

    #[test]
    fn is_timed_out_respects_atomic() {
        let agent = AgentState::new(
            Uuid::new_v4(),
            "a".into(),
            "d".into(),
            "Observer".into(),
            Permissions::VIEW_CHANNEL,
            1,
        );
        assert!(!agent.is_timed_out());
        agent
            .timeout_until
            .store(Utc::now().timestamp() + 3600, Ordering::Release);
        assert!(agent.is_timed_out());
    }

    #[test]
    fn is_builtin_role_recognizes_defaults() {
        assert!(is_builtin_role("Leader"));
        assert!(is_builtin_role("observer"));
        assert!(!is_builtin_role("SecurityAuditor"));
    }

    #[test]
    fn agenthub_config_dir_respects_env_override() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("AGENTHUB_CONFIG_DIR", dir.path());
        assert_eq!(agenthub_config_dir(), dir.path());
        std::env::remove_var("AGENTHUB_CONFIG_DIR");
    }
}
