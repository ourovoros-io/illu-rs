#![expect(clippy::unwrap_used, reason = "integration tests")]

use illu_rs::agents::{SetupFlags, configure_global};
use std::fs;
use std::sync::Mutex;
use tempfile::tempdir;

// `std::env::set_var` is unsafe in Rust 2024 AND mutates process-global state.
// cargo test runs tests within a single binary in parallel by default, so two
// tests racing to set `HOME` could see each other's value. Serialize the env
// mutation + orchestrator call + assertions with a module-level mutex.
static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn install_codex_cli_writes_toml_under_home() {
    let _guard = ENV_LOCK.lock().unwrap();
    let dir = tempdir().unwrap();
    // Use the tempdir as HOME to avoid touching the real filesystem.
    // configure_global currently reads HOME via RealContext; override here.
    unsafe {
        std::env::set_var("HOME", dir.path());
    }
    let flags = SetupFlags {
        explicit_agents: vec!["codex-cli".into()],
        ..SetupFlags::default()
    };
    configure_global(dir.path(), &flags).unwrap();

    let toml_path = dir.path().join(".codex/config.toml");
    assert!(toml_path.exists(), "codex config was not written");
    let content = fs::read_to_string(&toml_path).unwrap();
    assert!(content.contains("[mcp_servers.illu]"));
    assert!(content.contains("command = \"illu-rs\""));
}

#[test]
fn install_rejects_agent_with_no_global_config() {
    let _guard = ENV_LOCK.lock().unwrap();
    let dir = tempdir().unwrap();
    unsafe {
        std::env::set_var("HOME", dir.path());
    }
    let flags = SetupFlags {
        explicit_agents: vec!["vscode-copilot".into()],
        ..SetupFlags::default()
    };
    let err = configure_global(dir.path(), &flags).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("vscode-copilot"),
        "error should mention vscode-copilot, got: {msg}"
    );
}

#[test]
fn install_claude_code_writes_global_settings() {
    let _guard = ENV_LOCK.lock().unwrap();
    let dir = tempdir().unwrap();
    unsafe {
        std::env::set_var("HOME", dir.path());
    }
    let flags = SetupFlags {
        explicit_agents: vec!["claude-code".into()],
        ..SetupFlags::default()
    };
    configure_global(dir.path(), &flags).unwrap();

    assert!(dir.path().join(".claude/settings.json").exists());
    assert!(dir.path().join(".claude/CLAUDE.md").exists());
    assert!(dir.path().join(".claude/agents").is_dir());

    let claude_md = fs::read_to_string(dir.path().join(".claude/CLAUDE.md")).unwrap();
    assert!(claude_md.contains("Plan before code"));
    assert!(claude_md.contains("Read docs before use"));
    assert!(claude_md.contains("planning data structures documentation comments idiomatic rust"));
}
