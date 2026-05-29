//! Mock CLI: Simulates an interactive AI CLI for integration testing.
//! Reads from stdin, prints a canned response, then prints the prompt symbol.
//!
//! Behaviour flags (via environment variables):
//!   MOCK_CLI_PROMPT         : The prompt string to print. Default: "> "
//!   MOCK_CLI_RESPONSE       : The response to any input. Default: "Mock response."
//!   MOCK_CLI_LATENCY_MS     : Simulated thinking time in ms. Default: 100
//!   MOCK_CLI_RATE_LIMIT_ON  : If "1", print a rate limit error instead of responding.
//!   MOCK_CLI_INDUCTION_ACK  : If "1", respond to induction with "READY".
//!   MOCK_CLI_ECHO_INPUT     : If "1", include a short prefix of the input in the response (spar/pipeline tests).

use std::io::{self, BufRead, Write};
use std::time::Duration;

fn main() {
    let prompt = std::env::var("MOCK_CLI_PROMPT").unwrap_or_else(|_| "> ".to_string());
    let response =
        std::env::var("MOCK_CLI_RESPONSE").unwrap_or_else(|_| "Mock response.".to_string());
    let latency_ms: u64 = std::env::var("MOCK_CLI_LATENCY_MS")
        .unwrap_or_else(|_| "100".to_string())
        .parse()
        .unwrap_or(100);
    let rate_limit = std::env::var("MOCK_CLI_RATE_LIMIT_ON")
        .map(|v| v == "1")
        .unwrap_or(false);
    let ack_induction = std::env::var("MOCK_CLI_INDUCTION_ACK")
        .map(|v| v == "1")
        .unwrap_or(true);
    let echo_input = std::env::var("MOCK_CLI_ECHO_INPUT")
        .map(|v| v == "1")
        .unwrap_or(false);

    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();

    // Line-terminated prompts so piped stdout (AGENTHUB_SKIP_PTY=1) unblocks readers on Windows.
    writeln!(stdout, "{}", prompt.trim_end()).expect("initial prompt");
    stdout.flush().unwrap();

    for line in stdin.lock().lines() {
        let input = line.unwrap_or_default();
        std::thread::sleep(Duration::from_millis(latency_ms));

        if ack_induction && input.contains("AGENTHUB") {
            writeln!(stdout, "READY").expect("ready");
        } else if rate_limit {
            writeln!(
                stdout,
                "Error: rate limit exceeded. Please try again later."
            )
            .expect("rate limit");
        } else if echo_input {
            let snippet: String = input.chars().take(48).collect();
            writeln!(stdout, "{response} [{snippet}]").expect("echo response");
        } else {
            writeln!(stdout, "{response}").expect("response");
        }
        stdout.flush().expect("flush response");

        writeln!(stdout, "{}", prompt.trim_end()).expect("prompt");
        stdout.flush().expect("flush prompt");
    }
}
