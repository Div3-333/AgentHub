//! PTY lifecycle: [`AgentPty`], [`PtyStatus`], and [`spawn_agent`] (blueprint §5.1–5.2).

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

use portable_pty::MasterPty;
#[cfg(feature = "full")]
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::bus::{BusEvent, OfflineReason};
#[cfg(feature = "full")]
use crate::config::{load_driver_profile_from_dir, AgentHubConfig, DriverProfile};
#[cfg(feature = "full")]
use crate::db::DbClient;
use crate::error::{AgentHubError, Result};
#[cfg(feature = "full")]
use crate::pty::debug_log::PtyDebugSink;
use crate::pty::io::PtyRingBuffer;
#[cfg(feature = "full")]
use crate::pty::io::{pty_reader_task, stdio_reader_task};
#[cfg(feature = "full")]
use crate::pty::spawn_cmd::{format_resolved_command, resolve_spawn_command, ResolvedCommand};
#[cfg(feature = "full")]
use crate::pty::subagent::ensure_subagent_watcher;
#[cfg(feature = "full")]
use crate::pty::trace::{emit_pty_io_trace, emit_spawn_trace};
#[cfg(feature = "full")]
use crate::sanitizer::spawn_sanitizer_task;
#[cfg(feature = "full")]
use crate::server::induction::run_induction;
#[cfg(feature = "full")]
use crate::server::modes::{sync_agent_pty, validate_spawn};
#[cfg(feature = "full")]
use crate::server::rbac::Permissions;
use crate::server::ServerState;

/// Lifecycle state of a single agent PTY (blueprint §5.1).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PtyStatus {
    Initializing = 0,
    Idle = 1,
    Thinking = 2,
    Muted = 3,
    Deafened = 4,
    Suspended = 5,
    Dead = 6,
    RateLimited = 7,
}

impl PtyStatus {
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Initializing),
            1 => Some(Self::Idle),
            2 => Some(Self::Thinking),
            3 => Some(Self::Muted),
            4 => Some(Self::Deafened),
            5 => Some(Self::Suspended),
            6 => Some(Self::Dead),
            7 => Some(Self::RateLimited),
            _ => None,
        }
    }
}

/// Cache-line aligned PTY handle for one agent child process (blueprint §5.1).
#[repr(C, align(64))]
pub struct AgentPty {
    pub id: Uuid,
    pub tag: String,
    pub driver_name: String,
    pub pid: u32,
    pub status: AtomicU8,
    pub role_mask: AtomicU32,
    role: Mutex<String>,
    instance_number: u8,
    online_since: DateTime<Utc>,
    timeout_until: AtomicI64,
    banned: AtomicBool,
    pub master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    pub receives_broadcast: AtomicBool,
    pub visible_in_chat: AtomicBool,
    ring_buffer: Arc<PtyRingBuffer>,
    pub(crate) pty_reader: Mutex<Option<Box<dyn Read + Send>>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    child: Mutex<Option<Box<dyn portable_pty::Child + Send + Sync>>>,
}

impl AgentPty {
    #[must_use]
    pub fn status(&self) -> Option<PtyStatus> {
        PtyStatus::from_u8(self.status.load(Ordering::Acquire))
    }

    #[must_use]
    pub fn role(&self) -> String {
        self.role
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_else(|_| "Builder".to_string())
    }

    pub fn set_role(&self, role: &str) {
        if let Ok(mut guard) = self.role.lock() {
            *guard = role.to_string();
        }
    }

    #[must_use]
    pub fn permissions(&self) -> crate::server::rbac::Permissions {
        crate::server::rbac::Permissions::from_bits_truncate(
            self.role_mask.load(Ordering::Acquire) as u64
        )
    }

    pub fn set_permissions(&self, permissions: crate::server::rbac::Permissions) {
        self.role_mask.store(
            (permissions.bits() & u32::MAX as u64) as u32,
            Ordering::Release,
        );
    }

    #[must_use]
    pub fn instance_number(&self) -> u8 {
        instance_number_from_tag(&self.tag)
    }

    #[must_use]
    pub fn online_since(&self) -> DateTime<Utc> {
        self.online_since
    }

    #[must_use]
    pub fn timeout_until(&self) -> i64 {
        self.timeout_until.load(Ordering::Acquire)
    }

    #[must_use]
    pub fn banned(&self) -> bool {
        self.banned.load(Ordering::Acquire)
    }

    #[must_use]
    pub fn ring_buffer(&self) -> Arc<PtyRingBuffer> {
        Arc::clone(&self.ring_buffer)
    }

    /// Stub PTY record for a detected child process (no master/writer; §5.4 subagents).
    pub(crate) fn subagent_stub(
        id: Uuid,
        tag: String,
        driver_name: String,
        pid: u32,
        role: &str,
        instance_number: u8,
        perms: crate::server::rbac::Permissions,
    ) -> Self {
        Self {
            id,
            tag,
            driver_name,
            pid,
            status: AtomicU8::new(PtyStatus::Idle.as_u8()),
            role_mask: AtomicU32::new((perms.bits() & u32::MAX as u64) as u32),
            role: Mutex::new(role.to_string()),
            instance_number,
            online_since: Utc::now(),
            timeout_until: AtomicI64::new(0),
            banned: AtomicBool::new(false),
            master: Mutex::new(None),
            receives_broadcast: AtomicBool::new(false),
            visible_in_chat: AtomicBool::new(true),
            ring_buffer: Arc::new(PtyRingBuffer::new()),
            pty_reader: Mutex::new(None),
            writer: Mutex::new(None),
            child: Mutex::new(None),
        }
    }

    /// Write bytes to the agent PTY stdin.
    pub fn write_stdin(&self, data: &[u8]) -> Result<usize> {
        use std::io::Write;

        let mut guard = self
            .writer
            .lock()
            .map_err(|_| AgentHubError::Pty("PTY writer lock poisoned".into()))?;
        let writer = guard
            .as_mut()
            .ok_or_else(|| AgentHubError::Pty("PTY writer closed".into()))?;
        let n = writer
            .write(data)
            .map_err(|e| AgentHubError::Pty(format!("stdin write failed: {e}")))?;
        writer
            .flush()
            .map_err(|e| AgentHubError::Pty(format!("stdin flush failed: {e}")))?;
        Ok(n)
    }

    /// Last-resort shutdown: close I/O, SIGTERM → SIGKILL → reap (§5.1).
    ///
    /// Stdio/pipe children must have stdin/stdout closed before `wait()` on Windows;
    /// otherwise `read()` and `wait()` can block indefinitely while the child waits on stdin.
    fn teardown_child(&self) {
        if let Ok(mut writer_guard) = self.writer.lock() {
            let _ = writer_guard.take();
        }
        if let Ok(mut reader_guard) = self.pty_reader.lock() {
            let _ = reader_guard.take();
        }
        if let Ok(mut master_guard) = self.master.lock() {
            let _ = master_guard.take();
        }

        escalate_kill(self.pid);

        if let Ok(mut child_guard) = self.child.lock() {
            if let Some(mut child) = child_guard.take() {
                reap_child_with_timeout(child.as_mut());
            }
        }
    }
}

/// Reap a child with a bounded wait so teardown never blocks the runtime (CI smoke / exit paths).
fn reap_child_with_timeout(child: &mut dyn portable_pty::Child) {
    use std::io::ErrorKind;

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if Instant::now() >= deadline => break,
            Ok(None) => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {
                if Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

impl Drop for AgentPty {
    fn drop(&mut self) {
        self.status
            .store(PtyStatus::Dead.as_u8(), Ordering::Release);
        self.teardown_child();
    }
}

/// Suspend agent processes during `/timeout` or VFS revert (SIGSTOP on Unix).
pub fn freeze_agent_pids(pids: &[u32]) {
    #[cfg(unix)]
    {
        for &pid in pids {
            if pid == 0 {
                continue;
            }
            let rc = unsafe { libc::kill(pid as i32, libc::SIGSTOP) };
            if rc != 0 {
                tracing::warn!(pid, "freeze_agent_pids: SIGSTOP failed");
            }
        }
    }

    #[cfg(windows)]
    {
        for &pid in pids {
            if pid == 0 {
                continue;
            }
            if let Err(e) = windows_suspend_process(pid) {
                tracing::warn!(pid, error = %e, "freeze_agent_pids: SuspendThread failed");
            }
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = pids;
    }
}

/// Resume agent processes after `/timeout` or VFS revert (SIGCONT on Unix).
pub fn resume_agent_pids(pids: &[u32]) {
    #[cfg(unix)]
    {
        for &pid in pids {
            if pid == 0 {
                continue;
            }
            let rc = unsafe { libc::kill(pid as i32, libc::SIGCONT) };
            if rc != 0 {
                tracing::warn!(pid, "resume_agent_pids: SIGCONT failed");
            }
        }
    }

    #[cfg(windows)]
    {
        for &pid in pids {
            if pid == 0 {
                continue;
            }
            if let Err(e) = windows_resume_process(pid) {
                tracing::warn!(pid, error = %e, "resume_agent_pids: ResumeThread failed");
            }
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = pids;
    }
}

/// SIGTERM → 100ms grace → SIGKILL → `waitpid` on Unix; graceful `taskkill` then `/F /T` on Windows (§5.1).
pub fn escalate_kill(pid: u32) {
    if pid == 0 {
        return;
    }

    #[cfg(unix)]
    {
        const GRACE_MS: u64 = 100;
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
        std::thread::sleep(Duration::from_millis(GRACE_MS));
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let mut status = 0;
            let waited = unsafe { libc::waitpid(pid as i32, &mut status, libc::WNOHANG) };
            if waited == pid as i32 || waited == -1 {
                break;
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(windows)]
    {
        if windows_process_exited(pid) {
            return;
        }

        const GRACE_MS: u64 = 100;
        windows_taskkill(&["/PID", &pid.to_string()]);
        std::thread::sleep(Duration::from_millis(GRACE_MS));
        if !windows_process_exited(pid) {
            windows_taskkill(&["/PID", &pid.to_string(), "/T", "/F"]);
        }

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if windows_process_exited(pid) {
                break;
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(windows)]
fn windows_suspend_process(pid: u32) -> std::io::Result<()> {
    windows_for_each_thread(pid, |thread_id| {
        let handle = windows_open_thread(thread_id)?;
        let previous = unsafe { windows_sys::Win32::System::Threading::SuspendThread(handle) };
        unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) };
        if previous == u32::MAX {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    })
}

#[cfg(windows)]
fn windows_resume_process(pid: u32) -> std::io::Result<()> {
    windows_for_each_thread(pid, |thread_id| {
        let handle = windows_open_thread(thread_id)?;
        let previous = unsafe { windows_sys::Win32::System::Threading::ResumeThread(handle) };
        unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) };
        if previous == u32::MAX {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    })
}

#[cfg(windows)]
fn windows_for_each_thread<F>(pid: u32, mut f: F) -> std::io::Result<()>
where
    F: FnMut(u32) -> std::io::Result<()>,
{
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };

    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }

    let mut entry = THREADENTRY32 {
        dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
        ..unsafe { std::mem::zeroed() }
    };

    let mut found = false;
    if unsafe { Thread32First(snapshot, &mut entry) } != FALSE {
        loop {
            if entry.th32OwnerProcessID == pid {
                found = true;
                f(entry.th32ThreadID)?;
            }
            if unsafe { Thread32Next(snapshot, &mut entry) } == FALSE {
                break;
            }
        }
    }
    unsafe { CloseHandle(snapshot) };
    if found {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("no threads found for pid {pid}"),
        ))
    }
}

#[cfg(windows)]
fn windows_open_thread(thread_id: u32) -> std::io::Result<windows_sys::Win32::Foundation::HANDLE> {
    use windows_sys::Win32::Foundation::FALSE;
    use windows_sys::Win32::System::Threading::{OpenThread, THREAD_SUSPEND_RESUME};

    let handle = unsafe { OpenThread(THREAD_SUSPEND_RESUME, FALSE, thread_id) };
    if handle == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(handle)
    }
}

#[cfg(windows)]
fn windows_taskkill(args: &[&str]) {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let _ = Command::new("taskkill")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

#[cfg(windows)]
fn windows_process_exited(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    const STILL_ACTIVE: u32 = 259;

    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
    if handle == 0 {
        return true;
    }
    let mut code = 0u32;
    let ok = unsafe { GetExitCodeProcess(handle, &mut code) };
    unsafe { CloseHandle(handle) };
    ok == 0 || code != STILL_ACTIVE
}

/// Returns `true` when PTY integration tests should run.
///
/// True when [`spawn_agent`] should use piped stdio instead of a pseudo-terminal.
///
/// Set `AGENTHUB_SKIP_PTY=1` on CI runners without pseudo-terminal support. The mock CLI
/// path still runs end-to-end over stdin/stdout pipes.
#[must_use]
pub fn pty_skip_mode() -> bool {
    std::env::var("AGENTHUB_SKIP_PTY").is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

/// Set `AGENTHUB_SKIP_PTY=1` on CI runners without pseudo-terminal support.
/// On Windows, PTY tests are skipped unless `AGENTHUB_FORCE_PTY=1` (ConPTY reads block
/// in the portable-pty reader used by integration tests).
#[must_use]
pub fn pty_tests_enabled() -> bool {
    if pty_skip_mode() {
        return false;
    }
    if cfg!(windows) {
        return std::env::var("AGENTHUB_FORCE_PTY")
            .is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    }
    true
}

#[cfg(feature = "full")]
fn count_driver_instances(state: &ServerState, driver_name: &str) -> usize {
    state
        .agents
        .iter()
        .filter(|entry| entry.value().driver_name == driver_name)
        .count()
}

#[cfg(feature = "full")]
fn next_instance_tag(state: &ServerState, driver_name: &str) -> String {
    let n = count_driver_instances(state, driver_name) + 1;
    format!("{driver_name}-{n}")
}

fn instance_number_from_tag(tag: &str) -> u8 {
    tag.rsplit_once('-')
        .and_then(|(_, n)| n.parse().ok())
        .unwrap_or(1)
}

/// Spawns a new agent CLI inside an isolated PTY (blueprint §5.2).
///
/// Steps:
/// 1. Load and validate the `DriverProfile` for `driver_name`.
/// 2. Check `max_agents` limit and `driver.max_instances` limit.
/// 3. Create a PTY pair using `portable_pty::native_pty_system()`.
/// 4. Configure the PTY size to 220 columns × 50 rows.
/// 5. Set environment variables from `driver.env` plus `TERM=dumb`, `NO_COLOR=1`, `AGENTHUB=1`.
/// 6. Spawn the child process using `pty_pair.slave.spawn_command(cmd)`.
/// 7. Close the slave PTY in the parent immediately after spawning.
/// 8. Move the master PTY handle, writer, and ring buffer into `Arc<AgentPty>`.
/// 9. Spawn `pty_reader_task` to drain stdout into the ring buffer.
/// 10. Insert the `Arc<AgentPty>` into `ServerState.agents`.
/// 11. Run `init_sequence` injections asynchronously (Phase 3 adds `sanitizer_task`).
/// 12. Return the agent `Uuid`.
///
/// Optional overrides for [`spawn_agent`].
#[derive(Debug, Clone, Default)]
pub struct SpawnOptions {
    pub tag: Option<String>,
    pub role: Option<String>,
    /// Skip sanitizer task (PTY ring-buffer tests are single-consumer).
    pub skip_sanitizer: bool,
    /// Skip init_sequence and induction (faster PTY lifecycle tests).
    pub skip_induction: bool,
}

#[cfg(feature = "full")]
pub async fn spawn_agent(
    driver_name: &str,
    config: &AgentHubConfig,
    server_state: Arc<ServerState>,
    bus_tx: broadcast::Sender<BusEvent>,
    db: Option<Arc<DbClient>>,
    options: SpawnOptions,
) -> Result<Uuid> {
    if server_state.banned_drivers.contains(driver_name) {
        return Err(AgentHubError::DriverProfile {
            driver: driver_name.to_string(),
            msg: "driver is banned for this session".into(),
        });
    }

    let driver = load_driver_profile_from_dir(&config.drivers_dir, driver_name)?;
    driver.validate()?;

    if server_state.agents.len() >= usize::from(config.max_agents) {
        return Err(AgentHubError::Config(format!(
            "max_agents limit ({}) reached",
            config.max_agents
        )));
    }

    if driver.max_instances > 0 {
        let instances = count_driver_instances(&server_state, driver_name);
        if instances >= usize::from(driver.max_instances) {
            return Err(AgentHubError::DriverProfile {
                driver: driver_name.to_string(),
                msg: format!("max_instances ({}) reached", driver.max_instances),
            });
        }
    }

    if !driver.supports_multi_instance && count_driver_instances(&server_state, driver_name) > 0 {
        tracing::warn!(
            driver = %driver.name,
            "spawning another instance though supports_multi_instance is false"
        );
    }

    let role_name = options
        .role
        .clone()
        .unwrap_or_else(|| "Builder".to_string());
    let perms = server_state
        .permissions_for_role(&role_name)
        .unwrap_or_else(|| Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES);

    let tag = options
        .tag
        .clone()
        .unwrap_or_else(|| next_instance_tag(&server_state, driver_name));

    validate_spawn(&server_state, config.max_agents, &tag)?;

    emit_spawn_trace(
        &bus_tx,
        &tag,
        format!("loading driver `{driver_name}`"),
        config.spawn_debug,
    );

    let resolved = resolve_spawn_command(&driver.executable, &driver.args)?;
    let command_line = format_resolved_command(&resolved);
    emit_spawn_trace(
        &bus_tx,
        &tag,
        format!("command: {command_line}"),
        config.spawn_debug,
    );

    let (agent, ring_buffer) = if pty_skip_mode() {
        spawn_stdio_process(
            driver_name,
            &driver,
            &resolved,
            tag.clone(),
            &role_name,
            perms,
        )?
    } else {
        spawn_pty_process(
            driver_name,
            &driver,
            &resolved,
            tag.clone(),
            &role_name,
            perms,
        )?
    };
    let agent = Arc::new(agent);
    let id = agent.id;

    server_state.register_agent_state(
        id,
        tag.clone(),
        driver_name.to_string(),
        &role_name,
        agent.instance_number(),
    )?;
    server_state.agents.insert(id, Arc::clone(&agent));
    sync_agent_pty(&server_state, id)?;

    let _ = bus_tx.send(BusEvent::AgentSpawnStarted {
        id,
        tag: tag.clone(),
        driver: driver_name.to_string(),
        role: role_name.clone(),
        command_line: command_line.clone(),
    });
    emit_spawn_trace(
        &bus_tx,
        &tag,
        "PTY process running; init_sequence + induction next",
        config.spawn_debug,
    );

    let debug_sink = if config.pty_debug_log {
        Some(PtyDebugSink::spawn(db, id))
    } else {
        None
    };

    let reader_agent = Arc::clone(&agent);
    let reader_bus = bus_tx.clone();
    let spawn_debug = config.spawn_debug;
    if pty_skip_mode() {
        tokio::spawn(stdio_reader_task(
            reader_agent,
            ring_buffer,
            reader_bus,
            debug_sink,
            spawn_debug,
        ));
    } else {
        tokio::spawn(pty_reader_task(
            reader_agent,
            ring_buffer,
            reader_bus,
            debug_sink,
            spawn_debug,
        ));
    }

    if !options.skip_sanitizer {
        spawn_sanitizer_task(
            Arc::clone(&agent),
            driver.clone(),
            Arc::clone(&server_state),
            bus_tx.clone(),
        );
    }

    if !options.skip_induction {
        let init_agent = Arc::clone(&agent);
        let init_driver = driver.clone();
        let induction_agent = Arc::clone(&agent);
        let induction_state = Arc::clone(&server_state);
        let induction_bus = bus_tx.clone();
        let induction_debug = config.spawn_debug;
        tokio::spawn(async move {
            run_init_sequence(
                init_agent,
                init_driver,
                induction_bus.clone(),
                induction_debug,
            )
            .await;
            run_induction(
                induction_agent,
                induction_state,
                induction_bus,
                induction_debug,
            )
            .await;
        });
    }

    ensure_subagent_watcher(Arc::clone(&server_state), bus_tx);

    Ok(id)
}

/// Apply driver env + AgentHub defaults onto a PTY command builder.
#[cfg(feature = "full")]
fn configure_command_builder(cmd: &mut CommandBuilder, driver: &DriverProfile) {
    for (key, value) in &driver.env {
        cmd.env(key, value);
    }
    cmd.env("TERM", "dumb");
    cmd.env("NO_COLOR", "1");
    cmd.env("AGENTHUB", "1");
    if let Ok(cwd) = std::env::current_dir() {
        cmd.cwd(cwd);
    }
}

#[cfg(feature = "full")]
fn build_pty_command(resolved: &ResolvedCommand) -> CommandBuilder {
    let mut cmd = CommandBuilder::new(&resolved.program);
    for arg in &resolved.args {
        cmd.arg(arg);
    }
    cmd
}

/// Spawn a driver with piped stdin/stdout (CI / `AGENTHUB_SKIP_PTY=1` path).
#[cfg(feature = "full")]
fn spawn_stdio_process(
    driver_name: &str,
    driver: &DriverProfile,
    resolved: &ResolvedCommand,
    tag: String,
    role_name: &str,
    perms: Permissions,
) -> Result<(AgentPty, Arc<PtyRingBuffer>)> {
    use std::process::{Command, Stdio};

    let mut cmd = Command::new(&resolved.program);
    for arg in &resolved.args {
        cmd.arg(arg);
    }
    for (key, value) in &driver.env {
        cmd.env(key, value);
    }
    cmd.env("TERM", "dumb");
    cmd.env("NO_COLOR", "1");
    cmd.env("AGENTHUB", "1");
    if let Ok(cwd) = std::env::current_dir() {
        cmd.current_dir(cwd);
    }
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::null());

    let mut child = cmd
        .spawn()
        .map_err(|e| AgentHubError::Pty(format!("stdio spawn failed: {e}")))?;
    let pid = child.id();
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| AgentHubError::Pty("stdio child has no stdin".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AgentHubError::Pty("stdio child has no stdout".into()))?;

    let id = Uuid::new_v4();
    let ring_buffer = Arc::new(PtyRingBuffer::new());
    let instance_number = instance_number_from_tag(&tag);
    let agent = AgentPty {
        id,
        tag,
        driver_name: driver_name.to_string(),
        pid,
        status: AtomicU8::new(PtyStatus::Initializing.as_u8()),
        role_mask: AtomicU32::new((perms.bits() & u32::MAX as u64) as u32),
        role: Mutex::new(role_name.to_string()),
        instance_number,
        online_since: Utc::now(),
        timeout_until: AtomicI64::new(0),
        banned: AtomicBool::new(false),
        master: Mutex::new(None),
        receives_broadcast: AtomicBool::new(true),
        visible_in_chat: AtomicBool::new(true),
        ring_buffer: Arc::clone(&ring_buffer),
        pty_reader: Mutex::new(Some(Box::new(stdout))),
        writer: Mutex::new(Some(Box::new(stdin))),
        child: Mutex::new(Some(
            Box::new(child) as Box<dyn portable_pty::Child + Send + Sync>
        )),
    };

    Ok((agent, ring_buffer))
}

#[cfg(feature = "full")]
fn spawn_pty_process(
    driver_name: &str,
    driver: &DriverProfile,
    resolved: &ResolvedCommand,
    tag: String,
    role_name: &str,
    perms: Permissions,
) -> Result<(AgentPty, Arc<PtyRingBuffer>)> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 50,
            cols: 220,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| AgentHubError::Pty(format!("openpty failed: {e}")))?;

    let mut cmd = build_pty_command(resolved);
    configure_command_builder(&mut cmd, driver);

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| AgentHubError::Pty(format!("spawn_command failed: {e}")))?;
    drop(pair.slave);

    let pid = child
        .process_id()
        .ok_or_else(|| AgentHubError::Pty("spawned child has no process id".into()))?;

    let writer = pair
        .master
        .take_writer()
        .map_err(|e| AgentHubError::Pty(format!("take_writer failed: {e}")))?;
    let pty_reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| AgentHubError::Pty(format!("try_clone_reader failed: {e}")))?;

    let id = Uuid::new_v4();
    let ring_buffer = Arc::new(PtyRingBuffer::new());

    let instance_number = instance_number_from_tag(&tag);
    let agent = AgentPty {
        id,
        tag,
        driver_name: driver_name.to_string(),
        pid,
        status: AtomicU8::new(PtyStatus::Initializing.as_u8()),
        role_mask: AtomicU32::new((perms.bits() & u32::MAX as u64) as u32),
        role: Mutex::new(role_name.to_string()),
        instance_number,
        online_since: Utc::now(),
        timeout_until: AtomicI64::new(0),
        banned: AtomicBool::new(false),
        master: Mutex::new(Some(pair.master)),
        receives_broadcast: AtomicBool::new(true),
        visible_in_chat: AtomicBool::new(true),
        ring_buffer: Arc::clone(&ring_buffer),
        pty_reader: Mutex::new(Some(pty_reader)),
        writer: Mutex::new(Some(writer)),
        child: Mutex::new(Some(child)),
    };

    Ok((agent, ring_buffer))
}

#[cfg(feature = "full")]
async fn run_init_sequence(
    agent: Arc<AgentPty>,
    driver: DriverProfile,
    bus_tx: broadcast::Sender<BusEvent>,
    spawn_debug: bool,
) {
    for line in driver.init_sequence {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let payload = format!("{line}\n");
        emit_pty_io_trace(&bus_tx, &agent.tag, "in", payload.as_bytes(), spawn_debug);
        if let Err(e) = agent.write_stdin(payload.as_bytes()) {
            tracing::warn!(agent = %agent.tag, "init_sequence write failed: {e}");
        }
    }
}

/// Integration-test helper: real PTY child without reader/sanitizer tasks (Drop / zombie tests).
#[cfg(feature = "full")]
pub fn spawn_test_pty_agent(
    driver_name: &str,
    drivers_dir: &std::path::Path,
    tag: &str,
) -> Result<Arc<AgentPty>> {
    let driver = load_driver_profile_from_dir(drivers_dir, driver_name)?;
    driver.validate()?;
    let perms = Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES;
    let resolved = resolve_spawn_command(&driver.executable, &driver.args)?;
    let (agent, _) = spawn_pty_process(
        driver_name,
        &driver,
        &resolved,
        tag.to_string(),
        "Builder",
        perms,
    )?;
    Ok(Arc::new(agent))
}

/// Test helper: mock [`AgentPty`] with stdin capture for router/moderation tests.
#[cfg(any(test, feature = "full"))]
pub fn mock_agent_with_capture(
    id: Uuid,
    tag: &str,
    status: PtyStatus,
    receives_broadcast: bool,
) -> (Arc<AgentPty>, Arc<Mutex<Vec<u8>>>) {
    use std::io::Write;
    use std::sync::atomic::{AtomicBool, AtomicI64};

    struct CapturingWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for CapturingWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("capture lock").extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let capture = Arc::new(Mutex::new(Vec::new()));
    let agent = AgentPty {
        id,
        tag: tag.to_string(),
        driver_name: "mock".into(),
        pid: 0,
        status: AtomicU8::new(status.as_u8()),
        role_mask: AtomicU32::new(0),
        role: Mutex::new("Builder".into()),
        instance_number: instance_number_from_tag(tag),
        online_since: Utc::now(),
        timeout_until: AtomicI64::new(0),
        banned: AtomicBool::new(false),
        master: Mutex::new(None),
        receives_broadcast: AtomicBool::new(receives_broadcast),
        visible_in_chat: AtomicBool::new(true),
        ring_buffer: Arc::new(PtyRingBuffer::new()),
        pty_reader: Mutex::new(None),
        writer: Mutex::new(Some(Box::new(CapturingWriter(Arc::clone(&capture))))),
        child: Mutex::new(None),
    };
    (Arc::new(agent), capture)
}

/// Test helper: create a mock [`AgentPty`] with a provided ring buffer and optional stdin writer.
#[cfg(any(test, feature = "full"))]
pub fn mock_agent_for_tests(
    tag: &str,
    status: PtyStatus,
    ring_buffer: Arc<PtyRingBuffer>,
    writer: Option<Box<dyn Write + Send>>,
) -> Arc<AgentPty> {
    use std::sync::atomic::{AtomicBool, AtomicI64};

    Arc::new(AgentPty {
        id: Uuid::new_v4(),
        tag: tag.to_string(),
        driver_name: "mock".into(),
        pid: 0,
        status: AtomicU8::new(status.as_u8()),
        role_mask: AtomicU32::new(0),
        role: Mutex::new("Builder".into()),
        instance_number: instance_number_from_tag(tag),
        online_since: Utc::now(),
        timeout_until: AtomicI64::new(0),
        banned: AtomicBool::new(false),
        master: Mutex::new(None),
        receives_broadcast: AtomicBool::new(true),
        visible_in_chat: AtomicBool::new(true),
        ring_buffer,
        pty_reader: Mutex::new(None),
        writer: Mutex::new(writer),
        child: Mutex::new(None),
    })
}

/// Force-kill an agent and remove it from server state.
pub fn kill_agent(
    server_state: &ServerState,
    id: Uuid,
    bus_tx: &broadcast::Sender<BusEvent>,
    reason: OfflineReason,
) -> Result<()> {
    let (_, agent) = server_state
        .agents
        .remove(&id)
        .ok_or(AgentHubError::AgentNotFound(id))?;
    let tag = agent.tag.clone();
    agent
        .status
        .store(PtyStatus::Dead.as_u8(), Ordering::Release);
    agent.teardown_child();
    let _ = bus_tx.send(BusEvent::AgentOffline { id, tag, reason });
    Ok(())
}

#[cfg(all(test, feature = "full"))]
mod tests {
    use super::*;
    use std::sync::Arc;

    use tokio::sync::broadcast;

    use crate::config::AgentHubConfig;
    use crate::server::ServerState;

    #[test]
    fn pty_status_roundtrip() {
        for status in [PtyStatus::Initializing, PtyStatus::Idle, PtyStatus::Dead] {
            assert_eq!(PtyStatus::from_u8(status.as_u8()), Some(status));
        }
        assert_eq!(PtyStatus::from_u8(99), None);
    }

    #[test]
    fn escalate_kill_zero_pid_is_noop() {
        escalate_kill(0);
    }

    #[test]
    fn freeze_resume_zero_pid_is_noop() {
        freeze_agent_pids(&[0]);
        resume_agent_pids(&[0]);
    }

    #[cfg(windows)]
    #[test]
    fn freeze_resume_roundtrip_on_child_process() {
        use std::process::{Command, Stdio};

        let child = Command::new("cmd")
            .args(["/c", "ping", "-n", "30", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn ping child");
        let pid = child.id();
        freeze_agent_pids(&[pid]);
        std::thread::sleep(Duration::from_millis(50));
        resume_agent_pids(&[pid]);
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .status();
    }

    #[cfg(windows)]
    #[test]
    fn escalate_kill_terminates_child_process() {
        use std::process::{Command, Stdio};

        let child = Command::new("cmd")
            .args(["/c", "ping", "-n", "60", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn ping child");
        let pid = child.id();
        escalate_kill(pid);
        assert!(
            super::windows_process_exited(pid),
            "escalate_kill should terminate pid {pid}"
        );
    }

    #[test]
    #[cfg(feature = "full")]
    fn subagent_exports_compile() {
        use crate::pty::{
            format_subagent_tag, subagent_backend, SubagentBackend, SUBAGENT_CAPTURE_PENDING,
        };
        assert!(!SUBAGENT_CAPTURE_PENDING);
        let _ = format_subagent_tag("mock-1", 1);
        #[cfg(not(target_os = "linux"))]
        assert_eq!(subagent_backend(), SubagentBackend::Polling);
    }

    #[test]
    fn pty_tests_enabled_respects_env() {
        let prev = std::env::var("AGENTHUB_SKIP_PTY").ok();
        std::env::set_var("AGENTHUB_SKIP_PTY", "1");
        assert!(!pty_tests_enabled());
        if let Some(v) = prev {
            std::env::set_var("AGENTHUB_SKIP_PTY", v);
        } else {
            std::env::remove_var("AGENTHUB_SKIP_PTY");
        }
    }

    #[tokio::test]
    #[cfg(feature = "full")]
    async fn spawn_agent_rejects_missing_driver() {
        let cfg = AgentHubConfig::default();
        let state = Arc::new(ServerState::default());
        let (bus_tx, _) = broadcast::channel(16);
        let err = spawn_agent(
            "nonexistent_driver_xyz",
            &cfg,
            state,
            bus_tx,
            None,
            SpawnOptions::default(),
        )
        .await
        .expect_err("missing driver");
        assert!(matches!(
            err,
            AgentHubError::DriverProfile { .. } | AgentHubError::Config(_)
        ));
    }

    #[tokio::test]
    #[cfg(feature = "full")]
    async fn spawn_agent_enforces_max_agents() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (executable, args): (&str, Vec<&str>) = if cfg!(windows) {
            ("cmd", vec!["/c", "echo", "ok"])
        } else {
            ("echo", vec!["ok"])
        };
        let profile = serde_json::json!({
            "name": "mock_echo",
            "display_name": "Mock",
            "executable": executable,
            "args": args,
            "env": { "NO_COLOR": "1", "TERM": "dumb" },
            "prompt_regex": "^>\\s*$",
            "silence_timeout_ms": 3000,
            "init_sequence": [],
            "rate_limit_patterns": [],
            "auto_reply_patterns": {},
            "supports_multi_instance": true,
            "max_instances": 0
        });
        std::fs::write(
            dir.path().join("mock_echo.json"),
            serde_json::to_string(&profile).expect("json"),
        )
        .expect("write");

        let mut cfg = AgentHubConfig::default();
        cfg.drivers_dir = dir.path().to_path_buf();
        cfg.max_agents = 0;

        let state = Arc::new(ServerState::default());
        let (bus_tx, _) = broadcast::channel(16);
        let err = spawn_agent(
            "mock_echo",
            &cfg,
            state,
            bus_tx,
            None,
            SpawnOptions::default(),
        )
        .await
        .expect_err("max agents");
        assert!(matches!(err, AgentHubError::Config(_)));
    }

    #[tokio::test]
    #[cfg(feature = "full")]
    async fn spawn_agent_spawns_process_when_pty_enabled() {
        if !pty_tests_enabled() {
            eprintln!("skip: AGENTHUB_SKIP_PTY is set");
            return;
        }

        let dir = tempfile::tempdir().expect("tempdir");
        let (executable, args): (&str, Vec<&str>) = if cfg!(windows) {
            ("cmd", vec!["/c", "echo", "ok"])
        } else {
            ("echo", vec!["ok"])
        };
        let profile = serde_json::json!({
            "name": "mock_echo",
            "display_name": "Mock",
            "executable": executable,
            "args": args,
            "env": { "NO_COLOR": "1", "TERM": "dumb" },
            "prompt_regex": "^>\\s*$",
            "silence_timeout_ms": 3000,
            "init_sequence": [],
            "rate_limit_patterns": [],
            "auto_reply_patterns": {},
            "supports_multi_instance": true,
            "max_instances": 0
        });
        std::fs::write(
            dir.path().join("mock_echo.json"),
            serde_json::to_string(&profile).expect("json"),
        )
        .expect("write");

        let mut cfg = AgentHubConfig::default();
        cfg.drivers_dir = dir.path().to_path_buf();

        let state = Arc::new(ServerState::default());
        let (bus_tx, _) = broadcast::channel(16);

        let id = spawn_agent(
            "mock_echo",
            &cfg,
            Arc::clone(&state),
            bus_tx,
            None,
            SpawnOptions::default(),
        )
        .await
        .expect("spawn");

        assert!(state.agents.contains_key(&id));
        let agent = state.agents.get(&id).expect("agent");
        assert!(agent.pid > 0);
    }
}
