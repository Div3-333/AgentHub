//! Subagent process capture (blueprint §5.4 / Phase 11).
//!
//! When an agent CLI spawns its own children, AgentHub registers them as distinct
//! participants tagged `@{parent_tag}-sub-{n}` with the [`SUBAGENT_ROLE`] role.
//!
//! ## Backends
//!
//! | Platform | Preferred | Active today |
//! |----------|-----------|--------------|
//! | Linux    | eBPF (`sched_process_exec` via `aya`, feature `ebpf`) | Polling until eBPF loads |
//! | macOS | `sysinfo` parent-PID scan every [`POLL_INTERVAL_MS`] | Polling |
//! | Windows | Tool Help snapshot (fallback: `sysinfo`) | Polling |
//!
//! Short-lived children may be missed on non-eBPF paths (see `docs/USER_GUIDE.md`).
//!
//! ## Linux eBPF (future)
//!
//! Enable with `cargo build -p agenthub-core --features full,ebpf` once the kernel
//! program is wired. Until then, [`init_subagent_backend`] logs `eBPF not loaded` and
//! [`subagent_watcher_task`] uses the polling fallback (still meets the 500ms DoD).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
#[cfg(target_os = "linux")]
use std::sync::Once;
use std::time::Duration;

use chrono::Utc;
use dashmap::DashSet;
#[cfg(not(windows))]
use sysinfo::{Pid, ProcessesToUpdate, System};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::bus::BusEvent;
use crate::pty::AgentPty;
use crate::server::modes::sync_agent_pty;
use crate::server::rbac::Permissions;
use crate::server::ServerState;

/// Built-in RBAC role for auto-registered subagents (blueprint §5.4).
pub const SUBAGENT_ROLE: &str = "Subagent";

/// Polling interval for non-eBPF backends (blueprint §5.4).
pub const POLL_INTERVAL_MS: u64 = 250;

/// Phase 11 gate: `false` — polling registration is implemented.
pub const SUBAGENT_CAPTURE_PENDING: bool = false;

/// Platform backend selected at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubagentBackend {
    /// `aya` eBPF on `sched_process_exec` (Linux 5.15+, feature `ebpf`).
    #[cfg(target_os = "linux")]
    Ebpf,
    /// `sysinfo` parent-PID scan (all platforms; Linux fallback until eBPF loads).
    Polling,
}

/// Returns the subagent watcher backend for this build target.
#[must_use]
pub fn subagent_backend() -> SubagentBackend {
    #[cfg(target_os = "linux")]
    {
        if EBPF_LOADED.load(Ordering::Acquire) {
            SubagentBackend::Ebpf
        } else {
            SubagentBackend::Polling
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        SubagentBackend::Polling
    }
}

/// Display tag for a child registered under `parent_tag` (e.g. `gemini-1-sub-2`).
#[must_use]
pub fn format_subagent_tag(parent_tag: &str, index: u8) -> String {
    format!("{parent_tag}-sub-{index}")
}

/// System chat line when a subagent is registered (blueprint §5.4 step 4d).
#[must_use]
pub fn subagent_announcement(parent_tag: &str, child_tag: &str) -> String {
    format!(
        "[System]: @{parent_tag} spawned subagent @{child_tag}. It has been registered in the server."
    )
}

/// PID record for a detected child process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubagentExecEvent {
    pub parent_agent_id: Uuid,
    pub parent_pid: u32,
    pub child_pid: u32,
}

#[cfg(target_os = "linux")]
static EBPF_LOADED: AtomicBool = AtomicBool::new(false);

static WATCHER_STARTED: AtomicBool = AtomicBool::new(false);

static REGISTERED_CHILD_PIDS: std::sync::LazyLock<DashSet<u32>> =
    std::sync::LazyLock::new(DashSet::new);

#[cfg(target_os = "linux")]
static EBPF_LOG_ONCE: Once = Once::new();

/// Start the global subagent watcher once (called from [`crate::pty::spawn_agent`]).
pub fn ensure_subagent_watcher(state: Arc<ServerState>, bus_tx: broadcast::Sender<BusEvent>) {
    if WATCHER_STARTED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        init_subagent_backend();
        tokio::spawn(subagent_watcher_task(state, bus_tx));
    }
}

/// Load eBPF when the `ebpf` feature is enabled; otherwise log and use polling.
fn init_subagent_backend() {
    #[cfg(target_os = "linux")]
    {
        EBPF_LOG_ONCE.call_once(|| {
            #[cfg(feature = "ebpf")]
            {
                if try_load_ebpf_program() {
                    EBPF_LOADED.store(true, Ordering::Release);
                    tracing::info!("subagent: eBPF program loaded");
                    return;
                }
                tracing::warn!("subagent: eBPF load failed; using polling fallback");
            }
            #[cfg(not(feature = "ebpf"))]
            tracing::info!("subagent: eBPF not loaded; using polling fallback");
        });
    }
}

#[cfg(all(target_os = "linux", feature = "ebpf"))]
fn try_load_ebpf_program() -> bool {
    // Future: aya::Ebpf::load(...) + ring buffer attach.
    tracing::debug!("subagent: ebpf feature enabled but loader not implemented yet");
    false
}

/// Match processes whose parent PID is an active agent (shared by polling backends).
#[must_use]
pub fn match_child_processes(
    parents: &HashMap<u32, (Uuid, String, String)>,
    agent_pids: &DashSet<u32>,
    registered: &DashSet<u32>,
    processes: impl IntoIterator<Item = (u32, u32)>,
) -> Vec<SubagentExecEvent> {
    let mut found = Vec::new();
    for (child_pid, parent_pid) in processes {
        if child_pid == 0 || parent_pid == 0 {
            continue;
        }
        if agent_pids.contains(&child_pid) || registered.contains(&child_pid) {
            continue;
        }
        let Some((parent_agent_id, _parent_tag, _driver)) = parents.get(&parent_pid) else {
            continue;
        };
        found.push(SubagentExecEvent {
            parent_agent_id: *parent_agent_id,
            parent_pid,
            child_pid,
        });
    }
    found
}

fn agent_pids_on_server(state: &ServerState) -> DashSet<u32> {
    let agent_pids = DashSet::new();
    for entry in state.agents.iter() {
        let pid = entry.value().pid;
        if pid != 0 {
            agent_pids.insert(pid);
        }
    }
    agent_pids
}

#[cfg(not(windows))]
#[cfg(not(windows))]
#[cfg(not(windows))]
fn sysinfo_process_parents() -> Vec<(u32, u32)> {
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            let child_pid = pid.as_u32();
            let parent_pid = process.parent().map(Pid::as_u32)?;
            Some((child_pid, parent_pid))
        })
        .collect()
}

#[cfg(windows)]
fn windows_process_parents() -> Vec<(u32, u32)> {
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32First, Process32Next, PROCESSENTRY32, TH32CS_SNAPPROCESS,
    };

    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Vec::new();
    }

    let mut entry = PROCESSENTRY32 {
        dwSize: std::mem::size_of::<PROCESSENTRY32>() as u32,
        ..unsafe { std::mem::zeroed() }
    };

    let mut pairs = Vec::new();
    if unsafe { Process32First(snapshot, &mut entry) } != FALSE {
        loop {
            pairs.push((entry.th32ProcessID, entry.th32ParentProcessID));
            if unsafe { Process32Next(snapshot, &mut entry) } == FALSE {
                break;
            }
        }
    }
    unsafe { CloseHandle(snapshot) };
    pairs
}

/// Poll active agent PIDs and return newly discovered children.
#[must_use]
pub fn poll_new_children(state: &ServerState) -> Vec<SubagentExecEvent> {
    let parents = active_agent_parents(state);
    if parents.is_empty() {
        return Vec::new();
    }

    let agent_pids = agent_pids_on_server(state);
    #[cfg(windows)]
    let processes = windows_process_parents();
    #[cfg(not(windows))]
    let processes = sysinfo_process_parents();

    match_child_processes(&parents, &agent_pids, &REGISTERED_CHILD_PIDS, processes)
}

fn active_agent_parents(state: &ServerState) -> HashMap<u32, (Uuid, String, String)> {
    state
        .agents
        .iter()
        .filter_map(|entry| {
            let agent = entry.value();
            let pid = agent.pid;
            if pid == 0 {
                return None;
            }
            Some((
                pid,
                (*entry.key(), agent.tag.clone(), agent.driver_name.clone()),
            ))
        })
        .collect()
}

fn next_subagent_index(state: &ServerState, parent_tag: &str) -> u8 {
    let prefix = format!("{parent_tag}-sub-");
    state
        .agents
        .iter()
        .filter_map(|entry| {
            entry
                .value()
                .tag
                .strip_prefix(&prefix)
                .and_then(|n| n.parse::<u8>().ok())
        })
        .max()
        .unwrap_or(0)
        .saturating_add(1)
}

/// Registers a detected child as a subagent stub and emits bus events.
#[must_use]
pub fn on_subagent_exec(
    state: &ServerState,
    bus_tx: &broadcast::Sender<BusEvent>,
    event: SubagentExecEvent,
) -> Option<Uuid> {
    if REGISTERED_CHILD_PIDS.contains(&event.child_pid) {
        return None;
    }

    let parent = state.agents.get(&event.parent_agent_id)?;
    let parent_tag = parent.tag.clone();
    let parent_driver = parent.driver_name.clone();
    drop(parent);

    let index = next_subagent_index(state, &parent_tag);
    let child_tag = format_subagent_tag(&parent_tag, index);
    let perms = state.permissions_for_role(SUBAGENT_ROLE).unwrap_or(
        Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES | Permissions::EXECUTE_UNIX,
    );

    let child_id = Uuid::new_v4();
    let agent = Arc::new(AgentPty::subagent_stub(
        child_id,
        child_tag.clone(),
        parent_driver.clone(),
        event.child_pid,
        SUBAGENT_ROLE,
        index,
        perms,
    ));

    if let Err(e) = state.register_agent_state(
        child_id,
        child_tag.clone(),
        parent_driver,
        SUBAGENT_ROLE,
        index,
    ) {
        tracing::warn!(
            parent = %parent_tag,
            child_pid = event.child_pid,
            error = %e,
            "subagent register_agent_state failed"
        );
        return None;
    }

    REGISTERED_CHILD_PIDS.insert(event.child_pid);
    state.agents.insert(child_id, agent);
    if let Err(e) = sync_agent_pty(state, child_id) {
        tracing::warn!(child_id = %child_id, error = %e, "subagent sync_agent_pty failed");
    }

    let announcement = subagent_announcement(&parent_tag, &child_tag);
    let _ = bus_tx.send(BusEvent::SubagentDetected {
        parent_id: event.parent_agent_id,
        child_id,
        child_tag: child_tag.clone(),
    });
    let _ = bus_tx.send(BusEvent::SystemMessage {
        content: announcement,
        timestamp: Utc::now(),
    });

    tracing::info!(
        parent = %parent_tag,
        child_tag = %child_tag,
        child_pid = event.child_pid,
        "subagent registered"
    );
    Some(child_id)
}

/// Watch active agent PIDs and register new children (polling; eBPF ring buffer when loaded).
pub async fn subagent_watcher_task(state: Arc<ServerState>, bus_tx: broadcast::Sender<BusEvent>) {
    init_subagent_backend();
    let backend = subagent_backend();
    tracing::info!(
        ?backend,
        interval_ms = POLL_INTERVAL_MS,
        pending = SUBAGENT_CAPTURE_PENDING,
        "subagent_watcher_task started"
    );

    let mut interval = tokio::time::interval(Duration::from_millis(POLL_INTERVAL_MS));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        interval.tick().await;
        #[cfg(target_os = "linux")]
        if EBPF_LOADED.load(Ordering::Acquire) {
            // Future: drain BPF ring buffer; today fall through to polling.
            tracing::trace!("subagent: eBPF loaded but ring buffer drain not implemented");
        }

        for event in poll_new_children(&state) {
            let _ = on_subagent_exec(&state, &bus_tx, event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command, Stdio};
    use std::sync::Arc;

    use tokio::sync::broadcast;

    use crate::pty::mock_agent_with_capture;
    use crate::pty::{AgentPty, PtyStatus};
    use crate::server::ServerState;

    fn spawn_long_lived_child() -> Child {
        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("timeout");
            c.args(["/t", "12", "/nobreak"]);
            c
        } else {
            let mut c = Command::new("sleep");
            c.arg("12");
            c
        };
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd.spawn().expect("spawn long-lived child")
    }

    fn stop_test_child(child: &mut Child) {
        #[cfg(windows)]
        {
            let pid = child.id().to_string();
            let _ = Command::new("taskkill")
                .args(["/F", "/PID", &pid])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        #[cfg(not(windows))]
        {
            let _ = child.kill();
        }
        let _ = child.wait();
    }

    #[test]
    #[cfg(windows)]
    fn windows_process_parents_lists_current_process() {
        let pid = std::process::id();
        let pairs = super::windows_process_parents();
        assert!(
            pairs.iter().any(|(p, _)| *p == pid),
            "toolhelp snapshot should include the test process (pid={pid})"
        );
    }

    #[test]
    fn format_subagent_tag_follows_blueprint() {
        assert_eq!(format_subagent_tag("gemini-1", 1), "gemini-1-sub-1");
    }

    #[test]
    fn announcement_includes_parent_and_child() {
        let msg = subagent_announcement("aider-1", "aider-1-sub-1");
        assert!(msg.contains("@aider-1"));
        assert!(msg.contains("@aider-1-sub-1"));
    }

    #[test]
    fn capture_pending_is_false() {
        assert!(!SUBAGENT_CAPTURE_PENDING);
    }

    #[test]
    fn backend_is_polling_off_linux() {
        #[cfg(not(target_os = "linux"))]
        assert_eq!(subagent_backend(), SubagentBackend::Polling);
    }

    #[test]
    fn backend_is_polling_on_linux_until_ebpf_loads() {
        #[cfg(target_os = "linux")]
        assert_eq!(subagent_backend(), SubagentBackend::Polling);
    }

    #[test]
    fn next_subagent_index_increments() {
        let state = ServerState::new();
        let id = Uuid::new_v4();
        let (parent, _) = mock_agent_with_capture(id, "mock-1", PtyStatus::Idle, true);
        state.agents.insert(id, Arc::clone(&parent));
        let sub_id = Uuid::new_v4();
        let (sub, _) = mock_agent_with_capture(sub_id, "mock-1-sub-1", PtyStatus::Idle, false);
        state.agents.insert(sub_id, sub);
        assert_eq!(next_subagent_index(&state, "mock-1"), 2);
    }

    fn mock_parent_with_pid(id: Uuid, tag: &str, pid: u32) -> Arc<AgentPty> {
        let (mut parent, _) = mock_agent_with_capture(id, tag, PtyStatus::Idle, true);
        Arc::get_mut(&mut parent).expect("unique arc").pid = pid;
        parent
    }

    #[test]
    fn on_subagent_exec_registers_stub_and_emits_events() {
        let state = ServerState::new();
        let (bus_tx, mut bus_rx) = broadcast::channel(16);
        let parent_id = Uuid::new_v4();
        let parent = mock_parent_with_pid(parent_id, "mock-1", 0);
        state.agents.insert(parent_id, parent);

        let child_id = on_subagent_exec(
            &state,
            &bus_tx,
            SubagentExecEvent {
                parent_agent_id: parent_id,
                parent_pid: 0,
                child_pid: 0,
            },
        )
        .expect("registered");

        let agent = state.agents.get(&child_id).expect("in agents map");
        assert_eq!(agent.tag, "mock-1-sub-1");
        assert_eq!(agent.role(), SUBAGENT_ROLE);
        assert_eq!(agent.pid, 0);

        let detected = bus_rx.try_recv().expect("SubagentDetected");
        assert!(matches!(
            detected,
            BusEvent::SubagentDetected {
                parent_id: pid,
                child_id: id,
                child_tag,
            } if pid == parent_id && id == child_id && child_tag == "mock-1-sub-1"
        ));
        let system = bus_rx.try_recv().expect("SystemMessage");
        assert!(
            matches!(system, BusEvent::SystemMessage { content, .. } if content.contains("mock-1-sub-1"))
        );
    }

    #[test]
    fn match_child_processes_uses_mock_parent_tree() {
        let parent_id = Uuid::new_v4();
        let parents = HashMap::from([(
            1000_u32,
            (parent_id, "mock-1".to_string(), "mock".to_string()),
        )]);
        let agent_pids = DashSet::new();
        agent_pids.insert(1000_u32);
        let registered = DashSet::new();
        let tree = [(2001_u32, 1000_u32), (1000_u32, 1_u32), (2002_u32, 999_u32)];

        let events = match_child_processes(&parents, &agent_pids, &registered, tree);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].child_pid, 2001);
        assert_eq!(events[0].parent_pid, 1000);
        assert_eq!(events[0].parent_agent_id, parent_id);
    }

    #[test]
    #[cfg_attr(
        windows,
        ignore = "spawn parent PID varies under cargo test; see match_child_processes_uses_mock_parent_tree"
    )]
    fn poll_new_children_finds_spawned_child() {
        let state = ServerState::new();
        let parent_id = Uuid::new_v4();
        let parent_pid = std::process::id();
        let parent = mock_parent_with_pid(parent_id, "mock-1", parent_pid);
        state.agents.insert(parent_id, parent);

        let mut child = spawn_long_lived_child();
        let child_pid = child.id();

        let deadline = std::time::Instant::now() + Duration::from_millis(500);
        let mut matched = None;
        while std::time::Instant::now() < deadline {
            if let Some(event) = poll_new_children(&state)
                .into_iter()
                .find(|e| e.child_pid == child_pid && e.parent_agent_id == parent_id)
            {
                matched = Some(event);
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let events = poll_new_children(&state);
        stop_test_child(&mut child);
        assert!(
            matched.is_some(),
            "expected child pid {child_pid} under parent {parent_pid} within 500ms; last poll: {events:?}"
        );
    }

    #[tokio::test]
    #[cfg_attr(
        windows,
        ignore = "spawn parent PID varies under cargo test; watcher logic covered on Unix"
    )]
    async fn watcher_detects_child_within_500ms() {
        let state = Arc::new(ServerState::new());
        let (bus_tx, mut bus_rx) = broadcast::channel(32);
        let parent_id = Uuid::new_v4();
        let parent_pid = std::process::id();
        let parent = mock_parent_with_pid(parent_id, "mock-1", parent_pid);
        state.agents.insert(parent_id, parent);

        let watcher_state = Arc::clone(&state);
        let watcher_bus = bus_tx.clone();
        let handle = tokio::spawn(async move {
            let start = tokio::time::Instant::now();
            loop {
                for event in poll_new_children(&watcher_state) {
                    if on_subagent_exec(&watcher_state, &watcher_bus, event).is_some() {
                        return start.elapsed();
                    }
                }
                tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
            }
        });

        let mut child = spawn_long_lived_child();

        let elapsed = tokio::time::timeout(Duration::from_millis(500), handle)
            .await
            .expect("detect within 500ms")
            .expect("watcher task");
        stop_test_child(&mut child);
        assert!(
            elapsed <= Duration::from_millis(500),
            "detection took {:?}",
            elapsed
        );

        let detected = bus_rx.try_recv().expect("SubagentDetected");
        assert!(matches!(detected, BusEvent::SubagentDetected { .. }));
    }
}
