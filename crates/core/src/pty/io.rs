//! Lock-free ring buffer I/O between PTY reader and sanitizer tasks (blueprint §5.3).

use std::cell::UnsafeCell;
use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::sync::broadcast;

use crate::bus::{BusEvent, OfflineReason};

use super::debug_log::PtyDebugSink;
use super::manager::{AgentPty, PtyStatus};
use super::trace::emit_pty_io_trace;

/// Ring buffer capacity: 64 KiB (power of 2, blueprint §5.3).
pub const RING_CAPACITY: usize = 65536;

const _: () = assert!(RING_CAPACITY.is_power_of_two());
const _: () = assert!(RING_CAPACITY == 65536);

/// Lock-free single-producer, single-consumer ring buffer.
/// Producer: PTY reader task. Consumer: ANSI sanitizer task.
pub struct PtyRingBuffer {
    buffer: UnsafeCell<[u8; RING_CAPACITY]>,
    head: AtomicUsize,
    tail: AtomicUsize,
}

unsafe impl Send for PtyRingBuffer {}
unsafe impl Sync for PtyRingBuffer {}

impl Default for PtyRingBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl PtyRingBuffer {
    pub const fn new() -> Self {
        Self {
            buffer: UnsafeCell::new([0u8; RING_CAPACITY]),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    /// Producer-only write. Returns bytes written (0 if full; reader retries after 1ms).
    pub fn write(&self, data: &[u8]) -> usize {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        let available = RING_CAPACITY - (head.wrapping_sub(tail));
        let to_write = data.len().min(available);
        if to_write == 0 {
            return 0;
        }
        let buf = unsafe { &mut *self.buffer.get() };
        for (i, &byte) in data[..to_write].iter().enumerate() {
            buf[(head + i) % RING_CAPACITY] = byte;
        }
        self.head
            .store(head.wrapping_add(to_write), Ordering::Release);
        to_write
    }

    /// Consumer-only read. Returns bytes read (0 if empty).
    pub fn read(&self, dest: &mut [u8]) -> usize {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        let available = head.wrapping_sub(tail);
        let to_read = dest.len().min(available);
        if to_read == 0 {
            return 0;
        }
        let buf = unsafe { &*self.buffer.get() };
        for i in 0..to_read {
            dest[i] = buf[(tail + i) % RING_CAPACITY];
        }
        self.tail
            .store(tail.wrapping_add(to_read), Ordering::Release);
        to_read
    }

    pub fn is_empty(&self) -> bool {
        self.head.load(Ordering::Acquire) == self.tail.load(Ordering::Relaxed)
    }

    /// Non-destructive read of all unconsumed bytes (for induction ACK polling).
    #[must_use]
    pub fn peek_all(&self) -> Vec<u8> {
        let tail = self.tail.load(Ordering::Acquire);
        let head = self.head.load(Ordering::Acquire);
        let available = head.wrapping_sub(tail);
        if available == 0 {
            return Vec::new();
        }
        let len = available.min(RING_CAPACITY);
        let mut out = vec![0u8; len];
        let buf = unsafe { &*self.buffer.get() };
        for i in 0..len {
            out[i] = buf[(tail + i) % RING_CAPACITY];
        }
        out
    }

    /// Producer write position (for turn-confirmation; sanitizer observes only).
    #[must_use]
    pub fn head(&self) -> usize {
        self.head.load(Ordering::Acquire)
    }

    /// Alias for [`Self::head`] (blueprint §6.2 confirmation window).
    #[must_use]
    pub fn producer_head(&self) -> usize {
        self.head()
    }

    /// Consumer read position.
    #[must_use]
    pub fn tail(&self) -> usize {
        self.tail.load(Ordering::Relaxed)
    }

    /// Returns unread bytes without advancing the consumer tail.
    #[must_use]
    pub fn peek_unread(&self) -> Vec<u8> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        let available = head.wrapping_sub(tail);
        let mut out = vec![0u8; available];
        let buf = unsafe { &*self.buffer.get() };
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = buf[(tail + i) % RING_CAPACITY];
        }
        out
    }
}

/// Dedicated Tokio task: PTY master → ring buffer (blueprint §5.3).
///
/// Uses the `try_clone_reader()` handle stored at spawn time because `MasterPty`
/// in portable-pty 0.8 exposes `Read` only via a cloned reader, not `try_read`.
pub async fn pty_reader_task(
    master_pty: Arc<AgentPty>,
    ring_buffer: Arc<PtyRingBuffer>,
    bus_tx: broadcast::Sender<BusEvent>,
    debug_sink: Option<Arc<PtyDebugSink>>,
    spawn_debug: bool,
) {
    loop {
        if master_pty.status.load(Ordering::Acquire) == PtyStatus::Dead as u8 {
            break;
        }

        let read_result = tokio::task::spawn_blocking({
            let master_pty = Arc::clone(&master_pty);
            move || -> Option<(usize, [u8; 4096])> {
                let mut raw_buf = [0u8; 4096];
                let Ok(mut reader_guard) = master_pty.pty_reader.lock() else {
                    return None;
                };
                let reader = reader_guard.as_mut()?;
                match reader.read(&mut raw_buf) {
                    Ok(0) => Some((0, raw_buf)),
                    Ok(n) => Some((n, raw_buf)),
                    Err(_) => None,
                }
            }
        })
        .await
        .ok()
        .flatten();

        let Some((bytes_read, raw_buf)) = read_result else {
            break;
        };

        if bytes_read == 0 {
            master_pty
                .status
                .store(PtyStatus::Dead.as_u8(), Ordering::Release);
            let tag = master_pty.tag.clone();
            let _ = bus_tx.send(BusEvent::AgentOffline {
                id: master_pty.id,
                tag,
                reason: OfflineReason::Natural,
            });
            break;
        }

        if let Some(sink) = debug_sink.as_ref() {
            sink.record(&raw_buf[..bytes_read]);
        }
        emit_pty_io_trace(
            &bus_tx,
            &master_pty.tag,
            "out",
            &raw_buf[..bytes_read],
            spawn_debug,
        );

        let mut written = 0;
        while written < bytes_read {
            let w = ring_buffer.write(&raw_buf[written..bytes_read]);
            if w == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            } else {
                written += w;
            }
        }
    }
}

/// Stdio pipe reader → ring buffer (CI mock path when `AGENTHUB_SKIP_PTY=1`).
///
/// Uses a dedicated OS thread so blocking `read()` does not exhaust the Tokio blocking pool.
pub async fn stdio_reader_task(
    agent: Arc<AgentPty>,
    ring_buffer: Arc<PtyRingBuffer>,
    bus_tx: broadcast::Sender<BusEvent>,
    debug_sink: Option<Arc<PtyDebugSink>>,
    spawn_debug: bool,
) {
    let (done_tx, mut done_rx) = tokio::sync::mpsc::channel::<()>(1);
    let thread_agent = Arc::clone(&agent);
    let thread_ring = Arc::clone(&ring_buffer);
    let thread_bus = bus_tx.clone();
    let thread_debug = debug_sink;
    let trace_enabled = spawn_debug;

    std::thread::spawn(move || {
        loop {
            if thread_agent.status.load(Ordering::Acquire) == PtyStatus::Dead.as_u8() {
                break;
            }

            let mut raw_buf = [0u8; 4096];
            let read_result = {
                let Ok(mut reader_guard) = thread_agent.pty_reader.lock() else {
                    break;
                };
                let Some(reader) = reader_guard.as_mut() else {
                    break;
                };
                reader.read(&mut raw_buf)
            };

            let (bytes_read, raw_buf) = match read_result {
                Ok(0) => (0, raw_buf),
                Ok(n) => (n, raw_buf),
                Err(_) => break,
            };

            if bytes_read == 0 {
                thread_agent
                    .status
                    .store(PtyStatus::Dead.as_u8(), Ordering::Release);
                let tag = thread_agent.tag.clone();
                let _ = thread_bus.send(BusEvent::AgentOffline {
                    id: thread_agent.id,
                    tag,
                    reason: OfflineReason::Natural,
                });
                break;
            }

            if let Some(sink) = thread_debug.as_ref() {
                sink.record(&raw_buf[..bytes_read]);
            }
            emit_pty_io_trace(
                &thread_bus,
                &thread_agent.tag,
                "out",
                &raw_buf[..bytes_read],
                trace_enabled,
            );

            let mut written = 0;
            while written < bytes_read {
                let w = thread_ring.write(&raw_buf[written..bytes_read]);
                if w == 0 {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                } else {
                    written += w;
                }
            }
        }
        let _ = done_tx.try_send(());
    });

    let _ = done_rx.recv().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read_roundtrip() {
        let rb = PtyRingBuffer::new();
        let data = b"hello, agenthub";
        assert_eq!(rb.write(data), data.len());
        assert!(!rb.is_empty());

        let mut out = vec![0u8; data.len()];
        assert_eq!(rb.read(&mut out), data.len());
        assert_eq!(&out[..], data);
        assert!(rb.is_empty());
    }

    #[test]
    fn full_buffer_behavior() {
        let rb = PtyRingBuffer::new();
        let chunk = vec![0xABu8; RING_CAPACITY];
        assert_eq!(rb.write(&chunk), RING_CAPACITY);
        assert_eq!(rb.write(&[0xFF]), 0);
        assert!(!rb.is_empty());

        let mut partial = [0u8; 1000];
        assert_eq!(rb.read(&mut partial), 1000);
        assert!(partial.iter().all(|&b| b == 0xAB));

        let mut rest = vec![0u8; RING_CAPACITY - 1000];
        assert_eq!(rb.read(&mut rest), RING_CAPACITY - 1000);
        assert!(rest.iter().all(|&b| b == 0xAB));
        assert!(rb.is_empty());

        assert_eq!(rb.write(&[1, 2, 3]), 3);
        let mut tail = [0u8; 3];
        assert_eq!(rb.read(&mut tail), 3);
        assert_eq!(tail, [1, 2, 3]);
    }

    #[test]
    fn wrap_around_indexing() {
        let rb = PtyRingBuffer::new();
        let first = vec![0xAAu8; RING_CAPACITY - 4];
        let second = b"tail";
        assert_eq!(rb.write(&first), first.len());
        assert_eq!(rb.write(second), second.len());

        let mut out = vec![0u8; RING_CAPACITY];
        assert_eq!(rb.read(&mut out), RING_CAPACITY);
        assert_eq!(&out[..first.len()], first.as_slice());
        assert_eq!(&out[first.len()..], second);
        assert!(rb.is_empty());
    }
}
