//! Global configuration (`~/.agenthub/config.json`) and driver profiles.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AgentHubError, Result};

const LOG_LEVELS: &[&str] = &["trace", "debug", "info", "warn", "error"];

/// Root configuration file: `~/.agenthub/config.json`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentHubConfig {
    /// Default workspace mode on launch.
    pub default_mode: WorkspaceMode,
    /// Path to the drivers directory. Defaults to `~/.agenthub/drivers` (missing profiles fall back to bundled `drivers/`).
    pub drivers_dir: PathBuf,
    /// Path to the SQLite database. Defaults to `~/.agenthub/agenthub.db`.
    pub db_path: PathBuf,
    /// Path where VFS snapshots are stored. Defaults to `.agenthub_shadow/` in the CWD.
    pub shadow_dir: PathBuf,
    /// Maximum number of concurrent agent PTYs. Default: 16.
    pub max_agents: u8,
    /// Global log level. One of: "trace", "debug", "info", "warn", "error". Default: "info".
    pub log_level: String,
    /// Theme selection for the TUI. Default: "dark".
    pub theme: String,
    /// Custom key bindings. Keys are action names, values are key strings.
    pub keybindings: HashMap<String, String>,
    /// When true, raw PTY output is captured (zstd-compressed) for driver debugging.
    /// Default: false — no PTY bytes are written to disk unless explicitly enabled.
    #[serde(default)]
    pub pty_debug_log: bool,
    /// When true, emit live spawn steps and PTY I/O previews into the TUI chat.
    #[serde(default)]
    pub spawn_debug: bool,
}

impl Default for AgentHubConfig {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self {
            default_mode: WorkspaceMode::GroupChat,
            drivers_dir: home.join(".agenthub").join("drivers"),
            db_path: home.join(".agenthub").join("agenthub.db"),
            shadow_dir: PathBuf::from(".agenthub_shadow"),
            max_agents: 16,
            log_level: "info".to_string(),
            theme: "dark".to_string(),
            keybindings: HashMap::new(),
            pty_debug_log: false,
            spawn_debug: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceMode {
    /// 1-on-1 session. Minimal UI. No RBAC. No broadcast.
    DirectMessage,
    /// Small group, unconstrained. Broadcast enabled. Minimal governance.
    GroupChat,
    /// Full hierarchy: channels, roles, admin controls, structured moderation.
    Server,
}

/// Describes how to spawn and interact with a single CLI driver.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriverProfile {
    /// Unique machine-readable name. E.g., "gemini", "claude".
    pub name: String,
    /// Human-readable display name. E.g., "Gemini CLI".
    pub display_name: String,
    /// The executable to spawn. Must be on PATH or an absolute path.
    pub executable: String,
    /// Arguments passed to the executable on launch.
    pub args: Vec<String>,
    /// Environment variables to set for this process.
    pub env: HashMap<String, String>,
    /// Regex pattern that matches the CLI's input prompt.
    pub prompt_regex: String,
    /// Maximum milliseconds of output silence before the turn is complete.
    pub silence_timeout_ms: u64,
    /// Text injected after launch to suppress welcome screens and wizards.
    pub init_sequence: Vec<String>,
    /// Known rate-limit error substrings.
    pub rate_limit_patterns: Vec<String>,
    /// Key: regex to detect the prompt. Value: the string to inject.
    pub auto_reply_patterns: HashMap<String, String>,
    /// Whether this CLI supports multiple simultaneous instances.
    pub supports_multi_instance: bool,
    /// Maximum number of instances allowed. 0 = unlimited.
    pub max_instances: u8,
}

impl AgentHubConfig {
    /// Validates field values after deserialization.
    pub fn validate(&self) -> Result<()> {
        if !LOG_LEVELS.contains(&self.log_level.as_str()) {
            return Err(AgentHubError::Config(format!(
                "invalid log_level '{}'; expected one of: {}",
                self.log_level,
                LOG_LEVELS.join(", ")
            )));
        }
        if self.max_agents == 0 {
            return Err(AgentHubError::Config(
                "max_agents must be at least 1".to_string(),
            ));
        }
        if self.theme.is_empty() {
            return Err(AgentHubError::Config("theme must not be empty".to_string()));
        }
        if self.drivers_dir.as_os_str().is_empty() {
            return Err(AgentHubError::Config(
                "drivers_dir must not be empty".to_string(),
            ));
        }
        if self.db_path.as_os_str().is_empty() {
            return Err(AgentHubError::Config(
                "db_path must not be empty".to_string(),
            ));
        }
        Ok(())
    }

    /// Loads configuration from [`config_path`], writing defaults on first run.
    pub fn load() -> Result<Self> {
        Self::load_from(&config_path())
    }

    /// Loads configuration from `path`, creating the file with defaults if missing.
    pub fn load_from(path: &Path) -> Result<Self> {
        let config = if path.exists() {
            let contents = std::fs::read_to_string(path)?;
            parse_config_json(&contents)?
        } else {
            let config = Self::default();
            config.save_to(path)?;
            config
        };
        ensure_user_drivers(&config.drivers_dir)?;
        Ok(config)
    }

    /// Persists configuration to [`config_path`].
    pub fn save(&self) -> Result<()> {
        self.save_to(&config_path())
    }

    /// Persists configuration to `path`, creating parent directories as needed.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        self.validate()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(AgentHubError::from)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Loads a driver profile JSON file from [`Self::drivers_dir`] as `{name}.json`.
    pub fn load_driver_profile(&self, name: &str) -> Result<DriverProfile> {
        load_driver_profile_from_dir(&self.drivers_dir, name)
    }
}

impl DriverProfile {
    /// Loads `{name}.json` from `drivers_dir` (alias for [`load_driver_profile_from_dir`]).
    pub fn load_from_dir(drivers_dir: &Path, name: &str) -> Result<Self> {
        load_driver_profile_from_dir(drivers_dir, name)
    }

    /// Validates regex fields, required env, and profile invariants.
    pub fn validate(&self) -> Result<()> {
        if self.name.is_empty() {
            return Err(AgentHubError::DriverProfile {
                driver: String::new(),
                msg: "name must not be empty".to_string(),
            });
        }
        if self.display_name.is_empty() {
            return Err(AgentHubError::DriverProfile {
                driver: self.name.clone(),
                msg: "display_name must not be empty".to_string(),
            });
        }
        if self.executable.is_empty() {
            return Err(AgentHubError::DriverProfile {
                driver: self.name.clone(),
                msg: "executable must not be empty".to_string(),
            });
        }
        if self.silence_timeout_ms == 0 {
            return Err(AgentHubError::DriverProfile {
                driver: self.name.clone(),
                msg: "silence_timeout_ms must be greater than 0".to_string(),
            });
        }
        if !self.supports_multi_instance && self.max_instances == 0 {
            return Err(AgentHubError::DriverProfile {
                driver: self.name.clone(),
                msg: "max_instances must be 1 when supports_multi_instance is false (0 means unlimited)"
                    .to_string(),
            });
        }
        regex::Regex::new(&self.prompt_regex).map_err(|e| AgentHubError::DriverProfile {
            driver: self.name.clone(),
            msg: format!("invalid prompt_regex: {e}"),
        })?;
        for pattern in self.auto_reply_patterns.keys() {
            regex::Regex::new(pattern).map_err(|e| AgentHubError::DriverProfile {
                driver: self.name.clone(),
                msg: format!("invalid auto_reply pattern `{pattern}`: {e}"),
            })?;
        }
        for pattern in &self.rate_limit_patterns {
            if pattern.is_empty() {
                return Err(AgentHubError::DriverProfile {
                    driver: self.name.clone(),
                    msg: "rate_limit_patterns must not contain empty strings".to_string(),
                });
            }
        }
        if !self.env.contains_key("NO_COLOR") {
            return Err(AgentHubError::DriverProfile {
                driver: self.name.clone(),
                msg: "env must include NO_COLOR=1".to_string(),
            });
        }
        if self.env.get("TERM").map(String::as_str) != Some("dumb") {
            return Err(AgentHubError::DriverProfile {
                driver: self.name.clone(),
                msg: "env must include TERM=dumb".to_string(),
            });
        }
        Ok(())
    }
}

/// Bundled driver profiles at the repository root (`drivers/`).
pub fn bundled_drivers_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../drivers")
}

/// Copy bundled `drivers/*.json` into `drivers_dir` when missing (release binaries).
pub fn ensure_user_drivers(drivers_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(drivers_dir)?;
    let bundled = bundled_drivers_dir();
    if !bundled.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(&bundled)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let dest = drivers_dir.join(entry.file_name());
        if !dest.exists() {
            std::fs::copy(&path, &dest)?;
        }
    }
    Ok(())
}

/// Returns `~/.agenthub/config.json` (or the platform equivalent).
pub fn config_path() -> PathBuf {
    agenthub_home().join("config.json")
}

/// Returns the AgentHub data directory (`~/.agenthub`, or `AGENTHUB_CONFIG_DIR` when set).
pub fn agenthub_home() -> PathBuf {
    if let Ok(dir) = std::env::var("AGENTHUB_CONFIG_DIR") {
        let path = PathBuf::from(dir);
        if !path.as_os_str().is_empty() {
            return path;
        }
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".agenthub")
}

/// Directory for opt-in PTY debug captures (`~/.agenthub/debug/`).
pub fn agenthub_debug_dir() -> PathBuf {
    agenthub_home().join("debug")
}

/// Loads `{name}.json` from `drivers_dir`, falling back to [`bundled_drivers_dir`].
pub fn load_driver_profile_from_dir(drivers_dir: &Path, name: &str) -> Result<DriverProfile> {
    if name.is_empty() {
        return Err(AgentHubError::DriverProfile {
            driver: String::new(),
            msg: "driver name must not be empty".to_string(),
        });
    }

    let path = drivers_dir.join(format!("{name}.json"));
    let path = if path.is_file() {
        path
    } else {
        let bundled = bundled_drivers_dir().join(format!("{name}.json"));
        if bundled.is_file() {
            bundled
        } else {
            return Err(AgentHubError::DriverProfile {
                driver: name.to_string(),
                msg: format!(
                    "profile not found in {} or bundled drivers/",
                    drivers_dir.display()
                ),
            });
        }
    };
    let contents = std::fs::read_to_string(&path)?;
    let profile: DriverProfile = serde_json::from_str(&contents).map_err(|e| {
        AgentHubError::Config(format!(
            "failed to parse driver profile {}: {e}",
            path.display()
        ))
    })?;
    if profile.name != name {
        return Err(AgentHubError::DriverProfile {
            driver: name.to_string(),
            msg: format!(
                "profile name `{}` does not match requested `{}`",
                profile.name, name
            ),
        });
    }
    profile.validate()?;
    Ok(profile)
}

fn parse_config_json(contents: &str) -> Result<AgentHubConfig> {
    let config: AgentHubConfig = serde_json::from_str(contents).map_err(AgentHubError::from)?;
    config.validate()?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn config_roundtrip_via_tempfile() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("config.json");

        let mut original = AgentHubConfig::default();
        original.default_mode = WorkspaceMode::Server;
        original.max_agents = 8;
        original.log_level = "debug".to_string();
        original.theme = "light".to_string();
        original
            .keybindings
            .insert("quit".to_string(), "ctrl+q".to_string());

        original.save_to(&path).expect("save");
        let loaded = AgentHubConfig::load_from(&path).expect("load");
        assert_eq!(loaded, original);
    }

    #[test]
    fn default_pty_debug_log_is_false() {
        let config = AgentHubConfig::default();
        assert!(!config.pty_debug_log);
    }

    #[test]
    fn omitted_pty_debug_log_deserializes_false() {
        let json = r#"{
  "default_mode": "group_chat",
  "drivers_dir": "/tmp/drivers",
  "db_path": "/tmp/agenthub.db",
  "shadow_dir": ".agenthub_shadow",
  "max_agents": 16,
  "log_level": "info",
  "theme": "dark",
  "keybindings": {}
}"#;
        let config: AgentHubConfig = serde_json::from_str(json).expect("parse");
        assert!(!config.pty_debug_log);
    }

    #[test]
    fn load_creates_defaults_when_missing() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("nested").join("config.json");

        let loaded = AgentHubConfig::load_from(&path).expect("load");
        assert!(path.is_file());
        assert_eq!(loaded.default_mode, WorkspaceMode::GroupChat);
        assert_eq!(loaded.max_agents, 16);
        loaded.validate().expect("defaults valid");
    }

    #[test]
    fn invalid_log_level_rejected_on_load() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("config.json");
        fs::write(
            &path,
            r#"{
  "default_mode": "group_chat",
  "drivers_dir": "/tmp/drivers",
  "db_path": "/tmp/agenthub.db",
  "shadow_dir": ".agenthub_shadow",
  "max_agents": 16,
  "log_level": "verbose",
  "theme": "dark",
  "keybindings": {}
}"#,
        )
        .expect("write");

        let err = AgentHubConfig::load_from(&path).expect_err("bad log level");
        assert!(matches!(err, AgentHubError::Config(_)));
    }

    #[test]
    fn load_driver_profile_from_temp_dir() {
        let dir = TempDir::new().expect("tempdir");
        let drivers_dir = dir.path().join("drivers");
        fs::create_dir_all(&drivers_dir).expect("mkdir");

        let profile = DriverProfile {
            name: "mock".to_string(),
            display_name: "Mock CLI".to_string(),
            executable: "mock-cli".to_string(),
            args: vec!["--no-color".to_string()],
            env: HashMap::from([
                ("NO_COLOR".to_string(), "1".to_string()),
                ("TERM".to_string(), "dumb".to_string()),
            ]),
            prompt_regex: "^>\\s*$".to_string(),
            silence_timeout_ms: 3000,
            init_sequence: vec![],
            rate_limit_patterns: vec!["429".to_string()],
            auto_reply_patterns: HashMap::new(),
            supports_multi_instance: true,
            max_instances: 0,
        };

        let json = serde_json::to_string_pretty(&profile).expect("serialize");
        fs::write(drivers_dir.join("mock.json"), json).expect("write");

        let config = AgentHubConfig {
            drivers_dir: drivers_dir.clone(),
            ..AgentHubConfig::default()
        };

        let loaded = config.load_driver_profile("mock").expect("load profile");
        assert_eq!(loaded, profile);
    }

    #[test]
    fn load_driver_profile_falls_back_to_bundled() {
        let dir = TempDir::new().expect("tempdir");
        let empty_drivers = dir.path().join("drivers");
        fs::create_dir_all(&empty_drivers).expect("mkdir");

        let config = AgentHubConfig {
            drivers_dir: empty_drivers,
            ..AgentHubConfig::default()
        };

        let profile = config
            .load_driver_profile("gemini")
            .expect("bundled fallback");
        assert_eq!(profile.name, "gemini");
        assert_eq!(profile.display_name, "Gemini CLI");
    }

    #[test]
    fn load_driver_profile_missing_returns_error() {
        let dir = TempDir::new().expect("tempdir");
        let config = AgentHubConfig {
            drivers_dir: dir.path().join("drivers"),
            ..AgentHubConfig::default()
        };

        let err = config
            .load_driver_profile("missing")
            .expect_err("expected error");
        assert!(matches!(err, AgentHubError::DriverProfile { .. }));
    }

    #[test]
    fn driver_profile_name_mismatch_rejected() {
        let dir = TempDir::new().expect("tempdir");
        let drivers_dir = dir.path().join("drivers");
        fs::create_dir_all(&drivers_dir).expect("mkdir");

        let profile = DriverProfile {
            name: "wrong".to_string(),
            display_name: "Mock".to_string(),
            executable: "mock-cli".to_string(),
            args: vec![],
            env: HashMap::from([
                ("NO_COLOR".to_string(), "1".to_string()),
                ("TERM".to_string(), "dumb".to_string()),
            ]),
            prompt_regex: "^>\\s*$".to_string(),
            silence_timeout_ms: 3000,
            init_sequence: vec![],
            rate_limit_patterns: vec!["429".to_string()],
            auto_reply_patterns: HashMap::new(),
            supports_multi_instance: true,
            max_instances: 0,
        };
        let json = serde_json::to_string_pretty(&profile).expect("serialize");
        fs::write(drivers_dir.join("mock.json"), json).expect("write");

        let err = load_driver_profile_from_dir(&drivers_dir, "mock").expect_err("name mismatch");
        assert!(matches!(err, AgentHubError::DriverProfile { .. }));
    }

    #[test]
    fn invalid_prompt_regex_rejected() {
        let dir = TempDir::new().expect("tempdir");
        let drivers_dir = dir.path().join("drivers");
        fs::create_dir_all(&drivers_dir).expect("mkdir");

        let profile = DriverProfile {
            name: "bad".to_string(),
            display_name: "Bad".to_string(),
            executable: "bad-cli".to_string(),
            args: vec![],
            env: HashMap::from([
                ("NO_COLOR".to_string(), "1".to_string()),
                ("TERM".to_string(), "dumb".to_string()),
            ]),
            prompt_regex: "[unclosed".to_string(),
            silence_timeout_ms: 3000,
            init_sequence: vec![],
            rate_limit_patterns: vec![],
            auto_reply_patterns: HashMap::new(),
            supports_multi_instance: true,
            max_instances: 0,
        };
        let json = serde_json::to_string_pretty(&profile).expect("serialize");
        fs::write(drivers_dir.join("bad.json"), json).expect("write");

        let err = load_driver_profile_from_dir(&drivers_dir, "bad").expect_err("bad regex");
        assert!(matches!(err, AgentHubError::DriverProfile { .. }));
    }

    #[test]
    fn workspace_mode_snake_case_serde() {
        let json = r#""direct_message""#;
        let mode: WorkspaceMode = serde_json::from_str(json).expect("parse");
        assert_eq!(mode, WorkspaceMode::DirectMessage);
    }

    const BUNDLED_DRIVERS: &[&str] = &["gemini", "claude", "codex", "aider", "cursor"];

    #[test]
    fn bundled_driver_profiles_parse() {
        let drivers_dir = bundled_drivers_dir();
        assert!(
            drivers_dir.is_dir(),
            "bundled drivers dir missing: {}",
            drivers_dir.display()
        );

        for name in BUNDLED_DRIVERS {
            let profile = load_driver_profile_from_dir(&drivers_dir, name)
                .unwrap_or_else(|e| panic!("failed to load driver {name}: {e}"));
            assert_eq!(profile.name, *name);
            assert!(!profile.display_name.is_empty());
            assert!(!profile.executable.is_empty());
            assert!(profile.silence_timeout_ms > 0);
            assert!(
                profile.env.contains_key("NO_COLOR"),
                "{name}: expected NO_COLOR in env"
            );
            assert_eq!(profile.env.get("TERM").map(String::as_str), Some("dumb"));
        }
    }

    #[test]
    fn bundled_gemini_matches_blueprint_example() {
        let profile =
            load_driver_profile_from_dir(&bundled_drivers_dir(), "gemini").expect("gemini");
        assert_eq!(profile.silence_timeout_ms, 5000);
        assert!(profile
            .auto_reply_patterns
            .contains_key("Do you want to continue\\? \\[Y/n\\]"));
        assert!(profile.rate_limit_patterns.contains(&"429".to_string()));
    }

    #[test]
    fn bundled_claude_matches_blueprint_example() {
        let profile =
            load_driver_profile_from_dir(&bundled_drivers_dir(), "claude").expect("claude");
        assert_eq!(profile.prompt_regex, "^\\?\\s*$");
        assert_eq!(profile.silence_timeout_ms, 8000);
        assert!(profile
            .auto_reply_patterns
            .contains_key("Continue\\? \\(Y/n\\)"));
    }
}
