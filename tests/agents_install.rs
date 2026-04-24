#![expect(clippy::unwrap_used, clippy::expect_used, reason = "integration tests")]

use illu_rs::api::agents::{SetupFlags, configure_global, self_heal_on_serve};
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
    // Global `serve_resolved()` emits bare `["serve"]` — no `--repo` prefix,
    // because global configs have no repo context at write time. Assert the
    // token is present without pinning the exact `args = [...]` spelling, so
    // a future global-scope flag insertion (e.g. `--log-level`) doesn't fail
    // this assertion spuriously. The repo-scope shape is pinned exactly by
    // `self_heal_emits_repo_arg_for_repo_and_resolved_command_for_global`.
    assert!(content.contains("\"serve\""));
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

    // CLAUDE.md must carry the rust-design-discipline contract that the
    // discipline commit (a49534b) embedded into the rendered instruction
    // file.
    let claude_md = fs::read_to_string(dir.path().join(".claude/CLAUDE.md")).unwrap();
    assert!(claude_md.contains("rust_preflight"));
    assert!(claude_md.contains("std_docs"));
    assert!(claude_md.contains("quality_gate"));
    assert!(claude_md.contains("Plan before code"));
    assert!(claude_md.contains("Read docs before use"));
    assert!(claude_md.contains("planning data structures documentation comments idiomatic rust"));
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

#[cfg(target_os = "macos")]
#[test]
fn install_claude_desktop_writes_absolute_command_under_app_support() {
    let _guard = ENV_LOCK.lock().unwrap();
    let dir = tempdir().unwrap();
    unsafe {
        std::env::set_var("HOME", dir.path());
    }
    let flags = SetupFlags {
        explicit_agents: vec!["claude-desktop".into()],
        ..SetupFlags::default()
    };
    configure_global(dir.path(), &flags).unwrap();

    let cfg = dir
        .path()
        .join("Library/Application Support/Claude/claude_desktop_config.json");
    assert!(
        cfg.exists(),
        "claude_desktop_config.json not written: {cfg:?}"
    );
    let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&cfg).unwrap()).unwrap();
    let cmd = v["mcpServers"]["illu"]["command"]
        .as_str()
        .expect("command field must be a string");
    assert!(
        std::path::Path::new(cmd).is_absolute(),
        "Claude Desktop command must be absolute (GUI apps lack shell PATH), got: {cmd}",
    );
    assert_eq!(v["mcpServers"]["illu"]["args"][0], "serve");
}

#[test]
fn install_strips_legacy_mcp_entry_from_settings_json() {
    let _guard = ENV_LOCK.lock().unwrap();
    let dir = tempdir().unwrap();
    unsafe {
        std::env::set_var("HOME", dir.path());
    }
    // Seed a pre-patch settings.json with the now-ineffective `mcpServers.illu`
    // plus another server a third-party tool might have added plus an unrelated
    // permissions block. Only `mcpServers.illu` should disappear after install.
    let settings_path = dir.path().join(".claude/settings.json");
    fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
    let seeded = serde_json::json!({
        "permissions": { "deny": ["Bash(rm -rf *)"] },
        "mcpServers": {
            "illu": { "command": "illu-rs", "args": ["serve"] },
            "other": { "command": "keep-me", "args": [] },
        },
    });
    fs::write(
        &settings_path,
        serde_json::to_string_pretty(&seeded).unwrap(),
    )
    .unwrap();

    let flags = SetupFlags {
        explicit_agents: vec!["claude-code".into()],
        ..SetupFlags::default()
    };
    configure_global(dir.path(), &flags).unwrap();

    let after: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
    assert!(
        after["mcpServers"].get("illu").is_none(),
        "legacy mcpServers.illu should be migrated out of settings.json: {after}",
    );
    assert_eq!(
        after["mcpServers"]["other"]["command"], "keep-me",
        "sibling server entry must be preserved",
    );
    assert_eq!(
        after["permissions"]["deny"][0], "Bash(rm -rf *)",
        "unrelated permissions must be preserved",
    );
    // And the canonical user-scope MCP target should now hold illu.
    let claude_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join(".claude.json")).unwrap())
            .unwrap();
    assert!(claude_json["mcpServers"]["illu"].is_object());
}

/// Unset a process env var on drop so test panics can't leak the var into
/// sibling tests serialized by the same lock.
struct EnvVarGuard(&'static str);
impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            std::env::remove_var(self.0);
        }
    }
}

#[test]
fn self_heal_emits_repo_arg_for_repo_and_resolved_command_for_global() {
    let _guard = ENV_LOCK.lock().unwrap();
    let home = tempdir().unwrap();
    let repo = tempdir().unwrap();
    unsafe {
        std::env::set_var("HOME", home.path());
        // Claude Code is detected as `Active` via its env var, matching how
        // `illu-rs serve` is actually launched by the CLI in practice.
        std::env::set_var("CLAUDECODE", "1");
    }
    let _claudecode = EnvVarGuard("CLAUDECODE");

    self_heal_on_serve(Some(repo.path()), home.path()).unwrap();

    // Per-repo write: `command` stays bare (consumed from PATH), but `args`
    // pin the canonical repo path so the server does not depend on the MCP
    // client's spawn CWD. Without this, launching Claude Code from outside
    // the repo lands in empty-index / cross-repo-only mode.
    let repo_cfg: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(repo.path().join(".mcp.json")).unwrap()).unwrap();
    assert_eq!(
        repo_cfg["mcpServers"]["illu"]["command"], "illu-rs",
        "per-repo command stays bare; the repo path lives in args instead",
    );
    let repo_args = repo_cfg["mcpServers"]["illu"]["args"]
        .as_array()
        .expect("args array");
    assert_eq!(
        repo_args[0], "--repo",
        "per-repo args must start with --repo"
    );
    let embedded = repo_args[1]
        .as_str()
        .expect("repo path arg must be a string");
    assert!(
        std::path::Path::new(embedded).is_absolute(),
        "embedded repo path must be absolute, got: {embedded}",
    );
    assert_eq!(
        repo_args.last().and_then(|v| v.as_str()),
        Some("serve"),
        "per-repo args must terminate with `serve`",
    );

    // Global write: resolved absolute path, survives GUI launch without PATH.
    let global_cfg: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(home.path().join(".claude.json")).unwrap())
            .unwrap();
    let global_cmd = global_cfg["mcpServers"]["illu"]["command"]
        .as_str()
        .expect("global command must be a string");
    assert_ne!(
        global_cmd, "illu-rs",
        "global command must be resolved, not bare",
    );
    assert!(
        std::path::Path::new(global_cmd).is_absolute(),
        "global command must be an absolute path, got: {global_cmd}",
    );
}
