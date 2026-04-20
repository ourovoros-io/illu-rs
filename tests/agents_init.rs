#![expect(clippy::unwrap_used, reason = "integration tests")]

use illu_rs::agents::{AGENTS, SetupFlags, configure_repo, known_agent_ids};
use std::fs;
use tempfile::tempdir;

fn fake_cargo_repo(dir: &std::path::Path) {
    fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"x\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/lib.rs"), "").unwrap();
}

#[test]
fn init_with_explicit_claude_code_writes_expected_files() {
    let dir = tempdir().unwrap();
    fake_cargo_repo(dir.path());

    let flags = SetupFlags {
        explicit_agents: vec!["claude-code".into()],
        ..SetupFlags::default()
    };
    let reports = configure_repo(dir.path(), &flags).unwrap();
    assert_eq!(reports.len(), 1);
    assert!(!reports[0].skipped);

    assert!(dir.path().join(".mcp.json").exists());
    assert!(dir.path().join("CLAUDE.md").exists());
    assert!(dir.path().join(".claude/agents").is_dir());
    assert!(dir.path().join(".claude/settings.local.json").exists());

    let mcp: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join(".mcp.json")).unwrap()).unwrap();
    assert_eq!(mcp["mcpServers"]["illu"]["command"], "illu-rs");
}

#[test]
fn init_with_unknown_agent_errors() {
    let dir = tempdir().unwrap();
    fake_cargo_repo(dir.path());
    let flags = SetupFlags {
        explicit_agents: vec!["not-an-agent".into()],
        ..SetupFlags::default()
    };
    let err = configure_repo(dir.path(), &flags).unwrap_err();
    assert!(format!("{err}").contains("not-an-agent"));
}

#[test]
fn init_is_idempotent() {
    let dir = tempdir().unwrap();
    fake_cargo_repo(dir.path());
    let flags = SetupFlags {
        explicit_agents: vec!["claude-code".into()],
        ..SetupFlags::default()
    };
    configure_repo(dir.path(), &flags).unwrap();
    let first = fs::read_to_string(dir.path().join(".mcp.json")).unwrap();
    configure_repo(dir.path(), &flags).unwrap();
    let second = fs::read_to_string(dir.path().join(".mcp.json")).unwrap();
    assert_eq!(first, second);
}

#[test]
fn init_preserves_unrelated_mcp_servers() {
    let dir = tempdir().unwrap();
    fake_cargo_repo(dir.path());
    fs::write(
        dir.path().join(".mcp.json"),
        r#"{"mcpServers":{"other":{"command":"x","args":[]}}}"#,
    )
    .unwrap();

    let flags = SetupFlags {
        explicit_agents: vec!["claude-code".into()],
        ..SetupFlags::default()
    };
    configure_repo(dir.path(), &flags).unwrap();

    let v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join(".mcp.json")).unwrap()).unwrap();
    assert_eq!(v["mcpServers"]["other"]["command"], "x");
    assert_eq!(v["mcpServers"]["illu"]["command"], "illu-rs");
}

#[test]
fn init_with_two_agents_writes_both() {
    let dir = tempdir().unwrap();
    fake_cargo_repo(dir.path());
    let flags = SetupFlags {
        explicit_agents: vec!["claude-code".into(), "cursor".into()],
        ..SetupFlags::default()
    };
    configure_repo(dir.path(), &flags).unwrap();

    let claude: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join(".mcp.json")).unwrap()).unwrap();
    assert_eq!(claude["mcpServers"]["illu"]["command"], "illu-rs");
    // Claude's .mcp.json uses mcpServers, not servers.
    assert!(
        claude.get("servers").is_none(),
        "unexpected 'servers' key in Claude Code config"
    );

    let cursor: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join(".cursor/mcp.json")).unwrap())
            .unwrap();
    assert_eq!(cursor["mcpServers"]["illu"]["command"], "illu-rs");

    // Neither file should have cross-contaminated shape from the other format.
    assert!(claude["mcpServers"].get("illu").is_some());
    assert!(cursor["mcpServers"].get("illu").is_some());
}

#[test]
fn init_rejects_agent_with_no_repo_config() {
    let dir = tempdir().unwrap();
    fake_cargo_repo(dir.path());
    let flags = SetupFlags {
        explicit_agents: vec!["codex-cli".into()],
        ..SetupFlags::default()
    };
    let err = configure_repo(dir.path(), &flags).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("codex-cli"),
        "error should mention codex-cli, got: {msg}"
    );
}

#[test]
fn registry_has_expected_agents() {
    let ids: Vec<&str> = AGENTS.iter().map(|a| a.id).collect();
    for expected in [
        "claude-code",
        "gemini-cli",
        "codex-cli",
        "codex-desktop",
        "claude-desktop",
        "cursor",
        "vscode-copilot",
        "antigravity",
    ] {
        assert!(ids.contains(&expected), "missing agent: {expected}");
    }
    // If `AGENTS` grows or shrinks without an update to the expected list
    // above, this assertion catches the drift.
    let known = known_agent_ids();
    assert_eq!(
        known.len(),
        8,
        "known_agent_ids() count mismatch: {known:?}"
    );
}
