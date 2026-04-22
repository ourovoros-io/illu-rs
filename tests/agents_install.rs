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
    // Global configs use an absolute path (std::env::current_exe) so GUI
    // agents without shell PATH can still spawn the server. Under `cargo test`
    // the resolved path is the test binary, so only assert a command field
    // was written — not the literal "illu-rs" string.
    assert!(content.contains("command = \""));
    assert!(content.contains("args = [\"serve\"]"));
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

    // `.claude/settings.json` is written by the allow-list stage (permissions).
    assert!(dir.path().join(".claude/settings.json").exists());
    assert!(dir.path().join(".claude/CLAUDE.md").exists());
    assert!(dir.path().join(".claude/agents").is_dir());

    // User-scope MCP servers must land in `~/.claude.json` — Claude Code does
    // not read `mcpServers` from `~/.claude/settings.json`.
    let claude_json_path = dir.path().join(".claude.json");
    assert!(
        claude_json_path.exists(),
        "~/.claude.json was not written (user-scope MCP target)",
    );
    let claude_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&claude_json_path).unwrap()).unwrap();
    assert!(
        claude_json["mcpServers"]["illu"].is_object(),
        "mcpServers.illu missing from ~/.claude.json: {claude_json}",
    );
    assert_eq!(claude_json["mcpServers"]["illu"]["args"][0], "serve");

    // And `.claude/settings.json` must NOT carry an `mcpServers` key.
    let settings: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(dir.path().join(".claude/settings.json")).unwrap(),
    )
    .unwrap();
    assert!(
        settings.get("mcpServers").is_none(),
        "mcpServers leaked into .claude/settings.json: {settings}",
    );
}

#[test]
fn install_antigravity_writes_under_gemini_subdir() {
    let _guard = ENV_LOCK.lock().unwrap();
    let dir = tempdir().unwrap();
    unsafe {
        std::env::set_var("HOME", dir.path());
    }
    let flags = SetupFlags {
        explicit_agents: vec!["antigravity".into()],
        ..SetupFlags::default()
    };
    configure_global(dir.path(), &flags).unwrap();

    let path = dir.path().join(".gemini/antigravity/mcp_config.json");
    assert!(path.exists(), "antigravity mcp_config.json was not written");
    let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert!(v["mcpServers"]["illu"].is_object());

    // Legacy path must not be written.
    assert!(
        !dir.path().join(".antigravity/mcp.json").exists(),
        "legacy ~/.antigravity/mcp.json should no longer be written",
    );
}
