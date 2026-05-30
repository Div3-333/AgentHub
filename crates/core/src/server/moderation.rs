//! Moderation commands: mute, deafen, kick, timeout, ban (blueprint §8.3).
//!
//! VFS slash commands (`/snapshot`) are delegated to [`crate::vfs`].

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::bus::{BusEvent, OfflineReason};
use crate::config::AgentHubConfig;
use crate::db::DbClient;
use crate::error::{AgentHubError, Result};
use crate::pty::{freeze_agent_pids, resume_agent_pids};
use crate::pty::{kill_agent, spawn_agent, AgentPty, PtyStatus, SpawnOptions};
use crate::server::modes::{self, set_mode as set_workspace_mode, WorkspaceModeId};
use crate::server::rbac::{
    self, is_builtin_role, parse_permission_names, CustomRoleDefinition, Permissions,
};
use crate::server::state::ServerState;

/// Context for executing moderation slash commands.
pub struct ModerationContext {
    pub state: Arc<ServerState>,
    pub config: Arc<AgentHubConfig>,
    pub db: Option<Arc<DbClient>>,
    pub bus_tx: broadcast::Sender<BusEvent>,
    pub session_id: Uuid,
    pub cwd: std::path::PathBuf,
    /// Display name of who issued the command (e.g. `"user"`, `"system"`).
    pub issued_by: String,
    /// When set, RBAC checks apply to this agent in Server mode.
    pub caller_agent_id: Option<Uuid>,
}

impl ModerationContext {
    fn require_modify_roles(&self) -> Result<()> {
        if self.issued_by == "system" || self.issued_by == "user" {
            return Ok(());
        }
        let Some(caller_id) = self.caller_agent_id else {
            return Ok(());
        };
        modes::check_permission(&self.state, caller_id, Permissions::MODIFY_ROLES)
    }
}

/// Handles moderation and VFS slash commands when recognized.
pub async fn try_handle_slash_command(
    input: &str,
    db: &DbClient,
    config: &AgentHubConfig,
    cwd: &Path,
    session_id: Uuid,
    bus_tx: Option<&broadcast::Sender<BusEvent>>,
    state: Option<&ServerState>,
) -> Result<Option<String>> {
    crate::vfs::handle_slash_command(input, db, config, cwd, session_id, bus_tx, state).await
}

/// Parses and executes a moderation slash command (blueprint §8.3).
pub async fn execute_command(ctx: &ModerationContext, line: &str) -> Result<String> {
    let line = line.trim();
    if !line.starts_with('/') {
        return Err(AgentHubError::Config("not a slash command".into()));
    }

    let parts: Vec<&str> = line.split_whitespace().collect();
    let cmd = parts.first().copied().unwrap_or("").trim_start_matches('/');

    match cmd {
        "mute" => cmd_mute(ctx, &parts[1..]).await,
        "unmute" => cmd_unmute(ctx, &parts[1..]).await,
        "deafen" => cmd_deafen(ctx, &parts[1..]).await,
        "undeafen" => cmd_undeafen(ctx, &parts[1..]).await,
        "timeout" => cmd_timeout(ctx, &parts[1..]).await,
        "kick" => cmd_kick(ctx, &parts[1..]).await,
        "ban" => cmd_ban(ctx, &parts[1..]).await,
        "promote" => cmd_promote(ctx, &parts[1..]).await,
        "demote" => cmd_demote(ctx, &parts[1..]).await,
        "addrole" => cmd_addrole(ctx, &parts[1..]).await,
        "removerole" => cmd_removerole(ctx, &parts[1..]).await,
        "mode" => cmd_mode(ctx, &parts[1..]).await,
        "setprompt" => cmd_setprompt(ctx, &parts[1..]).await,
        "spawn" => cmd_spawn(ctx, &parts[1..]).await,
        "spar" => cmd_spar(ctx, line).await,
        "channel" => cmd_channel(ctx, &parts[1..]).await,
        other => Err(AgentHubError::Config(format!("unknown command: /{other}"))),
    }
}

fn resolve_tag(state: &ServerState, raw: &str) -> Result<(Uuid, Arc<AgentPty>)> {
    let tag = raw.strip_prefix('@').unwrap_or(raw);
    for entry in state.agents.iter() {
        if entry.value().tag.eq_ignore_ascii_case(tag) {
            return Ok((*entry.key(), Arc::clone(entry.value())));
        }
    }
    Err(AgentHubError::AgentNotFound(Uuid::nil()))
}

fn parse_duration(s: &str) -> Result<Duration> {
    let s = s.trim();
    if s.is_empty() {
        return Err(AgentHubError::Config(
            "duration required (e.g. 30s, 5m, 2h)".into(),
        ));
    }
    let (num_str, unit) = if let Some(stripped) = s.strip_suffix('s').or(s.strip_suffix('S')) {
        (stripped, 's')
    } else if let Some(stripped) = s.strip_suffix('m').or(s.strip_suffix('M')) {
        (stripped, 'm')
    } else if let Some(stripped) = s.strip_suffix('h').or(s.strip_suffix('H')) {
        (stripped, 'h')
    } else {
        return Err(AgentHubError::Config(format!("invalid duration: {s}")));
    };
    let n: u64 = num_str
        .trim()
        .parse()
        .map_err(|_| AgentHubError::Config(format!("invalid duration number: {s}")))?;
    let secs = match unit {
        's' => n,
        'm' => n.saturating_mul(60),
        'h' => n.saturating_mul(3600),
        _ => unreachable!(),
    };
    Ok(Duration::from_secs(secs))
}

pub fn assign_agent_role(state: &ServerState, agent_id: Uuid, role: &str) -> Result<()> {
    let perms = state
        .permissions_for_role(role)
        .ok_or_else(|| AgentHubError::RoleNotFound(role.to_string()))?;

    state.agent_roles.insert(agent_id, role.to_string());

    if let Some(updated) = state
        .agent_states
        .get(&agent_id)
        .map(|existing| Arc::new(existing.value().with_role(role.to_string(), perms)))
    {
        state.agent_states.insert(agent_id, updated);
    }

    if let Some(pty) = state.agents.get(&agent_id) {
        pty.set_role(role);
        pty.set_permissions(perms);
    }
    modes::sync_agent_pty(state, agent_id)?;
    Ok(())
}

async fn cmd_mute(ctx: &ModerationContext, args: &[&str]) -> Result<String> {
    let tag = args
        .first()
        .copied()
        .ok_or_else(|| AgentHubError::Config("usage: /mute @{tag}".into()))?;
    let (id, agent) = resolve_tag(&ctx.state, tag)?;
    agent.visible_in_chat.store(false, Ordering::Release);
    agent
        .status
        .store(PtyStatus::Muted.as_u8(), Ordering::Release);
    let _ = ctx.bus_tx.send(BusEvent::AgentMuted {
        id,
        by: ctx.issued_by.clone(),
    });
    Ok(format!("muted @{}", agent.tag))
}

async fn cmd_unmute(ctx: &ModerationContext, args: &[&str]) -> Result<String> {
    let tag = args
        .first()
        .copied()
        .ok_or_else(|| AgentHubError::Config("usage: /unmute @{tag}".into()))?;
    let (id, agent) = resolve_tag(&ctx.state, tag)?;
    agent.visible_in_chat.store(true, Ordering::Release);
    if agent.status() == Some(PtyStatus::Muted) {
        agent
            .status
            .store(PtyStatus::Idle.as_u8(), Ordering::Release);
    }
    let _ = ctx.bus_tx.send(BusEvent::AgentUnmuted {
        id,
        by: ctx.issued_by.clone(),
    });
    Ok(format!("unmuted @{}", agent.tag))
}

async fn cmd_deafen(ctx: &ModerationContext, args: &[&str]) -> Result<String> {
    let tag = args
        .first()
        .copied()
        .ok_or_else(|| AgentHubError::Config("usage: /deafen @{tag}".into()))?;
    let (id, agent) = resolve_tag(&ctx.state, tag)?;
    agent.receives_broadcast.store(false, Ordering::Release);
    agent
        .status
        .store(PtyStatus::Deafened.as_u8(), Ordering::Release);
    let _ = ctx.bus_tx.send(BusEvent::AgentDeafened {
        id,
        by: ctx.issued_by.clone(),
    });
    Ok(format!("deafened @{}", agent.tag))
}

async fn cmd_undeafen(ctx: &ModerationContext, args: &[&str]) -> Result<String> {
    let tag = args
        .first()
        .copied()
        .ok_or_else(|| AgentHubError::Config("usage: /undeafen @{tag}".into()))?;
    let (id, agent) = resolve_tag(&ctx.state, tag)?;
    agent.receives_broadcast.store(true, Ordering::Release);
    if agent.status() == Some(PtyStatus::Deafened) {
        agent
            .status
            .store(PtyStatus::Idle.as_u8(), Ordering::Release);
    }
    let _ = ctx.bus_tx.send(BusEvent::AgentUndeafened {
        id,
        by: ctx.issued_by.clone(),
    });
    Ok(format!("undeafened @{}", agent.tag))
}

async fn cmd_timeout(ctx: &ModerationContext, args: &[&str]) -> Result<String> {
    let tag = args
        .first()
        .copied()
        .ok_or_else(|| AgentHubError::Config("usage: /timeout @{tag} {duration}".into()))?;
    let duration_str = args
        .get(1)
        .copied()
        .ok_or_else(|| AgentHubError::Config("usage: /timeout @{tag} {duration}".into()))?;
    let duration = parse_duration(duration_str)?;
    let secs = duration.as_secs();

    let (id, agent) = resolve_tag(&ctx.state, tag)?;
    let pid = agent.pid;

    freeze_agent_pids(&[pid]);
    agent
        .status
        .store(PtyStatus::Suspended.as_u8(), Ordering::Release);

    let until = Utc::now().timestamp() + secs as i64;
    if let Some(state_entry) = ctx.state.agent_states.get(&id) {
        state_entry.timeout_until.store(until, Ordering::Release);
    }

    let _ = ctx.bus_tx.send(BusEvent::AgentTimedOut {
        id,
        duration_secs: secs,
        by: ctx.issued_by.clone(),
    });

    let state = Arc::clone(&ctx.state);
    let bus_tx = ctx.bus_tx.clone();
    let tag_name = agent.tag.clone();
    tokio::spawn(async move {
        tokio::time::sleep(duration).await;
        if let Some(agent) = state.agents.get(&id) {
            resume_agent_pids(&[agent.pid]);
            agent
                .status
                .store(PtyStatus::Idle.as_u8(), Ordering::Release);
            if let Some(meta) = state.agent_states.get(&id) {
                meta.timeout_until.store(0, Ordering::Release);
            }
            let _ = bus_tx.send(BusEvent::SystemMessage {
                content: format!("[System]: @{tag_name} timeout expired; resumed."),
                timestamp: Utc::now(),
            });
        }
    });

    Ok(format!("timed out @{} for {duration_str}", agent.tag))
}

async fn cmd_kick(ctx: &ModerationContext, args: &[&str]) -> Result<String> {
    let tag = args
        .first()
        .copied()
        .ok_or_else(|| AgentHubError::Config("usage: /kick @{tag} [reason]".into()))?;
    let reason = if args.len() > 1 {
        Some(args[1..].join(" "))
    } else {
        None
    };
    let (id, agent) = resolve_tag(&ctx.state, tag)?;
    let tag_name = agent.tag.clone();

    kick_with_events(
        &ctx.state,
        &ctx.bus_tx,
        id,
        reason.clone(),
        &ctx.issued_by,
        OfflineReason::Kicked,
    )?;

    Ok(format!("kicked @{tag_name}"))
}

fn kick_with_events(
    state: &ServerState,
    bus_tx: &broadcast::Sender<BusEvent>,
    id: Uuid,
    reason: Option<String>,
    by: &str,
    offline_reason: OfflineReason,
) -> Result<()> {
    let _ = bus_tx.send(BusEvent::AgentKicked {
        id,
        reason: reason.clone(),
        by: by.to_string(),
    });
    kill_agent(state, id, bus_tx, offline_reason)?;
    state.agent_states.remove(&id);
    state.agent_roles.remove(&id);
    Ok(())
}

async fn cmd_ban(ctx: &ModerationContext, args: &[&str]) -> Result<String> {
    if !modes::admin_commands_enabled(ctx.state.mode()) {
        return Err(AgentHubError::Config(
            "/ban is only available in Server mode".into(),
        ));
    }
    let tag = args
        .first()
        .copied()
        .ok_or_else(|| AgentHubError::Config("usage: /ban @{tag} [reason]".into()))?;
    let reason = if args.len() > 1 {
        Some(args[1..].join(" "))
    } else {
        None
    };
    let (id, agent) = resolve_tag(&ctx.state, tag)?;
    let driver_name = agent.driver_name.clone();
    let tag_name = agent.tag.clone();

    kick_with_events(
        &ctx.state,
        &ctx.bus_tx,
        id,
        reason,
        &ctx.issued_by,
        OfflineReason::Banned,
    )?;
    ctx.state.banned_drivers.insert(driver_name.clone());

    let _ = ctx.bus_tx.send(BusEvent::AgentBanned {
        id,
        driver_name,
        by: ctx.issued_by.clone(),
    });

    Ok(format!("banned @{tag_name}"))
}

async fn cmd_promote(ctx: &ModerationContext, args: &[&str]) -> Result<String> {
    ctx.require_modify_roles()?;
    let tag = args
        .first()
        .copied()
        .ok_or_else(|| AgentHubError::Config("usage: /promote @{tag} to {role}".into()))?;
    let role = if args.len() >= 3 && args[1].eq_ignore_ascii_case("to") {
        args[2]
    } else {
        args.get(1)
            .copied()
            .ok_or_else(|| AgentHubError::Config("usage: /promote @{tag} to {role}".into()))?
    };

    let (id, agent) = resolve_tag(&ctx.state, tag)?;
    assign_agent_role(&ctx.state, id, role)?;

    let override_text = ctx
        .state
        .role_induction_overrides
        .get(role)
        .map(|e| e.value().clone())
        .unwrap_or_else(|| format!("Fulfill the responsibilities of the {role} role."));

    let system_msg =
        format!("[System]: Your role has been changed to {role}. New behavior: {override_text}.\n");
    let _ = agent.write_stdin(system_msg.as_bytes());

    let _ = ctx.bus_tx.send(BusEvent::RoleAssigned {
        agent_id: id,
        role: role.to_string(),
        by: ctx.issued_by.clone(),
    });

    Ok(format!("promoted @{} to {role}", agent.tag))
}

async fn cmd_demote(ctx: &ModerationContext, args: &[&str]) -> Result<String> {
    ctx.require_modify_roles()?;
    let tag = args
        .first()
        .copied()
        .ok_or_else(|| AgentHubError::Config("usage: /demote @{tag}".into()))?;
    let (id, agent) = resolve_tag(&ctx.state, tag)?;
    assign_agent_role(&ctx.state, id, "Observer")?;

    let _ = ctx.bus_tx.send(BusEvent::RoleAssigned {
        agent_id: id,
        role: "Observer".to_string(),
        by: ctx.issued_by.clone(),
    });

    Ok(format!("demoted @{} to Observer", agent.tag))
}

async fn cmd_addrole(ctx: &ModerationContext, args: &[&str]) -> Result<String> {
    if !modes::admin_commands_enabled(ctx.state.mode()) {
        return Err(AgentHubError::Config(
            "/addrole is only available in Server mode".into(),
        ));
    }
    let name = args
        .first()
        .copied()
        .ok_or_else(|| AgentHubError::Config("usage: /addrole {name} [permissions...]".into()))?;
    if is_builtin_role(name) {
        return Err(AgentHubError::Config(format!(
            "role name conflicts with built-in role: {name}"
        )));
    }
    let perms = if args.len() > 1 {
        parse_permission_names(&args[1..])?
    } else {
        Permissions::VIEW_CHANNEL
    };

    ctx.state.roles.insert(name.to_string(), perms);

    rbac::upsert_custom_role(
        &rbac::roles_json_path(),
        &CustomRoleDefinition {
            name: name.to_string(),
            permissions: rbac::permission_names(perms),
            induction_prompt_override: None,
        },
    )?;

    Ok(format!("added role {name}"))
}

async fn cmd_removerole(ctx: &ModerationContext, args: &[&str]) -> Result<String> {
    if !modes::admin_commands_enabled(ctx.state.mode()) {
        return Err(AgentHubError::Config(
            "/removerole is only available in Server mode".into(),
        ));
    }
    let name = args
        .first()
        .copied()
        .ok_or_else(|| AgentHubError::Config("usage: /removerole {name}".into()))?;
    if is_builtin_role(name) {
        return Err(AgentHubError::Config(format!(
            "cannot delete built-in role: {name}"
        )));
    }
    ctx.state.roles.remove(name);
    ctx.state.role_induction_overrides.remove(name);

    rbac::remove_custom_role(&rbac::roles_json_path(), name)?;

    Ok(format!("removed role {name}"))
}

async fn cmd_mode(ctx: &ModerationContext, args: &[&str]) -> Result<String> {
    let mode_str = args
        .first()
        .copied()
        .ok_or_else(|| AgentHubError::Config("usage: /mode {dm|groupchat|server}".into()))?;
    let new = match mode_str.to_ascii_lowercase().as_str() {
        "dm" | "direct_message" => WorkspaceModeId::Dm,
        "groupchat" | "group_chat" => WorkspaceModeId::GroupChat,
        "server" => WorkspaceModeId::Server,
        other => {
            return Err(AgentHubError::Config(format!("unknown mode: {other}")));
        }
    };

    let change = set_workspace_mode(&ctx.state, new)?;
    let _ = ctx.bus_tx.send(BusEvent::ModeChanged {
        old: change.old.to_repr(),
        new: change.new.to_repr(),
    });

    Ok(format!("mode set to {:?}", change.new))
}

/// Parse `/spawn {driver} [--role {role}] [--tag {custom_tag}]` arguments.
pub fn parse_spawn_args<'a>(args: &'a [&'a str]) -> Result<(&'a str, SpawnOptions)> {
    let driver = args.first().copied().ok_or_else(|| {
        AgentHubError::Config("usage: /spawn {driver} [--role {role}] [--tag {custom_tag}]".into())
    })?;
    let mut options = SpawnOptions::default();
    let mut i = 1;
    while i < args.len() {
        match args[i] {
            "--role" => {
                i += 1;
                let role = args
                    .get(i)
                    .ok_or_else(|| AgentHubError::Config("--role requires a value".into()))?;
                options.role = Some((*role).to_string());
                i += 1;
            }
            "--tag" => {
                i += 1;
                let tag = args
                    .get(i)
                    .ok_or_else(|| AgentHubError::Config("--tag requires a value".into()))?;
                options.tag = Some((*tag).to_string());
                i += 1;
            }
            flag => {
                return Err(AgentHubError::Config(format!("unknown spawn flag: {flag}")));
            }
        }
    }
    Ok((driver, options))
}

async fn cmd_spawn(ctx: &ModerationContext, args: &[&str]) -> Result<String> {
    let (driver, options) = parse_spawn_args(args)?;

    let tag_preview = options.tag.clone().unwrap_or_else(|| format!("{driver}-1"));

    if let Some(conflict) = modes::dm_spawn_conflict(&ctx.state, &tag_preview) {
        return Err(AgentHubError::Config(format!(
            "DM mode already has @{}; kick @{} before spawning @{}",
            conflict.existing_tag, conflict.existing_tag, conflict.new_tag
        )));
    }

    modes::validate_spawn(&ctx.state, ctx.config.max_agents, &tag_preview)?;

    if let Some(caller_id) = ctx.caller_agent_id {
        modes::check_permission(&ctx.state, caller_id, Permissions::SPAWN_AGENTS)?;
    }

    let id = spawn_agent(
        driver,
        &ctx.config,
        Arc::clone(&ctx.state),
        ctx.bus_tx.clone(),
        ctx.db.clone(),
        options.clone(),
    )
    .await?;

    let tag = ctx
        .state
        .agents
        .get(&id)
        .map(|entry| entry.value().tag.clone())
        .unwrap_or(tag_preview);

    Ok(format!("spawned @{tag}"))
}

async fn cmd_spar(ctx: &ModerationContext, line: &str) -> Result<String> {
    use crate::pipeline::{parse_spar_command, SparEngine};

    let spar = parse_spar_command(line)?;
    let engine = SparEngine::new(
        Arc::clone(&ctx.state),
        ctx.bus_tx.clone(),
        &ctx.cwd,
        ctx.session_id,
        (*ctx.config).clone(),
        ctx.db.clone(),
    );
    let result = engine.run(&spar).await?;
    if result.aborted {
        return Ok("[Spar]: Aborted.".into());
    }
    if result.stagnation {
        return Ok(format!(
            "[Spar]: Stagnation detected after {} turns. Last: {}",
            result.turns_completed, result.last_output
        ));
    }
    Ok(format!(
        "[Spar]: Completed {} turns. Last: {}",
        result.turns_completed, result.last_output
    ))
}

async fn cmd_channel(ctx: &ModerationContext, args: &[&str]) -> Result<String> {
    let sub = args.first().copied().unwrap_or("list");
    match sub {
        "create" => {
            let name = args
                .get(1)
                .copied()
                .ok_or_else(|| AgentHubError::Config("usage: /channel create <name>".into()))?;
            modes::create_channel(&ctx.state, name)?;
            Ok(format!("[Server]: Channel #{name} created."))
        }
        "delete" => {
            let name = args
                .get(1)
                .copied()
                .ok_or_else(|| AgentHubError::Config("usage: /channel delete <name>".into()))?;
            modes::delete_channel(&ctx.state, name)?;
            Ok(format!("[Server]: Channel #{name} deleted."))
        }
        "assign" => {
            let tag = args.get(1).copied().ok_or_else(|| {
                AgentHubError::Config("usage: /channel assign @{tag} [to] <channel>".into())
            })?;
            let channel = if args.get(2) == Some(&"to") {
                args.get(3).copied()
            } else {
                args.get(2).copied()
            }
            .ok_or_else(|| {
                AgentHubError::Config("usage: /channel assign @{tag} [to] <channel>".into())
            })?;
            let (id, agent) = resolve_tag(&ctx.state, tag)?;
            modes::assign_agent_to_channel(&ctx.state, channel, id)?;
            Ok(format!(
                "[Server]: Assigned @{} to #{}.",
                agent.tag, channel
            ))
        }
        "remove" => {
            let tag = args.get(1).copied().ok_or_else(|| {
                AgentHubError::Config("usage: /channel remove @{tag} [from] <channel>".into())
            })?;
            let channel = if args.get(2) == Some(&"from") {
                args.get(3).copied()
            } else {
                args.get(2).copied()
            }
            .ok_or_else(|| {
                AgentHubError::Config("usage: /channel remove @{tag} [from] <channel>".into())
            })?;
            let (id, agent) = resolve_tag(&ctx.state, tag)?;
            modes::remove_agent_from_channel(&ctx.state, channel, id)?;
            Ok(format!(
                "[Server]: Removed @{} from #{}.",
                agent.tag, channel
            ))
        }
        "list" => {
            if !modes::channels_enabled(modes::get_mode(&ctx.state)) {
                return Err(AgentHubError::Config(
                    "channels are only available in Server mode (try /mode server)".into(),
                ));
            }
            if ctx.state.channels.is_empty() {
                return Ok("[Server]: No channels yet. Use /channel create <name>.".into());
            }
            let mut lines = vec!["[Server]: Channels:".into()];
            for entry in ctx.state.channels.iter() {
                let members: Vec<String> = entry
                    .value()
                    .iter()
                    .filter_map(|id| ctx.state.agents.get(id).map(|a| format!("@{}", a.tag)))
                    .collect();
                let member_text = if members.is_empty() {
                    "(empty)".into()
                } else {
                    members.join(", ")
                };
                lines.push(format!("  #{} — {member_text}", entry.key()));
            }
            Ok(lines.join("\n"))
        }
        other => Err(AgentHubError::Config(format!(
            "unknown /channel subcommand: {other} (try create, delete, assign, remove, list)"
        ))),
    }
}

async fn cmd_setprompt(ctx: &ModerationContext, args: &[&str]) -> Result<String> {
    let tag = args
        .first()
        .copied()
        .ok_or_else(|| AgentHubError::Config("usage: /setprompt @{tag} {prompt...}".into()))?;
    let prompt = args
        .get(1..)
        .ok_or_else(|| AgentHubError::Config("usage: /setprompt @{tag} {prompt...}".into()))?;
    if prompt.is_empty() {
        return Err(AgentHubError::Config("prompt text required".into()));
    }
    let (_, agent) = resolve_tag(&ctx.state, tag)?;
    let payload = format!("{}\n", prompt.join(" "));
    agent.write_stdin(payload.as_bytes())?;
    Ok(format!("set prompt for @{}", agent.tag))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_duration_units() {
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("2h").unwrap(), Duration::from_secs(7200));
        assert!(parse_duration("bad").is_err());
    }
}
