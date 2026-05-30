//! Turn-detection heuristics and sanitizer task (blueprint §6.2).

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use regex::Regex;
use tokio::sync::broadcast;
use tokio::time::{sleep, MissedTickBehavior};
use tracing::{debug, warn};

use crate::bus::BusEvent;
use crate::config::DriverProfile;
use crate::error::{AgentHubError, Result};
#[cfg(test)]
use crate::pty::io::PtyRingBuffer;
use crate::pty::manager::{AgentPty, PtyStatus};
use crate::sanitizer::parser::{last_non_empty_line, VirtualGrid};
use crate::server::{modes, ServerState};

/// Milliseconds of output stability required before a prompt-regex match is accepted.
pub const CONFIRMATION_MS: u64 = 100;
/// Interval for the silence-timeout background check.
pub const SILENCE_POLL_MS: u64 = 500;
/// Ring-buffer poll interval when no bytes are available.
const DRAIN_POLL_MS: u64 = 10;
const READ_CHUNK: usize = 4096;

/// Detects agent turn completion via the driver `prompt_regex`.
pub struct TurnDetector {
    prompt_regex: Regex,
}

impl TurnDetector {
    /// Builds a detector from a pre-compiled prompt regex (compile at agent spawn time).
    #[must_use]
    pub fn new(prompt_regex: Regex) -> Self {
        Self { prompt_regex }
    }

    /// Compiles `driver.prompt_regex` after validation.
    pub fn from_driver(driver: &DriverProfile) -> Result<Self> {
        let prompt_regex =
            Regex::new(&driver.prompt_regex).map_err(|e| AgentHubError::DriverProfile {
                driver: driver.name.clone(),
                msg: format!("invalid prompt_regex: {e}"),
            })?;
        Ok(Self::new(prompt_regex))
    }

    /// Returns true when the last non-empty line matches the driver's prompt pattern.
    #[must_use]
    pub fn is_prompt_visible(&self, extracted_text: &str) -> bool {
        last_non_empty_line(extracted_text).is_some_and(|line| self.prompt_regex.is_match(line))
    }

    /// Blueprint Phase 3 DoD alias: prompt visible on sanitized text (confirmation is task-level).
    #[must_use]
    pub fn is_turn_complete(&self, extracted_text: &str) -> bool {
        self.is_prompt_visible(extracted_text)
    }
}

/// Compiled auto-reply patterns from a [`DriverProfile`].
pub struct AutoReplies {
    entries: Vec<(Regex, String)>,
}

impl AutoReplies {
    /// Compiles all `auto_reply_patterns` keys at spawn time.
    pub fn from_driver(driver: &DriverProfile) -> Result<Self> {
        let mut entries = Vec::with_capacity(driver.auto_reply_patterns.len());
        for (pattern, reply) in &driver.auto_reply_patterns {
            let regex = Regex::new(pattern).map_err(|e| AgentHubError::DriverProfile {
                driver: driver.name.clone(),
                msg: format!("invalid auto_reply pattern `{pattern}`: {e}"),
            })?;
            entries.push((regex, reply.clone()));
        }
        Ok(Self { entries })
    }

    /// If the last line matches a pattern, returns the reply string to inject.
    #[must_use]
    pub fn match_reply(&self, extracted_text: &str) -> Option<(&str, &str)> {
        let last = last_non_empty_line(extracted_text)?;
        for (regex, reply) in &self.entries {
            if regex.is_match(last) {
                return Some((regex.as_str(), reply.as_str()));
            }
        }
        None
    }

    /// Like [`Self::match_reply`] but scans every non-empty line (multi-line TUI prompts).
    #[must_use]
    pub fn match_reply_any_line(&self, extracted_text: &str) -> Option<(&str, &str)> {
        for line in extracted_text.lines() {
            let trimmed = line.trim_end_matches('\r').trim();
            if trimmed.is_empty() {
                continue;
            }
            for (regex, reply) in &self.entries {
                if regex.is_match(trimmed) {
                    return Some((regex.as_str(), reply.as_str()));
                }
            }
        }
        None
    }
}

/// Pending 100ms confirmation after a prompt-regex match (blueprint §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingConfirmation {
    head_snapshot: usize,
    since: Instant,
}

/// Returns true when the confirmation window elapsed and the ring head is unchanged.
#[must_use]
fn confirmation_ready(pending: PendingConfirmation, current_head: usize, now: Instant) -> bool {
    current_head == pending.head_snapshot
        && now.duration_since(pending.since) >= Duration::from_millis(CONFIRMATION_MS)
}

/// Returns true when silence timeout should fire for a thinking agent.
#[must_use]
fn silence_timeout_elapsed(
    status: PtyStatus,
    last_byte: Instant,
    silence_timeout_ms: u64,
    now: Instant,
) -> bool {
    status == PtyStatus::Thinking
        && now.duration_since(last_byte) >= Duration::from_millis(silence_timeout_ms)
}

fn set_agent_status(agent: &AgentPty, new_status: PtyStatus, bus_tx: &broadcast::Sender<BusEvent>) {
    let old = agent.status.swap(new_status.as_u8(), Ordering::AcqRel);
    if old != new_status.as_u8() {
        let _ = bus_tx.send(BusEvent::AgentStatusChanged {
            id: agent.id,
            old,
            new: new_status.as_u8(),
        });
    }
}

fn emit_agent_message(agent: &AgentPty, content: String, bus_tx: &broadcast::Sender<BusEvent>) {
    let _ = bus_tx.send(BusEvent::AgentMessage {
        id: agent.id,
        tag: agent.tag.clone(),
        content,
        timestamp: Utc::now(),
        race_session_id: None,
    });
}

fn complete_turn(
    agent: &AgentPty,
    grid: &mut VirtualGrid,
    state: &ServerState,
    bus_tx: &broadcast::Sender<BusEvent>,
    via_silence: bool,
) {
    let content = grid.extract_text();
    if via_silence {
        warn!(
            agent = %agent.tag,
            "[Warning]: Turn completed via silence timeout for @{}. \
             Consider updating the driver's prompt_regex.",
            agent.tag
        );
    }

    match modes::gate_agent_bus_output(state, agent.id, &content) {
        Ok(()) => emit_agent_message(agent, content, bus_tx),
        Err(AgentHubError::WriteFilesBlocked { .. }) => {
            let _ = bus_tx.send(BusEvent::SystemMessage {
                content: format!(
                    "[RBAC]: Blocked output from @{}: filesystem write without WRITE_FILES",
                    agent.tag
                ),
                timestamp: Utc::now(),
            });
        }
        Err(AgentHubError::PermissionDenied { permission, .. }) => {
            let _ = bus_tx.send(BusEvent::SystemMessage {
                content: format!(
                    "[RBAC]: Blocked output from @{}: missing {permission}",
                    agent.tag
                ),
                timestamp: Utc::now(),
            });
        }
        Err(e) => warn!(agent = %agent.tag, "agent output gate failed: {e}"),
    }
    set_agent_status(agent, PtyStatus::Idle, bus_tx);
    *grid = VirtualGrid::new();
}

fn try_auto_reply(agent: &AgentPty, auto_replies: &AutoReplies, extracted_text: &str) -> bool {
    let Some((pattern, reply)) = auto_replies
        .match_reply(extracted_text)
        .or_else(|| auto_replies.match_reply_any_line(extracted_text))
    else {
        return false;
    };
    match agent.write_stdin(reply.as_bytes()) {
        Ok(_) => {
            debug!(
                agent = %agent.tag,
                "[Auto-reply]: Sent '{reply}' to @{tag} for prompt '{pattern}'",
                tag = agent.tag,
                reply = reply.escape_default(),
                pattern = pattern,
            );
            true
        }
        Err(e) => {
            warn!(agent = %agent.tag, "auto-reply stdin write failed: {e}");
            false
        }
    }
}

/// Consumes PTY output from the ring buffer, sanitizes via [`VirtualGrid`], and emits
/// [`BusEvent::AgentMessage`] when a turn completes (blueprint §6.2).
pub async fn sanitizer_task(
    agent: Arc<AgentPty>,
    driver: DriverProfile,
    state: Arc<ServerState>,
    bus_tx: broadcast::Sender<BusEvent>,
) {
    let turn_detector = match TurnDetector::from_driver(&driver) {
        Ok(td) => td,
        Err(e) => {
            warn!(agent = %agent.tag, "sanitizer_task: {e}");
            return;
        }
    };
    let auto_replies = match AutoReplies::from_driver(&driver) {
        Ok(ar) => ar,
        Err(e) => {
            warn!(agent = %agent.tag, "sanitizer_task: {e}");
            return;
        }
    };

    let ring = agent.ring_buffer();
    let mut grid = VirtualGrid::new();
    let mut read_buf = [0u8; READ_CHUNK];
    let mut last_byte = Instant::now();
    let mut pending_confirm: Option<PendingConfirmation> = None;
    let silence_timeout_ms = driver.silence_timeout_ms;

    let mut silence_tick = tokio::time::interval(Duration::from_millis(SILENCE_POLL_MS));
    silence_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut prev_status = agent.status();

    loop {
        if agent.status.load(Ordering::Acquire) == PtyStatus::Dead.as_u8() {
            break;
        }

        // Bus injection sets Thinking before PTY output arrives; reset the silence
        // baseline so we do not immediately complete an empty turn on stale `last_byte`.
        let current_status = agent.status();
        if matches!(
            (prev_status, current_status),
            (
                Some(PtyStatus::Idle) | Some(PtyStatus::Initializing),
                Some(PtyStatus::Thinking)
            )
        ) {
            last_byte = Instant::now();
            pending_confirm = None;
        }
        prev_status = current_status;

        let mut processed_bytes = false;
        loop {
            let n = ring.read(&mut read_buf);
            if n == 0 {
                break;
            }
            processed_bytes = true;
            last_byte = Instant::now();
            if agent.status() == Some(PtyStatus::Idle) {
                set_agent_status(&agent, PtyStatus::Thinking, &bus_tx);
            }
            grid.write_bytes(&read_buf[..n]);

            let extracted = grid.extract_text();
            if try_auto_reply(&agent, &auto_replies, &extracted) {
                // Blueprint §6.2: auto-reply prompts are control-plane only and must not
                // leak into user-visible output.
                grid = VirtualGrid::new();
                pending_confirm = None;
                continue;
            }

            if turn_detector.is_prompt_visible(&extracted) {
                pending_confirm = Some(PendingConfirmation {
                    head_snapshot: ring.head(),
                    since: Instant::now(),
                });
            } else if let Some(pending) = pending_confirm {
                if ring.head() != pending.head_snapshot {
                    pending_confirm = None;
                }
            }
        }

        let now = Instant::now();
        if let Some(pending) = pending_confirm {
            let head = ring.head();
            if head != pending.head_snapshot {
                pending_confirm = None;
            } else if confirmation_ready(pending, head, now) {
                complete_turn(&agent, &mut grid, &state, &bus_tx, false);
                pending_confirm = None;
            }
        }

        if processed_bytes {
            continue;
        }

        tokio::select! {
            _ = sleep(Duration::from_millis(DRAIN_POLL_MS)) => {}
            _ = silence_tick.tick() => {
                let now = Instant::now();
                let status = match agent.status() {
                    Some(s) => s,
                    None => PtyStatus::Dead,
                };
                if silence_timeout_elapsed(status, last_byte, silence_timeout_ms, now) {
                    complete_turn(&agent, &mut grid, &state, &bus_tx, true);
                    pending_confirm = None;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::AtomicU8;
    use std::sync::Arc;

    use crate::config::load_driver_profile_from_dir;
    use crate::config::{bundled_drivers_dir, DriverProfile};

    fn sample_driver(prompt_regex: &str) -> DriverProfile {
        DriverProfile {
            name: "test".into(),
            display_name: "Test".into(),
            executable: "echo".into(),
            args: vec![],
            env: HashMap::new(),
            prompt_regex: prompt_regex.into(),
            silence_timeout_ms: 50,
            init_sequence: vec![],
            rate_limit_patterns: vec![],
            auto_reply_patterns: HashMap::new(),
            supports_multi_instance: true,
            max_instances: 0,
        }
    }

    #[test]
    fn turn_detector_matches_prompt_line() {
        let driver = sample_driver("^>\\s*$");
        let td = TurnDetector::from_driver(&driver).expect("regex");
        assert!(td.is_prompt_visible("Hello\nworld\n> "));
        assert!(!td.is_prompt_visible("Hello\nworld\nstill typing"));
    }

    #[test]
    fn confirmation_ready_requires_stable_head() {
        let since = Instant::now();
        let pending = PendingConfirmation {
            head_snapshot: 42,
            since: since - Duration::from_millis(CONFIRMATION_MS),
        };
        assert!(confirmation_ready(pending, 42, Instant::now()));
        assert!(!confirmation_ready(pending, 43, Instant::now()));
    }

    #[test]
    fn confirmation_not_ready_before_window() {
        let pending = PendingConfirmation {
            head_snapshot: 10,
            since: Instant::now(),
        };
        assert!(!confirmation_ready(pending, 10, Instant::now()));
    }

    #[test]
    fn silence_timeout_only_when_thinking() {
        let last = Instant::now() - Duration::from_millis(100);
        assert!(silence_timeout_elapsed(
            PtyStatus::Thinking,
            last,
            50,
            Instant::now()
        ));
        assert!(!silence_timeout_elapsed(
            PtyStatus::Idle,
            last,
            50,
            Instant::now()
        ));
    }

    #[test]
    fn auto_reply_matches_and_returns_payload() {
        let mut driver = sample_driver("^>\\s*$");
        driver
            .auto_reply_patterns
            .insert(r"Continue\? \(Y/n\)".to_string(), "Y\n".to_string());
        let replies = AutoReplies::from_driver(&driver).expect("patterns");
        let (pattern, reply) = replies
            .match_reply("Working...\nContinue? (Y/n)\n")
            .expect("match");
        assert!(pattern.contains("Continue"));
        assert_eq!(reply, "Y\n");
    }

    #[test]
    fn auto_reply_any_line_matches_trust_folder_prompt() {
        let mut driver = sample_driver("^>\\s*$");
        driver.auto_reply_patterns.insert(
            "Do you trust the files in this folder".to_string(),
            "1\n".to_string(),
        );
        let replies = AutoReplies::from_driver(&driver).expect("patterns");
        let text = "│ Do you trust the files in this folder?          │\n│ 1. Trust folder (Downloads)                     │\n";
        let (_, reply) = replies
            .match_reply_any_line(text)
            .expect("trust prompt");
        assert_eq!(reply, "1\n");
    }

    #[test]
    fn bundled_driver_prompt_regexes_compile() {
        let dir = bundled_drivers_dir();
        for name in ["gemini", "claude", "codex", "cursor", "aider"] {
            let profile = load_driver_profile_from_dir(&dir, name)
                .unwrap_or_else(|e| panic!("load {name}: {e}"));
            let td = TurnDetector::from_driver(&profile).expect("turn detector");
            assert!(
                !profile.prompt_regex.is_empty(),
                "{name} prompt_regex must not be empty"
            );
            let _ = td;
        }
    }

    #[test]
    fn ring_head_advances_cancel_confirmation() {
        let ring = PtyRingBuffer::new();
        let h0 = ring.head();
        ring.write(b"more output");
        assert!(ring.head() > h0);
        let pending = PendingConfirmation {
            head_snapshot: h0,
            since: Instant::now() - Duration::from_millis(CONFIRMATION_MS),
        };
        assert!(!confirmation_ready(pending, ring.head(), Instant::now()));
    }

    async fn run_turn_detection_on_ring(
        ring: Arc<PtyRingBuffer>,
        driver: DriverProfile,
        status: Arc<AtomicU8>,
    ) -> Option<String> {
        let turn_detector = TurnDetector::from_driver(&driver).ok()?;
        let auto_replies = AutoReplies::from_driver(&driver).ok()?;
        let mut grid = VirtualGrid::new();
        let mut read_buf = [0u8; READ_CHUNK];
        let mut pending_confirm: Option<PendingConfirmation> = None;
        let mut last_byte = Instant::now();

        for _ in 0..200 {
            let n = ring.read(&mut read_buf);
            if n > 0 {
                last_byte = Instant::now();
                grid.write_bytes(&read_buf[..n]);
                let extracted = grid.extract_text();
                if auto_replies.match_reply(&extracted).is_some() {
                    pending_confirm = None;
                    sleep(Duration::from_millis(5)).await;
                    continue;
                }
                if turn_detector.is_prompt_visible(&extracted) {
                    pending_confirm = Some(PendingConfirmation {
                        head_snapshot: ring.head(),
                        since: Instant::now(),
                    });
                }
            }

            if let Some(pending) = pending_confirm {
                let head = ring.head();
                if head != pending.head_snapshot {
                    pending_confirm = None;
                } else if confirmation_ready(pending, head, Instant::now()) {
                    return Some(grid.extract_text());
                }
            }

            let st = PtyStatus::from_u8(status.load(Ordering::Acquire)).unwrap_or(PtyStatus::Dead);
            if silence_timeout_elapsed(st, last_byte, driver.silence_timeout_ms, Instant::now()) {
                return Some(grid.extract_text());
            }

            sleep(Duration::from_millis(15)).await;
        }
        None
    }

    #[tokio::test]
    async fn sanitizer_detects_gemini_style_prompt() {
        let ring = Arc::new(PtyRingBuffer::new());
        let status = Arc::new(AtomicU8::new(PtyStatus::Thinking.as_u8()));
        let driver = sample_driver("^>\\s*$");

        let ring_clone = Arc::clone(&ring);
        let status_clone = Arc::clone(&status);
        let driver_clone = driver.clone();
        let detect = tokio::spawn(async move {
            run_turn_detection_on_ring(ring_clone, driver_clone, status_clone).await
        });

        tokio::time::sleep(Duration::from_millis(20)).await;
        ring.write(b"Here is the answer.\n\n> ");
        tokio::time::sleep(Duration::from_millis(CONFIRMATION_MS + 50)).await;

        let text = detect.await.expect("join").expect("turn detected");
        assert!(text.contains("Here is the answer"));
        assert!(text.contains('>'));
    }

    #[tokio::test]
    async fn sanitizer_prompt_match_cancelled_when_head_advances() {
        let ring = Arc::new(PtyRingBuffer::new());
        let status = Arc::new(AtomicU8::new(PtyStatus::Thinking.as_u8()));
        let driver = sample_driver("^>\\s*$");

        ring.write(b"partial\n> ");
        tokio::time::sleep(Duration::from_millis(30)).await;
        ring.write(b"still generating\n");
        tokio::time::sleep(Duration::from_millis(CONFIRMATION_MS + 30)).await;
        ring.write(b"> ");
        tokio::time::sleep(Duration::from_millis(CONFIRMATION_MS + 50)).await;

        let ring_c = Arc::clone(&ring);
        let status_c = Arc::clone(&status);
        let text = run_turn_detection_on_ring(ring_c, driver, status_c)
            .await
            .expect("eventually completes");
        assert!(text.contains("still generating"));
    }

    #[tokio::test]
    async fn prompt_turn_detected_within_200ms() {
        let ring = Arc::new(PtyRingBuffer::new());
        let status = Arc::new(AtomicU8::new(PtyStatus::Thinking.as_u8()));
        let driver = sample_driver("^>\\s*$");

        let ring_clone = Arc::clone(&ring);
        let status_clone = Arc::clone(&status);
        let detect = tokio::spawn(async move {
            run_turn_detection_on_ring(ring_clone, driver, status_clone).await
        });

        tokio::time::sleep(Duration::from_millis(10)).await;
        ring.write(b"Done.\n> ");
        let text = tokio::time::timeout(Duration::from_millis(200), detect)
            .await
            .expect("within 200ms")
            .expect("join")
            .expect("turn detected");
        assert!(text.contains("Done."));
    }

    #[tokio::test]
    async fn sanitizer_task_emits_agent_message_after_confirmation() {
        let (agent, _capture) = crate::pty::manager::mock_agent_with_capture(
            uuid::Uuid::new_v4(),
            "gemini",
            PtyStatus::Thinking,
            true,
        );
        let ring = agent.ring_buffer();
        let (bus_tx, mut bus_rx) = broadcast::channel(16);
        let driver = sample_driver("^>\\s*$");
        let state = Arc::new(ServerState::new());

        let agent_c = Arc::clone(&agent);
        let task = tokio::spawn(sanitizer_task(agent_c, driver, state, bus_tx));

        tokio::time::sleep(Duration::from_millis(20)).await;
        ring.write(b"Answer body.\n\n> ");
        tokio::time::sleep(Duration::from_millis(CONFIRMATION_MS + 80)).await;

        let event = tokio::time::timeout(Duration::from_millis(500), async {
            loop {
                if let Ok(BusEvent::AgentMessage { content, tag, .. }) = bus_rx.recv().await {
                    return (tag, content);
                }
            }
        })
        .await
        .expect("message within 500ms");

        assert_eq!(event.0, "gemini");
        assert!(event.1.contains("Answer body"));

        agent
            .status
            .store(PtyStatus::Dead.as_u8(), Ordering::Release);
        let _ = task.await;
    }

    #[tokio::test]
    async fn sanitizer_task_auto_reply_writes_stdin_without_bus_message() {
        let (agent, stdin_capture) = crate::pty::manager::mock_agent_with_capture(
            uuid::Uuid::new_v4(),
            "claude",
            PtyStatus::Thinking,
            true,
        );
        let ring = agent.ring_buffer();
        let (bus_tx, mut bus_rx) = broadcast::channel(16);
        let mut driver = sample_driver("^>\\s*$");
        driver
            .auto_reply_patterns
            .insert(r"Continue\? \(Y/n\)".to_string(), "Y\n".to_string());
        let state = Arc::new(ServerState::new());

        let agent_c = Arc::clone(&agent);
        let task = tokio::spawn(sanitizer_task(agent_c, driver, state, bus_tx));

        tokio::time::sleep(Duration::from_millis(20)).await;
        ring.write(b"Working...\nContinue? (Y/n)\n");
        tokio::time::sleep(Duration::from_millis(CONFIRMATION_MS + 100)).await;

        let stdin = stdin_capture.lock().expect("stdin cap");
        assert_eq!(std::str::from_utf8(&stdin).expect("utf8"), "Y\n");

        let maybe_msg = tokio::time::timeout(Duration::from_millis(50), bus_rx.recv()).await;
        assert!(
            maybe_msg.is_err(),
            "auto-reply must not emit BusEvent::AgentMessage"
        );

        agent
            .status
            .store(PtyStatus::Dead.as_u8(), Ordering::Release);
        let _ = task.await;
    }

    #[tokio::test]
    async fn sanitizer_task_silence_timeout_when_thinking() {
        let (agent, _capture) = crate::pty::manager::mock_agent_with_capture(
            uuid::Uuid::new_v4(),
            "slow",
            PtyStatus::Thinking,
            true,
        );
        let ring = agent.ring_buffer();
        let (bus_tx, mut bus_rx) = broadcast::channel(16);
        let driver = sample_driver(r"^NEVER_MATCH_PROMPT_\d+$");

        let state = Arc::new(ServerState::new());
        let agent_c = Arc::clone(&agent);
        let sleep_ms = driver.silence_timeout_ms + SILENCE_POLL_MS + 50;
        let task = tokio::spawn(sanitizer_task(agent_c, driver, state, bus_tx));

        ring.write(b"partial output without prompt\n");
        tokio::time::sleep(Duration::from_millis(sleep_ms)).await;

        let event = tokio::time::timeout(Duration::from_millis(SILENCE_POLL_MS + 200), async {
            loop {
                if let Ok(BusEvent::AgentMessage { content, .. }) = bus_rx.recv().await {
                    return content;
                }
            }
        })
        .await
        .expect("silence timeout should emit");

        assert!(event.contains("partial output"));

        agent
            .status
            .store(PtyStatus::Dead.as_u8(), Ordering::Release);
        let _ = task.await;
    }
}
