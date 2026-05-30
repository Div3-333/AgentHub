//! Live spawn / PTY trace helpers (optional `spawn_debug` in config).

use chrono::Utc;
use tokio::sync::broadcast;

use crate::bus::BusEvent;

/// Short, chat-safe preview of raw PTY bytes.
#[must_use]
pub fn preview_pty_bytes(data: &[u8], max_chars: usize) -> String {
    let text = String::from_utf8_lossy(data);
    let mut out = String::new();
    for ch in text.chars().take(max_chars) {
        if ch == '\n' || ch == '\r' || ch == '\t' || !ch.is_control() {
            out.push(ch);
        } else {
            out.push('·');
        }
    }
    if text.chars().count() > max_chars {
        out.push('…');
    }
    out
}

pub fn emit_spawn_trace(
    bus_tx: &broadcast::Sender<BusEvent>,
    tag: &str,
    message: impl Into<String>,
    enabled: bool,
) {
    if !enabled {
        return;
    }
    let _ = bus_tx.send(BusEvent::SpawnTrace {
        tag: tag.to_string(),
        message: message.into(),
        timestamp: Utc::now(),
    });
}

pub fn emit_pty_io_trace(
    bus_tx: &broadcast::Sender<BusEvent>,
    tag: &str,
    direction: &str,
    data: &[u8],
    enabled: bool,
) {
    if !enabled || data.is_empty() {
        return;
    }
    let preview = preview_pty_bytes(data, 240);
    if preview.trim().is_empty() {
        return;
    }
    let _ = bus_tx.send(BusEvent::PtyIoTrace {
        tag: tag.to_string(),
        direction: direction.to_string(),
        preview,
        timestamp: Utc::now(),
    });
}
