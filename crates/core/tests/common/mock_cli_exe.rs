// Shared mock_cli path resolution for integration tests (core + workspace `include!`).

fn mock_cli_debug_dir() -> std::path::PathBuf {
    let candidates = [
        std::env::var("CARGO_TARGET_DIR")
            .ok()
            .map(|dir| std::path::PathBuf::from(dir).join("debug")),
        Some(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug"),
        ),
    ];
    for dir in candidates.into_iter().flatten() {
        for name in ["mock_cli.exe", "mock_cli"] {
            if dir.join(name).is_file() {
                return dir;
            }
        }
    }
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug")
}

fn mock_cli_executable() -> std::path::PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_mock_cli") {
        return std::path::PathBuf::from(path);
    }
    let debug_dir = mock_cli_debug_dir();
    for name in ["mock_cli.exe", "mock_cli"] {
        let candidate = debug_dir.join(name);
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!(
        "mock_cli binary not found under {}; run `cargo build -p mock-cli` first",
        debug_dir.display()
    );
}
