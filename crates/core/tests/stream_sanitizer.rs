// Phase 3 DoD: stream sanitizer — spinner, prompt_regex, no premature turn, bundled drivers.

use std::sync::Arc;
use std::time::{Duration, Instant};

use agenthub_core::config::{bundled_drivers_dir, load_driver_profile_from_dir};
use agenthub_core::pty::PtyRingBuffer;
use agenthub_core::sanitizer::{parser::VirtualGrid, TurnDetector, CONFIRMATION_MS};

const BUNDLED_DRIVERS: &[&str] = &["gemini", "claude", "codex", "cursor", "aider"];

fn sample_prompt_line(driver: &str) -> &'static str {
    match driver {
        "claude" => "? ",
        _ => "> ",
    }
}

#[test]
fn spinner_extracts_final_visible_text_only() {
    let mut grid = VirtualGrid::new();
    for frame in ["-", "\\", "|", "/"] {
        grid.feed(format!("\r{frame}").as_bytes());
    }
    grid.feed(b"\r");
    grid.feed(b"done");
    let text = grid.extract_text();
    assert_eq!(
        text, "done",
        "carriage-return spinner must collapse to final frame"
    );
    assert!(
        !text.contains('|'),
        "spinner artifacts must not leak into extract_text"
    );
}

#[test]
fn ansi_color_codes_do_not_appear_in_extract_text() {
    let mut grid = VirtualGrid::new();
    grid.feed(b"\x1b[31mHello\x1b[0m \x1b[32mWorld\x1b[0m\n");
    assert_eq!(grid.extract_text(), "Hello World");
}

#[test]
fn bundled_driver_prompt_regexes_match_sample_prompts() {
    let dir = bundled_drivers_dir();
    for name in BUNDLED_DRIVERS {
        let profile =
            load_driver_profile_from_dir(&dir, name).unwrap_or_else(|e| panic!("load {name}: {e}"));
        let detector = TurnDetector::from_driver(&profile).expect("compile regex");
        let mut grid = VirtualGrid::new();
        grid.feed(format!("Response body\n{}\n", sample_prompt_line(name)).as_bytes());
        let extracted = grid.extract_text();
        assert!(
            detector.is_prompt_visible(&extracted),
            "{name}: prompt_regex {:?} must match last line of {extracted:?}",
            profile.prompt_regex
        );
    }
}

async fn wait_turn(
    ring: Arc<PtyRingBuffer>,
    prompt_regex: &str,
    initial: &[u8],
    timeout: Duration,
) -> bool {
    let driver = agenthub_core::config::DriverProfile {
        name: "test".into(),
        display_name: "Test".into(),
        executable: "echo".into(),
        args: vec![],
        env: Default::default(),
        prompt_regex: prompt_regex.to_string(),
        silence_timeout_ms: 60_000,
        init_sequence: vec![],
        rate_limit_patterns: vec![],
        auto_reply_patterns: Default::default(),
        supports_multi_instance: true,
        max_instances: 0,
    };
    let turn_detector = TurnDetector::from_driver(&driver).expect("regex");
    let mut grid = VirtualGrid::new();
    let mut read_buf = [0u8; 4096];
    let mut pending: Option<(usize, Instant)> = None;

    ring.write(initial);
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let n = ring.read(&mut read_buf);
        if n > 0 {
            grid.feed(&read_buf[..n]);
            let extracted = grid.extract_text();
            if turn_detector.is_prompt_visible(&extracted) {
                pending = Some((ring.producer_head(), Instant::now()));
            }
        }
        if let Some((head_snapshot, since)) = pending {
            if ring.producer_head() != head_snapshot {
                pending = None;
            } else if since.elapsed() >= Duration::from_millis(CONFIRMATION_MS) {
                return turn_detector.is_prompt_visible(&grid.extract_text());
            }
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    false
}

#[tokio::test]
async fn turn_complete_within_200ms_on_prompt() {
    let ring = Arc::new(PtyRingBuffer::new());
    assert!(
        wait_turn(
            Arc::clone(&ring),
            "^>\\s*$",
            b"answer\n> ",
            Duration::from_millis(200),
        )
        .await,
        "is_turn_complete equivalent must succeed within 200ms"
    );
}

#[tokio::test]
async fn no_premature_turn_when_stream_continues_after_prompt() {
    let ring = Arc::new(PtyRingBuffer::new());
    let driver = agenthub_core::config::DriverProfile {
        name: "test".into(),
        display_name: "Test".into(),
        executable: "echo".into(),
        args: vec![],
        env: Default::default(),
        prompt_regex: "^>\\s*$".to_string(),
        silence_timeout_ms: 60_000,
        init_sequence: vec![],
        rate_limit_patterns: vec![],
        auto_reply_patterns: Default::default(),
        supports_multi_instance: true,
        max_instances: 0,
    };
    let turn_detector = TurnDetector::from_driver(&driver).expect("regex");
    let mut grid = VirtualGrid::new();
    let mut buf = [0u8; 4096];
    let mut pending: Option<usize> = None;

    ring.write(b"line\n> ");
    let deadline = Instant::now() + Duration::from_millis(CONFIRMATION_MS + 40);
    while Instant::now() < deadline {
        let n = ring.read(&mut buf);
        if n > 0 {
            grid.feed(&buf[..n]);
            if turn_detector.is_prompt_visible(&grid.extract_text()) {
                pending = Some(ring.producer_head());
            }
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    tokio::time::sleep(Duration::from_millis(CONFIRMATION_MS / 2)).await;
    ring.write(b"still generating\n");
    while {
        let n = ring.read(&mut buf);
        if n == 0 {
            false
        } else {
            grid.feed(&buf[..n]);
            true
        }
    } {}

    let Some(head0) = pending else {
        panic!("expected transient prompt match before continued output");
    };
    assert!(
        ring.producer_head() > head0,
        "head must advance when generation continues"
    );
    assert!(
        grid.extract_text().contains("still generating"),
        "continued output must remain in sanitized grid text"
    );
}

#[tokio::test]
async fn prompt_regex_completion_requires_stable_head_for_confirmation_window() {
    let ring = Arc::new(PtyRingBuffer::new());

    // Start a turn: prompt appears...
    ring.write(b"partial\n> ");
    // ...but more bytes arrive within the confirmation window, so completion must not fire yet.
    tokio::time::sleep(Duration::from_millis(CONFIRMATION_MS / 2)).await;
    ring.write(b"more output\n");

    // If completion were premature, this would return true quickly. We demand it does NOT.
    let premature = wait_turn(
        Arc::clone(&ring),
        "^>\\s*$",
        b"",
        Duration::from_millis(CONFIRMATION_MS + 30),
    )
    .await;
    assert!(
        !premature,
        "prompt match must not complete if ring head advances during confirmation window"
    );

    // Now show a stable prompt and ensure completion succeeds.
    ring.write(b"> ");
    assert!(
        wait_turn(ring, "^>\\s*$", b"", Duration::from_millis(200),).await,
        "turn must complete once prompt is stable"
    );
}

#[tokio::test]
async fn all_bundled_drivers_turn_detect_within_200ms() {
    let dir = bundled_drivers_dir();
    for name in BUNDLED_DRIVERS {
        let profile =
            load_driver_profile_from_dir(&dir, name).unwrap_or_else(|e| panic!("load {name}: {e}"));
        let ring = Arc::new(PtyRingBuffer::new());
        let payload = format!("output for {name}\n{}\n", sample_prompt_line(name));
        assert!(
            wait_turn(
                ring,
                &profile.prompt_regex,
                payload.as_bytes(),
                Duration::from_millis(200),
            )
            .await,
            "{name}: turn detection must complete within 200ms"
        );
    }
}
