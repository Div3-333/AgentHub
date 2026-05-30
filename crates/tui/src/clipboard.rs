//! Copy plain text to the OS clipboard (for sharing TUI output).

use std::io::Write;
use std::process::{Command, Stdio};

/// Write `text` to the system clipboard. Best-effort per platform.
pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        return copy_via_stdin("clip", text);
    }
    #[cfg(target_os = "macos")]
    {
        return copy_via_stdin("pbcopy", text);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if copy_via_stdin("xclip", text).is_ok() {
            return Ok(());
        }
        if copy_via_stdin("xsel", text).is_ok() {
            return Ok(());
        }
        return copy_via_stdin("wl-copy", text);
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = text;
        Err("clipboard not supported on this platform".into())
    }
}

fn copy_via_stdin(program: &str, text: &str) -> Result<(), String> {
    let mut child = Command::new(program)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to run {program}: {e}"))?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| format!("{program} stdin unavailable"))?;
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| format!("clipboard write failed: {e}"))?;
    }
    let status = child
        .wait()
        .map_err(|e| format!("{program} wait failed: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} exited with {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_empty_string_is_ok_or_missing_tool() {
        let result = copy_to_clipboard("");
        if let Err(e) = result {
            assert!(
                e.contains("failed to run") || e.contains("not supported"),
                "unexpected error: {e}"
            );
        }
    }
}
