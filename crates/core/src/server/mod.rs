//! Server mechanics: RBAC (§8.1–8.2), moderation (§8.3), induction (§8.4), modes (§9).

pub mod rbac;
pub mod state;

#[cfg(feature = "full")]
pub mod induction;
#[cfg(feature = "full")]
pub mod moderation;
#[cfg(any(feature = "full", feature = "server-tests", feature = "bus-tests"))]
pub mod modes;
#[cfg(feature = "full")]
pub mod shutdown;

pub use rbac::{
    agent_state_for_role, agenthub_config_dir, check_permission as check_agent_permission,
    default_roles, is_builtin_role, load_custom_roles, parse_permission_names,
    permission_from_name, permission_label, permission_names, permissions_for_role,
    populate_session_roles, remove_custom_role, roles_json_path, save_custom_roles,
    upsert_custom_role, AgentState, CustomRoleDefinition, Permissions, BUILTIN_ROLES,
};
pub use state::ServerState;

#[cfg(feature = "full")]
pub use moderation::{
    assign_agent_role, execute_command, parse_spawn_args, try_handle_slash_command,
    ModerationContext,
};
#[cfg(feature = "full")]
pub use modes::sync_agent_pty;
#[cfg(any(feature = "full", feature = "server-tests", feature = "bus-tests"))]
pub use modes::{
    active_agent_count, admin_commands_enabled, agent_may_broadcast, assign_agent_to_channel,
    broadcast_enabled, channel_members, channels_enabled, chaos_heuristics_enabled,
    check_permission as check_mode_permission, create_channel, cycle_mode, delete_channel,
    detect_filesystem_write_attempt, dm_spawn_conflict, effective_permissions,
    enforce_broadcast_output, enforce_execute_unix, enforce_send_messages,
    enforce_trigger_pipeline, enforce_write_files, filter_recipients_by_channel,
    gate_agent_bus_output, get_mode, normalize_channel_name, parse_channel_tag, rbac_enforced,
    remove_agent_from_channel, set_mode as set_workspace_mode, simplified_induction,
    validate_spawn, validate_spawn_for_mode, DmSpawnConflict, ModeChange, WorkspaceModeId,
    DM_FULL_PERMISSIONS, GROUP_CHAT_DEFAULT_PERMISSIONS,
};
#[cfg(feature = "full")]
pub use shutdown::{install_global_shutdown, kill_all_agents};
