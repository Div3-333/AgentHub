//! PTY → bus bridge for pipeline/spar tests when the sanitizer task is not running.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use regex::Regex;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use crate::bus::BusEvent;
use crate::config::DriverProfile;
use crate::pty::manager::{AgentPty, PtyStatus};
use crate::sanitizer::parser::VirtualGrid;
use crate::sanitizer::{last_non_empty_line, CONFIRMATION_MS};

const DRAIN_POLL_MS: u64 = 10;

/// Spawns a background task that emits [`BusEvent::AgentMessage`] when the mock CLI finishes a turn.
pub fn spawn_agent_message_bridge(
    agent: Arc<AgentPty>,
    driver: DriverProfile,
    bus_tx: broadcast::Sender<BusEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let ring = agent.ring_buffer();
        let prompt_regex = match Regex::new(&driver.prompt_regex) {
            Ok(r) => r,
            Err(_) => return,
        };
        let mut grid = VirtualGrid::new();
        let mut read_buf = [0u8; 4096];
        let mut last_byte = Instant::now();
        let mut emitted_for_turn = false;
        let mut prev_status = agent.status();

        loop {
            if agent.status.load(Ordering::Acquire) == PtyStatus::Dead.as_u8() {
                break;
            }

            let current_status = agent.status();
            if matches!(
                (prev_status, current_status),
                (
                    Some(PtyStatus::Idle) | Some(PtyStatus::Initializing),
                    Some(PtyStatus::Thinking)
                )
            ) {
                last_byte = Instant::now();
                emitted_for_turn = false;
            }
            prev_status = current_status;

            let mut got_bytes = false;
            loop {
                let n = ring.read(&mut read_buf);
                if n == 0 {
                    break;
                }
                got_bytes = true;
                last_byte = Instant::now();
                emitted_for_turn = false;
                grid.feed(&read_buf[..n]);
            }

            let text = grid.extract_text();
            let prompt_visible = last_non_empty_line(&text)
                .is_some_and(|line| prompt_regex.is_match(trim_pty_line(line)));

            if !emitted_for_turn
                && prompt_visible
                && last_byte.elapsed() >= Duration::from_millis(CONFIRMATION_MS)
            {
                let content = extract_response_body(&text, &prompt_regex);
                if !content.is_empty() {
                    let _ = bus_tx.send(BusEvent::AgentMessage {
                        id: agent.id,
                        tag: agent.tag.clone(),
                        content,
                        timestamp: Utc::now(),
                        race_session_id: None,
                    });
                    emitted_for_turn = true;
                    grid = VirtualGrid::new();
                    agent
                        .status
                        .store(PtyStatus::Idle.as_u8(), Ordering::Release);
                }
            }

            if !got_bytes {
                tokio::time::sleep(Duration::from_millis(DRAIN_POLL_MS)).await;
            }
        }
    })
}

fn trim_pty_line(line: &str) -> &str {
    line.trim_end_matches('\r').trim()
}

fn extract_response_body(text: &str, prompt_regex: &Regex) -> String {
    let lines: Vec<&str> = text
        .lines()
        .map(trim_pty_line)
        .filter(|line| !line.is_empty())
        .collect();

    let Some(last_prompt) = lines.iter().rposition(|line| prompt_regex.is_match(line)) else {
        return String::new();
    };

    let prev_prompt = lines
        .iter()
        .take(last_prompt)
        .rposition(|line| prompt_regex.is_match(line))
        .map(|idx| idx + 1)
        .unwrap_or(0);

    lines[prev_prompt..last_prompt]
        .iter()
        .filter(|line| !prompt_regex.is_match(line))
        .copied()
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}
