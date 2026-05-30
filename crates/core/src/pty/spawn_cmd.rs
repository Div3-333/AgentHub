//! Resolve driver executables for PTY spawn (Windows npm/cmd shims).

#[cfg(windows)]
use std::path::{Path, PathBuf};

use crate::error::{AgentHubError, Result};

/// Resolved program + args for [`portable_pty::CommandBuilder`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCommand {
    pub program: String,
    pub args: Vec<String>,
}

/// Resolve `executable` (+ driver args) into a process image CreateProcess can launch.
pub fn resolve_spawn_command(executable: &str, driver_args: &[String]) -> Result<ResolvedCommand> {
    if executable.trim().is_empty() {
        return Err(AgentHubError::Pty(
            "driver executable must not be empty".into(),
        ));
    }

    #[cfg(windows)]
    {
        resolve_windows(executable, driver_args)
    }

    #[cfg(not(windows))]
    {
        Ok(ResolvedCommand {
            program: executable.to_string(),
            args: driver_args.to_vec(),
        })
    }
}

#[cfg(windows)]
fn resolve_windows(executable: &str, driver_args: &[String]) -> Result<ResolvedCommand> {
    use std::process::Command;

    let path = Path::new(executable);
    if path.is_file() {
        return wrap_windows_path(path, driver_args);
    }

    let output = Command::new("where.exe")
        .arg(executable)
        .output()
        .map_err(|e| AgentHubError::Pty(format!("where.exe failed for `{executable}`: {e}")))?;

    if !output.status.success() {
        if let Some(fallback) = windows_executable_fallback(executable) {
            return wrap_windows_path(&fallback, driver_args);
        }
        return Err(AgentHubError::Pty(format!(
            "executable `{executable}` not found on PATH (install the CLI or set an absolute path in the driver JSON)"
        )));
    }

    let mut candidates: Vec<PathBuf> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect();

    if candidates.is_empty() {
        return Err(AgentHubError::Pty(format!(
            "executable `{executable}` not found on PATH"
        )));
    }

    candidates.sort_by_key(|candidate| extension_priority(candidate));
    wrap_windows_path(&candidates[0], driver_args)
}

/// Alternate names / install locations when `where.exe` misses the driver shim.
#[cfg(windows)]
fn windows_executable_fallback(executable: &str) -> Option<PathBuf> {
    let alts: &[&str] = match executable.to_ascii_lowercase().as_str() {
        "cursor" | "cursor-agent" => &["cursor-agent", "cursor"][..],
        _ => return None,
    };

    for alt in alts {
        if alt.eq_ignore_ascii_case(executable) {
            continue;
        }
        if let Ok(output) = std::process::Command::new("where.exe").arg(alt).output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(PathBuf::from)?;
                if path.is_file() {
                    return Some(path);
                }
            }
        }
        if let Some(path) = windows_known_install_path(alt) {
            return Some(path);
        }
    }
    None
}

#[cfg(windows)]
fn windows_known_install_path(executable: &str) -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    let local = PathBuf::from(local);
    let candidates: Vec<PathBuf> = match executable {
        "cursor-agent" => vec![
            local.join("cursor-agent").join("cursor-agent.cmd"),
            local.join("cursor-agent").join("cursor-agent.exe"),
            local
                .join("Programs")
                .join("cursor")
                .join("resources")
                .join("app")
                .join("bin")
                .join("cursor.cmd"),
        ],
        "cursor" => vec![local
            .join("Programs")
            .join("cursor")
            .join("resources")
            .join("app")
            .join("bin")
            .join("cursor.cmd")],
        _ => return None,
    };
    candidates.into_iter().find(|path| path.is_file())
}

#[cfg(windows)]
fn extension_priority(path: &Path) -> u8 {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "exe" => 0,
        "cmd" => 1,
        "bat" => 2,
        "com" => 3,
        "ps1" => 4,
        _ => 5,
    }
}

#[cfg(windows)]
fn wrap_windows_path(path: &Path, driver_args: &[String]) -> Result<ResolvedCommand> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let path_str = path.to_string_lossy().into_owned();

    match ext.as_str() {
        "ps1" => {
            let mut args = vec![
                "-NoProfile".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-File".into(),
                path_str,
            ];
            args.extend(driver_args.iter().cloned());
            Ok(ResolvedCommand {
                program: "powershell.exe".into(),
                args,
            })
        }
        "cmd" | "bat" => {
            let mut args = vec!["/C".into(), path_str];
            args.extend(driver_args.iter().cloned());
            Ok(ResolvedCommand {
                program: std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into()),
                args,
            })
        }
        _ => Ok(ResolvedCommand {
            program: path_str,
            args: driver_args.to_vec(),
        }),
    }
}

/// Human-readable preview for spawn / debug traces.
#[must_use]
pub fn format_resolved_command(resolved: &ResolvedCommand) -> String {
    if resolved.args.is_empty() {
        return resolved.program.clone();
    }
    format!("{} {}", resolved.program, resolved.args.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_resolved_command_joins_args() {
        let cmd = ResolvedCommand {
            program: "cmd.exe".into(),
            args: vec!["/C".into(), "gemini.cmd".into()],
        };
        assert_eq!(format_resolved_command(&cmd), "cmd.exe /C gemini.cmd");
    }

    #[cfg(windows)]
    #[test]
    fn wrap_cmd_uses_comspec() {
        let resolved = wrap_windows_path(Path::new(r"C:\npm\gemini.cmd"), &[]).expect("wrap");
        assert!(resolved.program.ends_with("cmd.exe") || resolved.program.contains("CMD"));
        assert_eq!(resolved.args[0], "/C");
        assert!(resolved.args[1].ends_with("gemini.cmd"));
    }

    #[cfg(windows)]
    #[test]
    fn wrap_ps1_uses_powershell() {
        let resolved =
            wrap_windows_path(Path::new(r"C:\npm\gemini.ps1"), &["--help".into()]).expect("wrap");
        assert_eq!(resolved.program, "powershell.exe");
        assert_eq!(resolved.args[0], "-NoProfile");
        assert!(resolved.args.contains(&"-File".to_string()));
        assert!(resolved.args.iter().any(|a| a.contains("gemini.ps1")));
    }
}
