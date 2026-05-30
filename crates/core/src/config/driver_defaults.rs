//! Shared headless / trust defaults merged into every driver profile at load time.

use std::collections::HashMap;

use super::DriverProfile;

/// Regex → stdin reply for interactive prompts common across coding CLIs.
pub fn common_auto_reply_patterns() -> HashMap<String, String> {
    HashMap::from([
        (
            r"Do you want to continue\? \[Y/n\]".to_string(),
            "Y\n".to_string(),
        ),
        (r"Continue\? \(Y/n\)".to_string(), "Y\n".to_string()),
        (r"Press Enter to continue".to_string(), "\n".to_string()),
        (
            "Do you trust the files in this folder".to_string(),
            "1\n".to_string(),
        ),
        (
            "Do you trust the contents of this directory".to_string(),
            "1\n".to_string(),
        ),
        (r"Trust folder \(".to_string(), "1\n".to_string()),
        (
            r"Trust this (folder|directory)".to_string(),
            "1\n".to_string(),
        ),
        (r"\(Y/n\)".to_string(), "Y\n".to_string()),
        (r"\[Y/n\]".to_string(), "Y\n".to_string()),
        (r"\(y/N\)".to_string(), "y\n".to_string()),
        (r"\[y/N\]".to_string(), "y\n".to_string()),
    ])
}

/// Merge shared patterns and apply per-driver non-interactive flags (trust, approvals).
pub fn apply_driver_orchestration_defaults(profile: &mut DriverProfile) {
    merge_auto_reply_patterns(profile);
    apply_driver_specific_defaults(profile);
}

fn merge_auto_reply_patterns(profile: &mut DriverProfile) {
    for (pattern, reply) in common_auto_reply_patterns() {
        profile.auto_reply_patterns.entry(pattern).or_insert(reply);
    }
}

fn push_arg_if_missing(args: &mut Vec<String>, flag: &str) {
    if args.iter().any(|a| a == flag) {
        return;
    }
    if args.iter().any(|a| a.starts_with(&format!("{flag}="))) {
        return;
    }
    args.push(flag.to_string());
}

fn set_env_if_missing(env: &mut HashMap<String, String>, key: &str, value: &str) {
    env.entry(key.to_string())
        .or_insert_with(|| value.to_string());
}

fn apply_driver_specific_defaults(profile: &mut DriverProfile) {
    match profile.name.as_str() {
        "gemini" => {
            push_arg_if_missing(&mut profile.args, "--skip-trust");
            set_env_if_missing(&mut profile.env, "GEMINI_CLI_TRUST_WORKSPACE", "true");
        }
        "cursor" => {
            push_arg_if_missing(&mut profile.args, "--trust");
            push_arg_if_missing(&mut profile.args, "--approve-mcps");
        }
        "claude" => {
            // Interactive PTY session; rely on common auto_reply for trust/continue prompts.
            set_env_if_missing(
                &mut profile.env,
                "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
                "1",
            );
        }
        "codex" => {
            if !profile
                .args
                .windows(2)
                .any(|w| w[0] == "--ask-for-approval")
            {
                profile.args.push("--ask-for-approval".to_string());
                profile.args.push("never".to_string());
            }
            apply_codex_cwd_trust(profile);
        }
        "aider" => {
            push_arg_if_missing(&mut profile.args, "--yes");
            push_arg_if_missing(&mut profile.args, "--no-git");
        }
        _ => {}
    }
}

fn push_codex_config_arg(profile: &mut DriverProfile, config_fragment: &str) {
    let pair = format!("-c={config_fragment}");
    if profile
        .args
        .iter()
        .any(|a| a == &pair || a.ends_with(config_fragment))
    {
        return;
    }
    profile.args.push("-c".to_string());
    profile.args.push(config_fragment.to_string());
}

fn apply_codex_cwd_trust(profile: &mut DriverProfile) {
    let Ok(cwd) = std::env::current_dir() else {
        return;
    };
    let path = cwd.to_string_lossy();
    let escaped = path.replace('\\', "\\\\");
    push_codex_config_arg(
        profile,
        &format!(r#"projects."{escaped}".trust_level="trusted""#),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn base_profile(name: &str) -> DriverProfile {
        DriverProfile {
            name: name.to_string(),
            display_name: name.to_string(),
            executable: name.to_string(),
            args: vec![],
            env: HashMap::from([
                ("NO_COLOR".to_string(), "1".to_string()),
                ("TERM".to_string(), "dumb".to_string()),
            ]),
            prompt_regex: "^>\\s*$".to_string(),
            silence_timeout_ms: 5000,
            init_sequence: vec![],
            rate_limit_patterns: vec![],
            auto_reply_patterns: HashMap::new(),
            supports_multi_instance: true,
            max_instances: 0,
        }
    }

    #[test]
    fn merges_common_auto_reply_without_overwriting_driver() {
        let mut profile = base_profile("codex");
        profile.auto_reply_patterns.insert(
            "Do you trust the files in this folder".to_string(),
            "9\n".to_string(),
        );
        apply_driver_orchestration_defaults(&mut profile);
        assert_eq!(
            profile
                .auto_reply_patterns
                .get("Do you trust the files in this folder"),
            Some(&"9\n".to_string())
        );
        assert!(profile
            .auto_reply_patterns
            .contains_key("Do you trust the contents of this directory"));
    }

    #[test]
    fn gemini_gets_skip_trust() {
        let mut profile = base_profile("gemini");
        apply_driver_orchestration_defaults(&mut profile);
        assert!(profile.args.iter().any(|a| a == "--skip-trust"));
        assert_eq!(
            profile
                .env
                .get("GEMINI_CLI_TRUST_WORKSPACE")
                .map(String::as_str),
            Some("true")
        );
    }

    #[test]
    fn cursor_gets_trust_flag() {
        let mut profile = base_profile("cursor");
        apply_driver_orchestration_defaults(&mut profile);
        assert!(profile.args.iter().any(|a| a == "--trust"));
    }
}
