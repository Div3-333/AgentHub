//! Keyboard event routing (Part 15.2) and slash-command dispatch (§8.3).

use std::future::Future;

use agenthub_core::bus::{is_racing_input, BusEvent, MessageTarget};
use agenthub_core::server::moderation;
use chrono::Utc;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, Focus, Overlay};

/// Slash commands advertised by `/help` and Tab completion (blueprint §8.3).
pub const SLASH_COMMANDS: &[&str] = &[
    "/help",
    "/mute",
    "/unmute",
    "/deafen",
    "/undeafen",
    "/timeout",
    "/kick",
    "/ban",
    "/promote",
    "/demote",
    "/addrole",
    "/removerole",
    "/mode",
    "/spawn",
    "/setprompt",
    "/snapshot",
    "/undo",
    "/channel",
    "/spar",
];

/// Full help shown by `/help` and the F1 overlay (slash §8.3 + keys §15.2).
pub const TUI_HELP: &str = r#"AgentHub — Commands & Keys

Slash commands (blueprint §8.3):

  /help                   List commands and key bindings (F1)
  /mute @tag              Hide agent output in chat
  /unmute @tag            Restore chat visibility
  /deafen @tag            Stop broadcast delivery to agent
  /undeafen @tag          Restore broadcast delivery
  /timeout @tag 30s       Suspend agent (30s, 5m, 2h)
  /kick @tag [reason]     Terminate agent process
  /ban @tag [reason]      Kick and block driver for session (Server)
  /promote @tag to Role   Assign role from ServerState.roles
  /demote @tag            Revert agent to Observer role
  /addrole Name perms…    Create custom role (Server)
  /removerole Name        Delete custom role (Server)
  /mode dm|groupchat|server
                          Change workspace mode
  /spawn driver [--role R] [--tag T]
                          Start agent via driver
  /setprompt @tag text    Inject system prompt into agent PTY
  /snapshot               Manual VFS workspace snapshot
  /undo                   Revert workspace to latest snapshot (Y/n confirm)
  /undo --yes             Revert immediately without confirmation
  /channel create|delete|assign|remove|list
                          Manage Server-mode channels
  /spar @a as R vs @b as R [--turns N] [--goal "…"]
                          Autonomous two-agent sparring session

Key bindings (Part 15.2):

  j / Down              Scroll chat down
  k / Up                Scroll chat up
  PgDn / PgUp           Scroll one page
  G                     Jump to latest message (any time)
  F4                    Toggle chat scroll vs input typing
  Ctrl+/                Search chat history
  Esc                   Cancel overlay / search / racing
  Enter                 Send message or run slash command
  Ctrl+Enter            Activate LLM Racing (multi @tag)
  Tab                   Autocomplete @tags and /commands
  Up / Down             Input history (when input focused)
  Ctrl+L                Clear input box
  F1                    Toggle this help
  F2                    Cycle mode (DM → GroupChat → Server)
  F3                    Manual VFS snapshot (/snapshot)
  Ctrl+Z                Revert to last snapshot (/undo)
  Ctrl+S                Save chat history (path prompt)
  Ctrl+Q                Quit (confirmation)
  F5                    Spawn agent dialog
  F6                    Agent list (kick / mute / role)
  Ctrl+R                Activate LLM Racing
"#;

/// Alias for chat `/help` output (same text as F1).
pub const SLASH_HELP: &str = TUI_HELP;

/// Mouse click: focus chat (j/k scroll) or input (typing / slash commands).
pub fn handle_mouse_click(app: &mut App, col: u16, row: u16) {
    match app.hit_test(col, row) {
        Some(crate::app::PaneTarget::Chat) => {
            app.focus = Focus::Chat;
            app.status_message = "Chat scroll (j/k/G) — F4 back to input".into();
        }
        Some(crate::app::PaneTarget::Input) => {
            app.focus = Focus::Input;
            if app.search_mode {
                app.search_mode = false;
                app.search_query.clear();
            }
            app.status_message.clear();
        }
        None => {}
    }
}

/// Handle a key press; returns `true` when the app should exit.
///
/// Callers must pass only [`KeyEventKind::Press`] events (see `run_app` in `lib.rs`).
/// Windows terminals also emit `Release` events that would otherwise duplicate input.
pub fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    let viewport = app.chat_viewport_rows();

    if app.overlay == Overlay::QuitConfirm {
        return handle_quit_confirm(app, key);
    }

    if app.overlay == Overlay::RevertConfirm {
        return handle_revert_confirm(app, key);
    }

    if app.overlay == Overlay::Racing {
        return handle_racing_keys(app, key);
    }

    if app.overlay == Overlay::Help {
        if key.code == KeyCode::F(1) || key.code == KeyCode::Esc {
            app.overlay = Overlay::None;
        }
        return false;
    }

    if matches!(
        app.overlay,
        Overlay::SpawnDialog | Overlay::AgentList | Overlay::SavePath
    ) {
        return handle_overlay_keys(app, key);
    }

    if app.search_mode {
        return handle_search_keys(app, key, viewport);
    }

    if key.code == KeyCode::Esc {
        app.cancel_overlay();
        return false;
    }

    match key.code {
        KeyCode::F(1) => {
            app.overlay = if app.overlay == Overlay::Help {
                Overlay::None
            } else {
                Overlay::Help
            };
        }
        KeyCode::F(2) => {
            let next = app.workspace_mode.cycle();
            if app.core.is_some() {
                let slug = next.mode_slug();
                route_slash_command(app, &format!("/mode {slug}"));
            } else {
                app.workspace_mode = next;
                app.status_message = format!("Mode → {}", app.workspace_mode.label());
            }
        }
        KeyCode::F(3) => {
            route_slash_command(app, "/snapshot");
        }
        KeyCode::F(4) => {
            app.focus = match app.focus {
                Focus::Input => Focus::Chat,
                Focus::Chat => Focus::Input,
            };
            app.status_message = match app.focus {
                Focus::Chat => "Chat scroll (j/k/G) — F4 back to input".into(),
                Focus::Input => "Input focus — F4 for chat scroll".into(),
            };
        }
        KeyCode::F(5) => {
            app.overlay = Overlay::SpawnDialog;
            app.spawn_buffer.clear();
        }
        KeyCode::F(6) => {
            app.overlay = Overlay::AgentList;
            app.agent_list_selected = 0;
        }
        KeyCode::Char('z') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            begin_revert_confirm(app);
        }
        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.overlay = Overlay::SavePath;
        }
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            submit_racing_prompt(app);
        }
        KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.overlay = Overlay::QuitConfirm;
        }
        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.focus == Focus::Input {
                app.input_buffer.clear();
                app.input_cursor = 0;
            }
        }
        KeyCode::Char('/') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.search_mode = true;
            app.search_query.clear();
        }
        KeyCode::Char('G') if app.overlay == Overlay::None && !app.search_mode => {
            app.scroll_chat_to_bottom(viewport);
        }
        KeyCode::Char('j') if app.focus == Focus::Chat && app.overlay == Overlay::None => {
            app.scroll_chat_down(1, viewport);
        }
        KeyCode::Char('k') if app.focus == Focus::Chat && app.overlay == Overlay::None => {
            app.scroll_chat_up(1);
        }
        KeyCode::Up => handle_up_arrow(app, viewport),
        KeyCode::Down => handle_down_arrow(app, viewport),
        KeyCode::PageDown => {
            app.scroll_chat_down(viewport, viewport);
        }
        KeyCode::PageUp => {
            app.scroll_chat_up(viewport);
        }
        _ if app.focus == Focus::Input => {
            handle_input_keys(app, key);
        }
        _ => {}
    }

    false
}

/// §15.2 navigation: Up scrolls chat unless input history should take precedence.
fn handle_up_arrow(app: &mut App, _viewport: usize) {
    if app.focus == Focus::Chat {
        app.scroll_chat_up(1);
        return;
    }
    if !app.input_buffer.is_empty() || !app.input_history.is_empty() {
        input_history_prev(app);
        return;
    }
    app.scroll_chat_up(1);
}

/// §15.2: Down scrolls chat when the input box is empty; otherwise input history.
fn handle_down_arrow(app: &mut App, viewport: usize) {
    if app.focus == Focus::Chat {
        app.scroll_chat_down(1, viewport);
        return;
    }
    if app.input_history_idx.is_some() {
        input_history_next(app);
        return;
    }
    if app.input_buffer.is_empty() {
        app.scroll_chat_down(1, viewport);
        return;
    }
    input_history_next(app);
}

fn handle_agent_list_keys(app: &mut App, key: KeyEvent) -> bool {
    let count = app.agent_list_entries().len();
    match key.code {
        KeyCode::Esc => app.cancel_overlay(),
        KeyCode::Char('j') | KeyCode::Down => {
            if count > 0 {
                app.agent_list_selected = (app.agent_list_selected + 1) % count;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if count > 0 {
                app.agent_list_selected =
                    app.agent_list_selected.checked_sub(1).unwrap_or(count - 1);
            }
        }
        KeyCode::Char('m') => agent_list_slash(app, "mute"),
        KeyCode::Char('K') => agent_list_slash(app, "kick"),
        KeyCode::Char('p') => agent_list_slash(app, "promote"),
        KeyCode::Char('d') => agent_list_slash(app, "demote"),
        _ => {}
    }
    false
}

fn agent_list_slash(app: &mut App, cmd: &str) {
    let entries = app.agent_list_entries();
    let Some(agent) = entries.get(app.agent_list_selected) else {
        return;
    };
    let tag = agent.tag.clone();
    let line = match cmd {
        "promote" => format!("/promote @{tag} to Builder"),
        "demote" => format!("/demote @{tag}"),
        other => format!("/{other} @{tag}"),
    };
    route_slash_command(app, &line);
}

fn handle_overlay_keys(app: &mut App, key: KeyEvent) -> bool {
    if app.overlay == Overlay::AgentList {
        return handle_agent_list_keys(app, key);
    }

    match key.code {
        KeyCode::Esc => app.cancel_overlay(),
        KeyCode::Enter => match app.overlay {
            Overlay::SpawnDialog => {
                let line = if app.spawn_buffer.starts_with('/') {
                    app.spawn_buffer.clone()
                } else {
                    format!("/spawn {}", app.spawn_buffer.trim())
                };
                app.spawn_buffer.clear();
                app.overlay = Overlay::None;
                route_slash_command(app, &line);
            }
            Overlay::SavePath => {
                let path = app.save_path_buffer.clone();
                app.overlay = Overlay::None;
                match app.save_chat_to_path(&path) {
                    Ok(bytes) => {
                        app.apply_command_result(format!("Chat saved to {path} ({bytes} bytes)"));
                    }
                    Err(e) => app.apply_command_result(format!("Save failed: {e}")),
                }
            }
            _ => {}
        },
        KeyCode::Backspace => match app.overlay {
            Overlay::SpawnDialog => {
                if !app.spawn_buffer.is_empty() {
                    app.spawn_buffer.pop();
                }
            }
            Overlay::SavePath if !app.save_path_buffer.is_empty() => {
                app.save_path_buffer.pop();
            }
            _ => {}
        },
        KeyCode::Char(c) => match app.overlay {
            Overlay::SpawnDialog => app.spawn_buffer.push(c),
            Overlay::SavePath => app.save_path_buffer.push(c),
            _ => {}
        },
        _ => {}
    }
    false
}

fn handle_search_keys(app: &mut App, key: KeyEvent, viewport: usize) -> bool {
    match key.code {
        KeyCode::Esc => {
            app.search_mode = false;
            app.search_query.clear();
        }
        KeyCode::Char('j') | KeyCode::Down => app.scroll_chat_down(1, viewport),
        KeyCode::Char('k') | KeyCode::Up => app.scroll_chat_up(1),
        KeyCode::PageDown => app.scroll_chat_down(viewport, viewport),
        KeyCode::PageUp => app.scroll_chat_up(viewport),
        KeyCode::Char('g') | KeyCode::Char('G') => app.scroll_chat_to_bottom(viewport),
        KeyCode::Backspace => {
            app.search_query.pop();
        }
        KeyCode::Char(c) => app.search_query.push(c),
        _ => {}
    }
    false
}

fn handle_racing_keys(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => app.cancel_racing(),
        KeyCode::Left => app.racing_select_prev(),
        KeyCode::Right => app.racing_select_next(),
        KeyCode::Enter => app.confirm_racing_winner(),
        _ => {}
    }
    false
}

fn handle_quit_confirm(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
            app.should_quit = true;
            return true;
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.overlay = Overlay::None;
        }
        KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.should_quit = true;
            return true;
        }
        _ => {}
    }
    false
}

/// Ctrl+Enter / Ctrl+R: send multi-@ input as an LLM race (blueprint §15.2).
fn submit_racing_prompt(app: &mut App) {
    let text = app.input_buffer.trim().to_string();
    if text.is_empty() {
        return;
    }
    if !is_racing_input(&text) {
        app.status_message = "LLM Racing needs at least two @tags before the prompt.".into();
        return;
    }
    if let Some(bridge) = app.core.clone() {
        let _ = bridge.bus_tx.send(BusEvent::UserMessage {
            content: text.clone(),
            timestamp: Utc::now(),
            target: MessageTarget::Broadcast,
        });
        app.input_history.push(text);
        app.input_history_idx = None;
        return;
    }
    app.on_submit(text);
}

fn handle_input_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Enter => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                submit_racing_prompt(app);
                app.input_buffer.clear();
                app.input_cursor = 0;
            } else {
                let text = app.input_buffer.clone();
                app.input_buffer.clear();
                app.input_cursor = 0;
                if text.starts_with('/') {
                    route_slash_command(app, &text);
                    if !text.is_empty() {
                        app.input_history.push(text);
                        app.input_history_idx = None;
                    }
                } else {
                    app.on_submit(text);
                }
            }
        }
        KeyCode::Backspace => {
            if app.input_cursor > 0 {
                app.input_cursor -= 1;
                app.input_buffer.remove(app.input_cursor);
            }
        }
        KeyCode::Delete => {
            if app.input_cursor < app.input_buffer.len() {
                app.input_buffer.remove(app.input_cursor);
            }
        }
        KeyCode::Left => {
            app.input_cursor = app.input_cursor.saturating_sub(1);
        }
        KeyCode::Right => {
            if app.input_cursor < app.input_buffer.len() {
                app.input_cursor += 1;
            }
        }
        KeyCode::Home => app.input_cursor = 0,
        KeyCode::End => app.input_cursor = app.input_buffer.len(),
        KeyCode::Tab => tab_complete(app),
        KeyCode::Char(c) => {
            app.input_buffer.insert(app.input_cursor, c);
            app.input_cursor += 1;
        }
        _ => {}
    }
}

/// Detect `/commands` and route to core moderation / VFS (blueprint §8.3).
pub fn route_slash_command(app: &mut App, line: &str) {
    let trimmed = line.trim();
    if !trimmed.starts_with('/') {
        return;
    }

    if trimmed.eq_ignore_ascii_case("/help") {
        app.apply_command_result(SLASH_HELP.to_string());
        return;
    }

    if is_interactive_undo(trimmed) {
        begin_revert_confirm(app);
        return;
    }

    if let Some(bridge) = app.core.clone() {
        match block_on(dispatch_slash_core(&bridge, trimmed)) {
            Ok(msg) => app.apply_command_result(msg),
            Err(e) => app.apply_command_result(format!("Error: {e}")),
        }
        return;
    }

    route_slash_command_local(app, trimmed);
}

async fn dispatch_slash_core(
    bridge: &crate::app::CoreBridge,
    line: &str,
) -> agenthub_core::Result<String> {
    if let Some(msg) = moderation::try_handle_slash_command(
        line,
        &bridge.db,
        &bridge.config,
        &bridge.cwd,
        bridge.session_id,
        Some(&bridge.bus_tx),
        Some(&bridge.moderation.state),
    )
    .await?
    {
        return Ok(msg);
    }

    moderation::execute_command(&bridge.moderation, line).await
}

fn route_slash_command_local(app: &mut App, line: &str) {
    let cmd = line.split_whitespace().next().unwrap_or(line);
    if is_interactive_undo(line) {
        app.apply_command_result(
            "Revert requires a live AgentHub session with snapshots. Run the `agenthub` binary."
                .into(),
        );
        return;
    }
    app.apply_command_result(format!(
        "Command {cmd} requires a live AgentHub session. Run the `agenthub` binary (not the preview UI)."
    ));
}

fn is_interactive_undo(line: &str) -> bool {
    let parts: Vec<&str> = line.split_whitespace().collect();
    parts
        .first()
        .is_some_and(|cmd| cmd.eq_ignore_ascii_case("/undo"))
        && !parts.iter().any(|p| *p == "--yes" || *p == "-y")
}

pub fn begin_revert_confirm(app: &mut App) {
    let Some(bridge) = app.core.clone() else {
        app.apply_command_result(
            "Revert requires a live AgentHub session. Run the `agenthub` binary.".into(),
        );
        return;
    };

    match block_on(async { agenthub_core::vfs::preview_revert(&bridge.db.pool, &bridge.cwd).await })
    {
        Ok(preview) => {
            app.status_message = agenthub_core::vfs::revert_confirmation_message(
                preview.snapshot_id,
                preview.file_count,
                &preview.elapsed_label,
            );
            app.revert_dialog = Some(crate::app::RevertDialogState {
                preview,
                step: crate::app::RevertDialogStep::ConfirmRevert,
            });
            app.overlay = Overlay::RevertConfirm;
        }
        Err(e) => app.apply_command_result(format!("Error: {e}")),
    }
}

fn handle_revert_confirm(app: &mut App, key: KeyEvent) -> bool {
    let Some(dialog) = app.revert_dialog.clone() else {
        app.overlay = Overlay::None;
        return false;
    };

    match key.code {
        KeyCode::Esc => {
            app.overlay = Overlay::None;
            app.revert_dialog = None;
            app.status_message = "Revert cancelled.".into();
        }
        KeyCode::Char('n') | KeyCode::Char('N') => match dialog.step {
            crate::app::RevertDialogStep::ConfirmRevert => {
                app.overlay = Overlay::None;
                app.revert_dialog = None;
                app.status_message = "Revert cancelled.".into();
            }
            crate::app::RevertDialogStep::ConfirmDeleteNewFiles => {
                finish_revert(app, false);
            }
        },
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => match dialog.step {
            crate::app::RevertDialogStep::ConfirmRevert => {
                if dialog.preview.new_files_count > 0 {
                    app.status_message = agenthub_core::vfs::delete_new_files_message(
                        dialog.preview.new_files_count,
                    );
                    app.revert_dialog = Some(crate::app::RevertDialogState {
                        preview: dialog.preview,
                        step: crate::app::RevertDialogStep::ConfirmDeleteNewFiles,
                    });
                } else {
                    finish_revert(app, false);
                }
            }
            crate::app::RevertDialogStep::ConfirmDeleteNewFiles => {
                finish_revert(app, true);
            }
        },
        _ => {}
    }
    false
}

fn finish_revert(app: &mut App, delete_new_files: bool) {
    let Some(bridge) = app.core.clone() else {
        app.overlay = Overlay::None;
        app.revert_dialog = None;
        return;
    };

    match block_on(agenthub_core::vfs::execute_revert(
        &bridge.db,
        &bridge.config,
        &bridge.cwd,
        delete_new_files,
        Some(&bridge.bus_tx),
        Some(&bridge.moderation.state),
    )) {
        Ok(msg) => app.apply_command_result(msg),
        Err(e) => app.apply_command_result(format!("Error: {e}")),
    }
    app.overlay = Overlay::None;
    app.revert_dialog = None;
}

fn block_on<F: Future>(future: F) -> F::Output {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        return tokio::task::block_in_place(|| handle.block_on(future));
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime for slash commands")
        .block_on(future)
}

fn input_history_prev(app: &mut App) {
    if app.input_history.is_empty() {
        return;
    }
    let idx = match app.input_history_idx {
        None => app.input_history.len() - 1,
        Some(0) => 0,
        Some(i) => i - 1,
    };
    app.input_history_idx = Some(idx);
    app.input_buffer = app.input_history[idx].clone();
    app.input_cursor = app.input_buffer.len();
}

fn input_history_next(app: &mut App) {
    let Some(pos) = app.input_history_idx else {
        return;
    };
    if pos + 1 >= app.input_history.len() {
        app.input_history_idx = None;
        app.input_buffer.clear();
        app.input_cursor = 0;
    } else {
        let next = pos + 1;
        app.input_history_idx = Some(next);
        app.input_buffer = app.input_history[next].clone();
        app.input_cursor = app.input_buffer.len();
    }
}

fn tab_complete(app: &mut App) {
    let word_start = app.input_buffer[..app.input_cursor]
        .rfind(|c: char| c.is_whitespace())
        .map(|i| i + 1)
        .unwrap_or(0);
    let prefix = &app.input_buffer[word_start..app.input_cursor];
    // Avoid replacing "/" with "/addrole" on lone slash or @ prefix.
    if prefix.len() < 2 {
        return;
    }
    let mut candidates: Vec<String> = app
        .agent_tags()
        .into_iter()
        .map(|t| format!("@{t}"))
        .chain(SLASH_COMMANDS.iter().map(|s| (*s).to_string()))
        .filter(|c| c.starts_with(prefix))
        .collect();
    candidates.sort();
    candidates.dedup();
    if let Some(choice) = candidates.first() {
        app.input_buffer
            .replace_range(word_start..app.input_cursor, choice);
        app.input_cursor = word_start + choice.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ChatSender;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    #[test]
    fn slash_help_lists_commands() {
        let mut app = App::new("dark");
        route_slash_command(&mut app, "/help");
        let last = app.chat_lines.last().expect("chat line");
        for needle in [
            "/help",
            "/mute",
            "/ban",
            "/spar",
            "/snapshot",
            "/undo",
            "F1",
        ] {
            assert!(
                last.text.contains(needle),
                "help missing {needle}: {}",
                last.text
            );
        }
        assert_eq!(last.text, SLASH_HELP);
    }

    #[test]
    fn f1_help_overlay_matches_slash_help() {
        assert_eq!(crate::app::HELP_TEXT, SLASH_HELP);
    }

    #[test]
    fn slash_spawn_types_into_input_not_search_mode() {
        let mut app = App::new("dark");
        for c in "/spawn".chars() {
            handle_key(&mut app, key(KeyCode::Char(c)));
        }
        assert!(!app.search_mode);
        assert_eq!(app.input_buffer, "/spawn");
    }

    #[test]
    fn ctrl_slash_opens_search_mode() {
        let mut app = App::new("dark");
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('/'), KeyModifiers::CONTROL),
        );
        assert!(app.search_mode);
    }

    #[test]
    fn input_focus_typing_j_inserts_without_vim_scroll() {
        let mut app = App::new("dark");
        assert_eq!(app.focus, Focus::Input);
        let scroll_before = app.chat_scroll;
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.input_buffer, "j");
        assert_eq!(app.chat_scroll, scroll_before);
    }

    #[test]
    fn tab_complete_fills_slash_command() {
        let mut app = App::new("dark");
        app.input_buffer = "/mu".into();
        app.input_cursor = app.input_buffer.len();
        tab_complete(&mut app);
        assert!(
            app.input_buffer.starts_with("/mute"),
            "got {}",
            app.input_buffer
        );
    }

    #[test]
    fn racing_arrow_select_and_enter_confirm() {
        let mut app = App::new("dark");
        app.activate_racing("@gemini-1 @claude-1 @codex-1 write code");
        assert_eq!(app.overlay, Overlay::Racing);
        assert_eq!(app.racing_selected, 0);

        handle_key(&mut app, key(KeyCode::Right));
        assert_eq!(app.racing_selected, 1);

        let lines_before = app.chat_lines.len();
        handle_key(&mut app, key(KeyCode::Enter));
        assert_eq!(app.overlay, Overlay::None);
        assert!(app.chat_lines.len() > lines_before);
        assert!(app.racing_panes.is_empty());
    }

    #[test]
    fn racing_esc_discards_without_chat_insert() {
        let mut app = App::new("dark");
        app.activate_racing("@gemini-1 @claude-1 test");
        let lines_before = app.chat_lines.len();
        handle_key(&mut app, key(KeyCode::Esc));
        assert_eq!(app.overlay, Overlay::None);
        assert_eq!(app.chat_lines.len(), lines_before);
    }

    #[test]
    fn state_machine_reports_search_mode() {
        use crate::app::AppState;
        let mut app = App::new("dark");
        assert_eq!(app.state(), AppState::Normal);
        app.search_mode = true;
        assert_eq!(app.state(), AppState::Search);
    }

    fn key_mod(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: mods,
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    fn fill_chat_for_scroll(app: &mut App) {
        app.update_terminal_size(80, 24);
        for i in 0..60 {
            app.push_chat(ChatSender::System, format!("scroll line {i}"));
        }
        let viewport = app.chat_viewport_rows();
        app.chat_scroll = app.max_chat_scroll(viewport);
        app.chat_scroll = 0;
    }

    #[test]
    fn capital_g_jumps_to_bottom_from_input_focus() {
        let mut app = App::new("dark");
        fill_chat_for_scroll(&mut app);
        assert_eq!(app.focus, Focus::Input);
        handle_key(&mut app, key(KeyCode::Char('G')));
        assert_eq!(
            app.chat_scroll,
            app.max_chat_scroll(app.chat_viewport_rows())
        );
    }

    #[test]
    fn f4_toggles_chat_scroll_focus() {
        let mut app = App::new("dark");
        fill_chat_for_scroll(&mut app);
        handle_key(&mut app, key(KeyCode::F(4)));
        assert_eq!(app.focus, Focus::Chat);
        let viewport = app.chat_viewport_rows();
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert!(app.chat_scroll > 0);
        handle_key(&mut app, key(KeyCode::F(4)));
        assert_eq!(app.focus, Focus::Input);
    }

    #[test]
    fn tab_on_lone_slash_does_not_replace_buffer() {
        let mut app = App::new("dark");
        app.input_buffer = "/".into();
        app.input_cursor = 1;
        tab_complete(&mut app);
        assert_eq!(app.input_buffer, "/");
    }

    #[test]
    fn part15_navigation_keys_scroll_chat() {
        let mut app = App::new("dark");
        fill_chat_for_scroll(&mut app);
        app.focus = Focus::Chat;
        let viewport = app.chat_viewport_rows();
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert!(app.chat_scroll > 0);
        app.chat_scroll = 0;
        handle_key(&mut app, key(KeyCode::PageDown));
        assert!(app.chat_scroll > 0);
        app.scroll_chat_to_bottom(viewport);
        let at_bottom = app.chat_scroll;
        handle_key(&mut app, key(KeyCode::Char('k')));
        assert!(app.chat_scroll < at_bottom);
        handle_key(&mut app, key(KeyCode::Char('G')));
        assert_eq!(app.chat_scroll, app.max_chat_scroll(viewport));
    }

    #[test]
    fn part15_empty_input_down_scrolls_chat_up_uses_history() {
        let mut app = App::new("dark");
        fill_chat_for_scroll(&mut app);
        let viewport = app.chat_viewport_rows();
        handle_key(&mut app, key(KeyCode::Down));
        assert!(app.chat_scroll > 0);
        app.chat_scroll = 0;
        app.input_history = vec!["prior".into()];
        app.input_buffer.clear();
        handle_key(&mut app, key(KeyCode::Up));
        assert_eq!(app.input_buffer, "prior");
        app.input_buffer.clear();
        app.input_history_idx = None;
        app.chat_scroll = 0;
        handle_key(&mut app, key(KeyCode::Down));
        assert!(
            app.chat_scroll > 0,
            "empty input Down scrolls chat (viewport={viewport})"
        );
    }

    #[test]
    fn part15_ctrl_l_clears_input() {
        let mut app = App::new("dark");
        app.input_buffer = "hello".into();
        app.input_cursor = 5;
        handle_key(&mut app, key_mod(KeyCode::Char('l'), KeyModifiers::CONTROL));
        assert!(app.input_buffer.is_empty());
    }

    #[test]
    fn part15_search_mode_esc_clears() {
        let mut app = App::new("dark");
        app.search_mode = true;
        app.search_query = "gemini".into();
        handle_key(&mut app, key(KeyCode::Esc));
        assert!(!app.search_mode);
        assert!(app.search_query.is_empty());
    }

    #[test]
    fn part15_f6_agent_list_mute_routes_command() {
        let mut app = App::new("dark");
        app.overlay = Overlay::AgentList;
        app.agent_list_selected = 0;
        let before = app.chat_lines.len();
        handle_key(&mut app, key(KeyCode::Char('m')));
        assert!(app.chat_lines.len() > before);
        assert!(app.chat_lines.last().unwrap().text.contains("/mute"));
    }

    #[test]
    fn part15_esc_clears_spar_and_pipeline() {
        let mut app = App::new("dark");
        app.spar_active = true;
        app.pipeline = Some(crate::components::pipeline_viz::pipeline_from_started(
            "@test run | > echo",
        ));
        handle_key(&mut app, key(KeyCode::Esc));
        assert!(!app.spar_active);
        assert!(app.pipeline.is_none());
    }

    fn live_app_with_core() -> crate::app::App {
        use std::sync::Arc;

        use agenthub_core::bus::BUS_CAPACITY;
        use agenthub_core::config::AgentHubConfig;
        use agenthub_core::db::DbClient;
        use agenthub_core::server::moderation::ModerationContext;
        use agenthub_core::server::modes::{set_mode, WorkspaceModeId};
        use agenthub_core::server::ServerState;
        use tokio::sync::broadcast;
        use uuid::Uuid;

        let rt = tokio::runtime::Runtime::new().expect("test runtime");
        rt.block_on(async {
            let workdir = tempfile::tempdir().expect("workdir");
            let cwd = workdir.path().to_path_buf();
            std::mem::forget(workdir);
            let file = tempfile::NamedTempFile::new().expect("temp db");
            let url = format!("sqlite://{}", file.path().display());
            let db = Arc::new(DbClient::init_pool(&url).await.expect("db pool"));
            db.run_migrations().await.expect("migrate");
            let state = Arc::new(ServerState::new());
            set_mode(&state, WorkspaceModeId::GroupChat).expect("mode");
            let (bus_tx, _) = broadcast::channel(BUS_CAPACITY);
            let config = Arc::new(AgentHubConfig::default());
            let session_id = Uuid::new_v4();
            let mut app = crate::app::App::new_live("dark");
            app.set_core_bridge(crate::app::CoreBridge {
                moderation: Arc::new(ModerationContext {
                    state: Arc::clone(&state),
                    config: Arc::clone(&config),
                    db: Some(Arc::clone(&db)),
                    bus_tx: bus_tx.clone(),
                    session_id,
                    cwd: cwd.clone(),
                    issued_by: "user".into(),
                    caller_agent_id: None,
                }),
                db,
                config,
                cwd,
                session_id,
                bus_tx,
            });
            app
        })
    }

    #[test]
    fn part15_f2_mode_live_core() {
        let mut app = live_app_with_core();
        let before = app.chat_lines.len();
        handle_key(&mut app, key(KeyCode::F(2)));
        assert!(
            app.chat_lines.len() > before || app.status_message.to_lowercase().contains("mode"),
            "F2 should route /mode through core"
        );
    }

    #[test]
    fn part15_f3_snapshot_live_core() {
        let mut app = live_app_with_core();
        let before = app.chat_lines.len();
        handle_key(&mut app, key(KeyCode::F(3)));
        assert!(
            app.chat_lines.len() > before,
            "F3 snapshot should produce system output via core"
        );
    }

    #[test]
    fn part15_ctrl_z_undo_live_core() {
        let mut app = live_app_with_core();
        handle_key(&mut app, key_mod(KeyCode::Char('z'), KeyModifiers::CONTROL));
        let last = app.chat_lines.last().expect("undo result");
        assert!(
            last.text.to_lowercase().contains("snapshot")
                || last.text.to_lowercase().contains("revert")
                || last.text.to_lowercase().contains("undo")
                || last.text.to_lowercase().contains("vfs"),
            "Ctrl+Z should invoke VFS /undo: {}",
            last.text
        );
    }

    #[test]
    fn part15_ctrl_enter_racing_live_core_no_demo_overlay() {
        let mut app = live_app_with_core();
        app.agents.push(crate::app::AgentEntry {
            id: None,
            tag: "gemini-1".into(),
            role: "Builder".into(),
            status: crate::app::AgentStatus::Idle,
        });
        app.agents.push(crate::app::AgentEntry {
            id: None,
            tag: "claude-1".into(),
            role: "Reviewer".into(),
            status: crate::app::AgentStatus::Idle,
        });
        app.input_buffer = "@gemini-1 @claude-1 write auth".into();
        app.input_cursor = app.input_buffer.len();
        handle_key(&mut app, key_mod(KeyCode::Enter, KeyModifiers::CONTROL));
        assert_eq!(
            app.overlay,
            Overlay::None,
            "live core must not open demo racing UI"
        );
        assert!(app.input_buffer.is_empty());
        assert!(app
            .input_history
            .last()
            .is_some_and(|s| s.contains("@gemini-1")));
    }
}
