# Multi-agent setup implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace illu's current "write every agent's config on every run" flow with a data-driven registry of eight agents (Claude Code, Gemini CLI, Codex CLI, Codex Desktop, Antigravity, Claude Desktop, Cursor, VS Code + Copilot) that is detected or prompted for before any config is written.

**Architecture:** A new `src/agents/` module owns a static `AGENTS: &[Agent]` registry. Three entry points — `configure_repo`, `configure_global`, `self_heal_on_serve` — dispatch on the registry. A `McpFormat` enum handles seven config-file shapes (JSON variants + TOML). Detection runs via env vars, `PATH`, config dirs, and (macOS) app bundles. `dialoguer` provides a multi-select prompt; no-TTY falls back to auto-detect-only.

**Tech Stack:** Rust 2024, rmcp, serde_json, new deps: `dialoguer`, `toml_edit`.

**Spec:** `docs/superpowers/specs/2026-04-20-multi-agent-setup-design.md`

**Binary name note:** The existing code uses `"illu-rs"` as the MCP `command` (not `"illu"` as prose might suggest). All writer tasks below preserve that, plus the existing `env: { "RUST_LOG": "warn" }` block.

---

## Phase 1 — Foundation

### Task 1: Create agents module skeleton with core types

**Files:**
- Create: `src/agents/mod.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Add module declaration to `src/lib.rs`**

Add this line alongside the existing `pub mod` lines (after `pub mod server;`):

```rust
pub mod agents;
```

- [ ] **Step 2: Create `src/agents/mod.rs` with type definitions (compiles, no logic yet)**

```rust
//! Per-agent config registry and orchestration.

use std::path::PathBuf;

/// Static metadata for one supported agent.
pub struct Agent {
    pub id: &'static str,
    pub display_name: &'static str,
    pub detection: Detection,
    pub repo_config: Option<RepoConfig>,
    pub global_config: Option<GlobalConfig>,
    pub tool_prefix: &'static str,
}

/// Heuristics used to detect whether an agent is installed or active.
pub struct Detection {
    pub env_vars: &'static [&'static str],
    pub binaries: &'static [&'static str],
    pub config_dirs: &'static [&'static str],
    pub app_bundles: &'static [&'static str],
}

/// Per-repo config targets.
pub struct RepoConfig {
    pub mcp_config_path: &'static str,
    pub mcp_format: McpFormat,
    pub instruction_file: Option<&'static str>,
    pub agents_dir: Option<&'static str>,
    pub allow_list_path: Option<&'static str>,
}

/// Global config targets.
pub struct GlobalConfig {
    pub mcp_config_path: GlobalPath,
    pub mcp_format: McpFormat,
    pub instruction_file: Option<GlobalPath>,
    pub agents_dir: Option<GlobalPath>,
    pub allow_list_path: Option<GlobalPath>,
}

/// Platform-sensitive path kind used in global configs.
#[derive(Clone, Copy)]
pub enum GlobalPath {
    Home(&'static str),
    AppSupport(&'static str, &'static str),
    AppData(&'static str, &'static str),
    XdgConfig(&'static str),
}

/// One entry per distinct on-disk config file schema.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum McpFormat {
    ClaudeCodeJson,
    GeminiJson,
    ClaudeDesktopJson,
    CursorJson,
    VsCodeJson,
    CodexToml,
    AntigravityJson,
}

/// Detection strength for a single agent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DetectionLevel {
    Active,
    Installed,
    Unknown,
}

/// Command invoked by the MCP client to run illu.
#[derive(Clone, Debug)]
pub struct IlluCommand {
    pub command: String,
    pub args: Vec<String>,
}

impl IlluCommand {
    pub fn serve() -> Self {
        Self {
            command: "illu-rs".to_string(),
            args: vec!["serve".to_string()],
        }
    }
}

/// User-facing flags shared by the `init` and `install` subcommands.
#[derive(Clone, Debug, Default)]
pub struct SetupFlags {
    pub explicit_agents: Vec<String>,
    pub all: bool,
    pub yes: bool,
    pub dry_run: bool,
}

/// One entry per agent we wrote to (or skipped).
#[derive(Debug)]
pub struct AgentWriteReport {
    pub agent_id: &'static str,
    pub written_paths: Vec<PathBuf>,
    pub skipped: bool,
}

/// Master registry. Populated in Phase 5 tasks.
pub static AGENTS: &[Agent] = &[];
```

- [ ] **Step 3: Run `cargo build` and confirm it compiles**

Run: `cargo build`
Expected: compiles cleanly with no warnings about dead code on the new types (they are all `pub`).

- [ ] **Step 4: Commit**

```bash
git add src/lib.rs src/agents/mod.rs
git commit -m "feat(agents): add module skeleton with registry types"
```

---

## Phase 2 — Lift-and-shift existing agent logic (no behavior change)

These tasks move code that already exists, keeping tests green after each. Nothing in `main.rs`'s orchestration changes yet.

### Task 2: Move `ensure_tools_allowed` to `src/agents/allow_list.rs`

**Files:**
- Create: `src/agents/allow_list.rs`
- Modify: `src/agents/mod.rs`
- Modify: `src/main.rs` (remove moved function; update 3 call sites)

- [ ] **Step 1: Create `src/agents/allow_list.rs`** (verbatim copy of current body)

```rust
//! Auto-allow the illu MCP tool pattern in Claude-family settings files.

use std::path::Path;

/// Ensure all illu MCP tools are auto-allowed in the settings file at `config_path`.
///
/// The tool pattern (`mcp__illu__*`) is appended to `permissions.allow` if not
/// already present. Other keys are preserved; unparseable files are replaced
/// with a minimal valid structure.
pub fn ensure_tools_allowed(config_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut config: serde_json::Value = std::fs::read_to_string(config_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));

    let pattern = "mcp__illu__*";

    let allow = config["permissions"]["allow"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let already = allow
        .iter()
        .any(|v| v.as_str().is_some_and(|s| s == pattern));

    if !already {
        let mut allow = allow;
        allow.push(serde_json::json!(pattern));
        config["permissions"]["allow"] = serde_json::Value::Array(allow);
        std::fs::write(config_path, serde_json::to_string_pretty(&config)?)?;
    }

    Ok(())
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn adds_pattern_to_empty_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        ensure_tools_allowed(&path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("mcp__illu__*"));
    }

    #[test]
    fn is_idempotent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        ensure_tools_allowed(&path).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        ensure_tools_allowed(&path).unwrap();
        let second = std::fs::read_to_string(&path).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn preserves_existing_permissions() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"permissions":{"allow":["existing_*"]},"other":"keep"}"#,
        )
        .unwrap();
        ensure_tools_allowed(&path).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let allow = v["permissions"]["allow"].as_array().unwrap();
        assert!(allow.iter().any(|e| e == "existing_*"));
        assert!(allow.iter().any(|e| e == "mcp__illu__*"));
        assert_eq!(v["other"], "keep");
    }
}
```

- [ ] **Step 2: Register the new sub-module in `src/agents/mod.rs`**

At the top of the file (below the doc comment, above the type defs):

```rust
pub mod allow_list;
```

- [ ] **Step 3: Remove the old function from `src/main.rs`**

Delete lines 586-615 (the `fn ensure_tools_allowed(...)` definition) and the existing test(s) for it inside `#[cfg(test)]` (find them — function name is `test_ensure_tools_allowed`; they were moved above in Step 1).

- [ ] **Step 4: Update call sites in `src/main.rs`**

Replace all three remaining `ensure_tools_allowed(...)` calls with `illu_rs::agents::allow_list::ensure_tools_allowed(...)`. They are at approximately lines 500, 683, 950 — search to find them after the deletion.

- [ ] **Step 5: Run tests**

Run: `cargo test --lib -- agents::allow_list`
Expected: 3 tests pass (`adds_pattern_to_empty_file`, `is_idempotent`, `preserves_existing_permissions`).

Run: `cargo test`
Expected: all existing tests still pass.

- [ ] **Step 6: Commit**

```bash
git add src/agents/allow_list.rs src/agents/mod.rs src/main.rs
git commit -m "refactor(agents): move ensure_tools_allowed to agents::allow_list"
```

---

### Task 3: Move instruction-md helpers to `src/agents/instruction_md.rs`

**Files:**
- Create: `src/agents/instruction_md.rs`
- Modify: `src/agents/mod.rs`
- Modify: `src/main.rs` (remove `ILLU_SECTION_START/END`, `illu_agent_section`, `write_md_section`, `write_claude_md_section`, `write_gemini_md_section`; update call sites)

- [ ] **Step 1: Create `src/agents/instruction_md.rs`**

Copy these items verbatim from `src/main.rs`:
- `ILLU_SECTION_START` (line 139)
- `ILLU_SECTION_END` (line 140)
- `illu_agent_section` (lines 142-186)
- `write_md_section` (lines 380-411)

Mark the two constants `pub const` and the two functions `pub fn`. Leave the function bodies unchanged.

- [ ] **Step 2: Add tests at the bottom of the new file**

```rust
#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn section_contains_tool_prefix() {
        let section = illu_agent_section("mcp__illu__");
        assert!(section.contains("mcp__illu__query"));
        assert!(section.contains(ILLU_SECTION_START));
        assert!(section.contains(ILLU_SECTION_END));
    }

    #[test]
    fn write_md_section_creates_file_when_missing() {
        let dir = tempdir().unwrap();
        let section = illu_agent_section("mcp__illu__");
        write_md_section(dir.path(), "CLAUDE.md", "# CLAUDE.md", &section).unwrap();
        let content = std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        assert!(content.starts_with("# CLAUDE.md"));
        assert!(content.contains(ILLU_SECTION_START));
    }

    #[test]
    fn write_md_section_is_idempotent() {
        let dir = tempdir().unwrap();
        let section = illu_agent_section("mcp__illu__");
        write_md_section(dir.path(), "CLAUDE.md", "# CLAUDE.md", &section).unwrap();
        let first = std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        write_md_section(dir.path(), "CLAUDE.md", "# CLAUDE.md", &section).unwrap();
        let second = std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn write_md_section_preserves_unrelated_content() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "# CLAUDE.md\n\nuser note\n").unwrap();
        let section = illu_agent_section("mcp__illu__");
        write_md_section(dir.path(), "CLAUDE.md", "# CLAUDE.md", &section).unwrap();
        let content = std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        assert!(content.contains("user note"));
        assert!(content.contains(ILLU_SECTION_START));
    }
}
```

- [ ] **Step 3: Register the sub-module in `src/agents/mod.rs`**

```rust
pub mod instruction_md;
```

- [ ] **Step 4: Remove the moved items from `src/main.rs` and update call sites**

Delete in `src/main.rs`:
- Lines 139-140 (`ILLU_SECTION_START/END`)
- Lines 142-186 (`illu_agent_section`)
- Lines 380-411 (`write_md_section`)
- `write_claude_md_section` (lines 413-416)
- `write_gemini_md_section` (lines 418-421)

Replace remaining call sites. Four locations:

1. Inside `init_repo` (replaces old `write_claude_md_section`/`write_gemini_md_section` calls):
```rust
illu_rs::agents::instruction_md::write_md_section(
    &repo_path,
    "CLAUDE.md",
    "# CLAUDE.md",
    &illu_rs::agents::instruction_md::illu_agent_section("mcp__illu__"),
)?;
illu_rs::agents::instruction_md::write_md_section(
    &repo_path,
    "GEMINI.md",
    "# GEMINI.md",
    &illu_rs::agents::instruction_md::illu_agent_section("mcp_illu_"),
)?;
```

2. Inside `install_global` (lines 691-706 area): replace the two `write_md_section` calls so they go through the new module path.

3. Inside `main`'s `Serve` branch (current lines ~944-945): same two-call pattern as `init_repo`.

- [ ] **Step 5: Run tests**

Run: `cargo test --lib -- agents::instruction_md`
Expected: 4 tests pass.

Run: `cargo test`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add src/agents/instruction_md.rs src/agents/mod.rs src/main.rs
git commit -m "refactor(agents): move instruction-md helpers to agents::instruction_md"
```

---

### Task 4: Move agent-defs generation to `src/agents/agent_files.rs`

**Files:**
- Create: `src/agents/agent_files.rs`
- Modify: `src/agents/mod.rs`
- Modify: `src/main.rs` (remove `AGENT_DEFS`, `BUILTIN_TOOLS`, `generate_agent_files`, move their existing tests)

- [ ] **Step 1: Create `src/agents/agent_files.rs`**

Copy verbatim from `src/main.rs`:
- `AGENT_DEFS` (lines 188-346), mark `pub const`.
- `BUILTIN_TOOLS` (line 348), mark `pub const`.
- `generate_agent_files` (lines 350-378), mark `pub fn`.

Also copy the existing test `test_generate_agent_files_creates_three_files` (search for it in `src/main.rs`'s `#[cfg(test)]` block around line 1168). Move its supporting imports into the new module's test section.

- [ ] **Step 2: Register the sub-module in `src/agents/mod.rs`**

```rust
pub mod agent_files;
```

- [ ] **Step 3: Remove the moved items from `src/main.rs`**

Delete the three items and the moved test. Update the two remaining call sites (in `init_repo` and `install_global` and `main`'s `Serve` branch) to use `illu_rs::agents::agent_files::generate_agent_files(...)`.

- [ ] **Step 4: Run tests**

Run: `cargo test --lib -- agents::agent_files`
Expected: 1 test (`test_generate_agent_files_creates_three_files`) passes.

Run: `cargo test`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/agents/agent_files.rs src/agents/mod.rs src/main.rs
git commit -m "refactor(agents): move agent-files generation to agents::agent_files"
```

---

### Task 5: Clippy + format checkpoint

- [ ] **Step 1: Run clippy and fmt**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: no warnings.

Run: `cargo fmt --all -- --check`
Expected: no output.

- [ ] **Step 2: If anything fails, fix inline and re-run; then commit**

If fixes were needed:
```bash
git add -A
git commit -m "refactor(agents): satisfy clippy after code moves"
```

---

## Phase 3 — Build new capabilities (test-first)

### Task 6: Detection context trait and implementations

**Files:**
- Create: `src/agents/detect.rs`
- Modify: `src/agents/mod.rs`

- [ ] **Step 1: Write failing tests in `src/agents/detect.rs`**

```rust
//! Detection of installed and active agents.

use super::{Agent, Detection, DetectionLevel};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Environment the detector sees. Abstracted for testability.
pub trait DetectionContext {
    fn env_var(&self, name: &str) -> Option<String>;
    fn binary_on_path(&self, name: &str) -> bool;
    fn path_exists(&self, path: &Path) -> bool;
    fn home(&self) -> &Path;
    fn os(&self) -> TargetOs;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TargetOs {
    MacOs,
    Linux,
    Windows,
}

pub fn detect_level(agent: &Agent, ctx: &dyn DetectionContext) -> DetectionLevel {
    // Active: any env var present
    for var in agent.detection.env_vars {
        if ctx.env_var(var).is_some() {
            return DetectionLevel::Active;
        }
    }
    // Installed: binary on PATH, or config dir present, or (macOS) app bundle
    for bin in agent.detection.binaries {
        if ctx.binary_on_path(bin) {
            return DetectionLevel::Installed;
        }
    }
    for rel in agent.detection.config_dirs {
        if ctx.path_exists(&ctx.home().join(rel)) {
            return DetectionLevel::Installed;
        }
    }
    if ctx.os() == TargetOs::MacOs {
        for bundle in agent.detection.app_bundles {
            if ctx.path_exists(Path::new(bundle)) {
                return DetectionLevel::Installed;
            }
        }
    }
    DetectionLevel::Unknown
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::agents::{Agent, Detection};
    use std::collections::HashSet;

    struct MockCtx {
        env: HashMap<String, String>,
        path_bins: HashSet<String>,
        fs: HashSet<PathBuf>,
        home: PathBuf,
        os: TargetOs,
    }

    impl DetectionContext for MockCtx {
        fn env_var(&self, name: &str) -> Option<String> {
            self.env.get(name).cloned()
        }
        fn binary_on_path(&self, name: &str) -> bool {
            self.path_bins.contains(name)
        }
        fn path_exists(&self, path: &Path) -> bool {
            self.fs.contains(path)
        }
        fn home(&self) -> &Path {
            &self.home
        }
        fn os(&self) -> TargetOs {
            self.os
        }
    }

    fn sample_agent() -> Agent {
        Agent {
            id: "x",
            display_name: "X",
            detection: Detection {
                env_vars: &["XCODE"],
                binaries: &["x"],
                config_dirs: &[".x"],
                app_bundles: &["/Applications/X.app"],
            },
            repo_config: None,
            global_config: None,
            tool_prefix: "mcp__x__",
        }
    }

    fn empty_ctx() -> MockCtx {
        MockCtx {
            env: HashMap::new(),
            path_bins: HashSet::new(),
            fs: HashSet::new(),
            home: PathBuf::from("/home/test"),
            os: TargetOs::Linux,
        }
    }

    #[test]
    fn active_when_env_var_set() {
        let mut ctx = empty_ctx();
        ctx.env.insert("XCODE".into(), "1".into());
        assert_eq!(detect_level(&sample_agent(), &ctx), DetectionLevel::Active);
    }

    #[test]
    fn installed_when_binary_on_path() {
        let mut ctx = empty_ctx();
        ctx.path_bins.insert("x".into());
        assert_eq!(
            detect_level(&sample_agent(), &ctx),
            DetectionLevel::Installed
        );
    }

    #[test]
    fn installed_when_config_dir_exists() {
        let mut ctx = empty_ctx();
        ctx.fs.insert(ctx.home.join(".x"));
        assert_eq!(
            detect_level(&sample_agent(), &ctx),
            DetectionLevel::Installed
        );
    }

    #[test]
    fn installed_when_app_bundle_exists_on_macos() {
        let mut ctx = empty_ctx();
        ctx.os = TargetOs::MacOs;
        ctx.fs.insert(PathBuf::from("/Applications/X.app"));
        assert_eq!(
            detect_level(&sample_agent(), &ctx),
            DetectionLevel::Installed
        );
    }

    #[test]
    fn app_bundle_ignored_on_linux() {
        let mut ctx = empty_ctx();
        ctx.fs.insert(PathBuf::from("/Applications/X.app"));
        assert_eq!(detect_level(&sample_agent(), &ctx), DetectionLevel::Unknown);
    }

    #[test]
    fn unknown_when_no_signal() {
        assert_eq!(detect_level(&sample_agent(), &empty_ctx()), DetectionLevel::Unknown);
    }

    #[test]
    fn env_var_wins_over_installed_signal() {
        let mut ctx = empty_ctx();
        ctx.env.insert("XCODE".into(), "1".into());
        ctx.path_bins.insert("x".into());
        assert_eq!(detect_level(&sample_agent(), &ctx), DetectionLevel::Active);
    }
}
```

- [ ] **Step 2: Add `pub mod detect;` to `src/agents/mod.rs`**

- [ ] **Step 3: Run tests**

Run: `cargo test --lib -- agents::detect`
Expected: all 7 tests pass.

- [ ] **Step 4: Add a real-system `DetectionContext` implementation at the bottom of `src/agents/detect.rs`**

```rust
/// Real detection context backed by the process environment and filesystem.
pub struct RealContext {
    home: PathBuf,
    os: TargetOs,
}

impl RealContext {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map(PathBuf::from)
            .map_err(|_| "neither HOME nor USERPROFILE set")?;
        let os = match std::env::consts::OS {
            "macos" => TargetOs::MacOs,
            "windows" => TargetOs::Windows,
            _ => TargetOs::Linux,
        };
        Ok(Self { home, os })
    }
}

impl DetectionContext for RealContext {
    fn env_var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    fn binary_on_path(&self, name: &str) -> bool {
        let Some(path) = std::env::var_os("PATH") else {
            return false;
        };
        std::env::split_paths(&path).any(|dir| {
            let candidate = dir.join(name);
            candidate.is_file()
                || candidate.with_extension("exe").is_file()
        })
    }

    fn path_exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn home(&self) -> &Path {
        &self.home
    }

    fn os(&self) -> TargetOs {
        self.os
    }
}
```

- [ ] **Step 5: Commit**

```bash
git add src/agents/detect.rs src/agents/mod.rs
git commit -m "feat(agents): add detection trait and real/mock implementations"
```

---

### Task 7: Global path resolution helper

**Files:**
- Create: `src/agents/paths.rs`
- Modify: `src/agents/mod.rs`

- [ ] **Step 1: Write `src/agents/paths.rs` with tests first**

```rust
//! Platform-aware resolution of `GlobalPath` into real filesystem paths.

use super::GlobalPath;
use super::detect::TargetOs;
use std::path::{Path, PathBuf};

pub fn resolve(global: &GlobalPath, os: TargetOs, home: &Path) -> PathBuf {
    match global {
        GlobalPath::Home(rel) => home.join(rel),
        GlobalPath::AppSupport(vendor, file) => match os {
            TargetOs::MacOs => home.join("Library/Application Support").join(vendor).join(file),
            TargetOs::Windows => home.join("AppData/Roaming").join(vendor).join(file),
            TargetOs::Linux => home.join(".config").join(vendor).join(file),
        },
        GlobalPath::AppData(vendor, file) => match os {
            TargetOs::Windows => home.join("AppData/Roaming").join(vendor).join(file),
            _ => home.join(".config").join(vendor).join(file),
        },
        GlobalPath::XdgConfig(rel) => home.join(".config").join(rel),
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn home_joins_relative() {
        let resolved = resolve(&GlobalPath::Home(".claude/settings.json"), TargetOs::Linux, Path::new("/h"));
        assert_eq!(resolved, Path::new("/h/.claude/settings.json"));
    }

    #[test]
    fn app_support_macos() {
        let resolved = resolve(
            &GlobalPath::AppSupport("Claude", "claude_desktop_config.json"),
            TargetOs::MacOs,
            Path::new("/h"),
        );
        assert_eq!(
            resolved,
            Path::new("/h/Library/Application Support/Claude/claude_desktop_config.json")
        );
    }

    #[test]
    fn app_support_windows() {
        let resolved = resolve(
            &GlobalPath::AppSupport("Claude", "claude_desktop_config.json"),
            TargetOs::Windows,
            Path::new("/h"),
        );
        assert_eq!(resolved, Path::new("/h/AppData/Roaming/Claude/claude_desktop_config.json"));
    }

    #[test]
    fn app_support_linux_uses_xdg_like_path() {
        let resolved = resolve(
            &GlobalPath::AppSupport("Claude", "claude_desktop_config.json"),
            TargetOs::Linux,
            Path::new("/h"),
        );
        assert_eq!(resolved, Path::new("/h/.config/Claude/claude_desktop_config.json"));
    }

    #[test]
    fn xdg_config_uses_dot_config() {
        let resolved = resolve(&GlobalPath::XdgConfig("antigravity/mcp.json"), TargetOs::Linux, Path::new("/h"));
        assert_eq!(resolved, Path::new("/h/.config/antigravity/mcp.json"));
    }
}
```

- [ ] **Step 2: Register the module**

In `src/agents/mod.rs`: `pub mod paths;`

- [ ] **Step 3: Run tests**

Run: `cargo test --lib -- agents::paths`
Expected: 5 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/agents/paths.rs src/agents/mod.rs
git commit -m "feat(agents): add platform-aware global path resolver"
```

---

### Task 8: MCP format writers — shared helpers + `mcpServers`-style JSON

Handles the four shapes that share a `mcpServers.illu` layout: `ClaudeCodeJson`, `GeminiJson`, `ClaudeDesktopJson`, `CursorJson`, plus the placeholder `AntigravityJson`.

**Files:**
- Create: `src/agents/formats.rs`
- Modify: `src/agents/mod.rs`

- [ ] **Step 1: Create `src/agents/formats.rs` with tests first**

```rust
//! Writers for each MCP config file format.

use super::{IlluCommand, McpFormat};
use std::path::Path;

pub fn write(path: &Path, format: McpFormat, cmd: &IlluCommand) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match format {
        McpFormat::ClaudeCodeJson
        | McpFormat::GeminiJson
        | McpFormat::ClaudeDesktopJson
        | McpFormat::CursorJson
        | McpFormat::AntigravityJson => write_mcp_servers_json(path, cmd),
        McpFormat::VsCodeJson => write_vscode_json(path, cmd),
        McpFormat::CodexToml => write_codex_toml(path, cmd),
    }
}

fn write_mcp_servers_json(path: &Path, cmd: &IlluCommand) -> Result<(), Box<dyn std::error::Error>> {
    let mut config: serde_json::Value = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({"mcpServers": {}}));

    let entry = serde_json::json!({
        "command": cmd.command,
        "args": cmd.args,
        "env": { "RUST_LOG": "warn" }
    });
    config["mcpServers"]["illu"] = entry;
    std::fs::write(path, serde_json::to_string_pretty(&config)?)?;
    Ok(())
}

fn write_vscode_json(path: &Path, cmd: &IlluCommand) -> Result<(), Box<dyn std::error::Error>> {
    let mut config: serde_json::Value = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({"servers": {}}));

    let entry = serde_json::json!({
        "type": "stdio",
        "command": cmd.command,
        "args": cmd.args,
        "env": { "RUST_LOG": "warn" }
    });
    config["servers"]["illu"] = entry;
    std::fs::write(path, serde_json::to_string_pretty(&config)?)?;
    Ok(())
}

fn write_codex_toml(path: &Path, cmd: &IlluCommand) -> Result<(), Box<dyn std::error::Error>> {
    use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value};

    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let mut doc = existing.parse::<DocumentMut>()?;

    let mcp_servers = doc
        .entry("mcp_servers")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .ok_or("mcp_servers is not a table")?;
    mcp_servers.set_implicit(true);

    let illu = mcp_servers
        .entry("illu")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .ok_or("mcp_servers.illu is not a table")?;

    illu.insert("command", Item::Value(Value::from(cmd.command.clone())));
    let mut args = Array::new();
    for a in &cmd.args {
        args.push(a.clone());
    }
    illu.insert("args", Item::Value(Value::Array(args)));

    let mut env = InlineTable::new();
    env.insert("RUST_LOG", Value::from("warn"));
    illu.insert("env", Item::Value(Value::InlineTable(env)));

    std::fs::write(path, doc.to_string())?;
    Ok(())
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn cmd() -> IlluCommand {
        IlluCommand::serve()
    }

    #[test]
    fn writes_mcp_servers_json_fresh() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".mcp.json");
        write(&path, McpFormat::ClaudeCodeJson, &cmd()).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["illu"]["command"], "illu-rs");
        assert_eq!(v["mcpServers"]["illu"]["args"][0], "serve");
    }

    #[test]
    fn preserves_other_mcp_servers() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".mcp.json");
        std::fs::write(
            &path,
            r#"{"mcpServers":{"other":{"command":"x"}}}"#,
        )
        .unwrap();
        write(&path, McpFormat::ClaudeCodeJson, &cmd()).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["other"]["command"], "x");
        assert_eq!(v["mcpServers"]["illu"]["command"], "illu-rs");
    }

    #[test]
    fn vscode_uses_servers_key_and_type_stdio() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".vscode/mcp.json");
        write(&path, McpFormat::VsCodeJson, &cmd()).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["servers"]["illu"]["type"], "stdio");
        assert_eq!(v["servers"]["illu"]["command"], "illu-rs");
    }

    #[test]
    fn codex_toml_writes_mcp_servers_illu_section() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write(&path, McpFormat::CodexToml, &cmd()).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("[mcp_servers.illu]"));
        assert!(content.contains("command = \"illu-rs\""));
        assert!(content.contains("args = [\"serve\"]"));
    }

    #[test]
    fn codex_toml_preserves_unrelated_sections() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[user]\nname = \"alice\"\n\n[mcp_servers.existing]\ncommand = \"x\"\n",
        )
        .unwrap();
        write(&path, McpFormat::CodexToml, &cmd()).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("[user]"));
        assert!(content.contains("name = \"alice\""));
        assert!(content.contains("[mcp_servers.existing]"));
        assert!(content.contains("[mcp_servers.illu]"));
    }

    #[test]
    fn idempotent_rewrites() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".mcp.json");
        write(&path, McpFormat::ClaudeCodeJson, &cmd()).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        write(&path, McpFormat::ClaudeCodeJson, &cmd()).unwrap();
        let second = std::fs::read_to_string(&path).unwrap();
        assert_eq!(first, second);
    }
}
```

- [ ] **Step 2: Add `toml_edit` to `Cargo.toml`**

Under `[dependencies]`:

```toml
toml_edit = "0.22"
```

- [ ] **Step 3: Register the module in `src/agents/mod.rs`**

```rust
pub mod formats;
pub mod paths;
```

(If `paths` already registered in Task 7, just add `formats`.)

- [ ] **Step 4: Run tests**

Run: `cargo test --lib -- agents::formats`
Expected: 6 tests pass.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/agents/formats.rs src/agents/mod.rs
git commit -m "feat(agents): add MCP format writers for JSON and TOML shapes"
```

---

### Task 9: Pure selection logic

**Files:**
- Create: `src/agents/selection.rs`
- Modify: `src/agents/mod.rs`

- [ ] **Step 1: Create `src/agents/selection.rs` with tests first**

```rust
//! Pure selection logic: given detection results and flags, return which agents to configure.

use super::{Agent, DetectionLevel, SetupFlags};

pub enum Mode {
    Explicit,   // --agent X [--agent Y ...]
    All,        // --all
    AutoDetect, // --yes or no TTY
    Interactive,// prompt the user
}

pub fn mode(flags: &SetupFlags, has_tty: bool) -> Mode {
    if !flags.explicit_agents.is_empty() {
        Mode::Explicit
    } else if flags.all {
        Mode::All
    } else if flags.yes || !has_tty {
        Mode::AutoDetect
    } else {
        Mode::Interactive
    }
}

pub fn select_from_flags<'a>(
    agents: &'a [Agent],
    flags: &SetupFlags,
    detection: &[(& 'a Agent, DetectionLevel)],
    has_tty: bool,
) -> Result<Vec<&'a Agent>, SelectionError> {
    match mode(flags, has_tty) {
        Mode::Explicit => select_explicit(agents, &flags.explicit_agents),
        Mode::All => Ok(agents.iter().collect()),
        Mode::AutoDetect => Ok(detected(detection)),
        Mode::Interactive => Err(SelectionError::NeedsPrompt),
    }
}

fn select_explicit<'a>(
    agents: &'a [Agent],
    ids: &[String],
) -> Result<Vec<&'a Agent>, SelectionError> {
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        match agents.iter().find(|a| a.id == id) {
            Some(a) => out.push(a),
            None => return Err(SelectionError::UnknownId(id.clone())),
        }
    }
    Ok(out)
}

fn detected<'a>(results: &[(&'a Agent, DetectionLevel)]) -> Vec<&'a Agent> {
    results
        .iter()
        .filter(|(_, lvl)| matches!(lvl, DetectionLevel::Active | DetectionLevel::Installed))
        .map(|(a, _)| *a)
        .collect()
}

#[derive(Debug, thiserror::Error)]
pub enum SelectionError {
    #[error("unknown agent id: {0}")]
    UnknownId(String),
    #[error("selection requires an interactive prompt")]
    NeedsPrompt,
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::agents::{Detection, McpFormat, RepoConfig};

    fn agent(id: &'static str) -> Agent {
        Agent {
            id,
            display_name: id,
            detection: Detection {
                env_vars: &[],
                binaries: &[],
                config_dirs: &[],
                app_bundles: &[],
            },
            repo_config: Some(RepoConfig {
                mcp_config_path: "x.json",
                mcp_format: McpFormat::ClaudeCodeJson,
                instruction_file: None,
                agents_dir: None,
                allow_list_path: None,
            }),
            global_config: None,
            tool_prefix: "mcp__x__",
        }
    }

    #[test]
    fn explicit_single_agent() {
        let a = agent("x");
        let b = agent("y");
        let agents = &[a, b];
        let flags = SetupFlags {
            explicit_agents: vec!["x".into()],
            ..SetupFlags::default()
        };
        let picked = select_from_flags(agents, &flags, &[], false).unwrap();
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].id, "x");
    }

    #[test]
    fn explicit_unknown_errors() {
        let a = agent("x");
        let flags = SetupFlags {
            explicit_agents: vec!["z".into()],
            ..SetupFlags::default()
        };
        let err = select_from_flags(&[a], &flags, &[], false).unwrap_err();
        assert!(matches!(err, SelectionError::UnknownId(_)));
    }

    #[test]
    fn all_selects_every_agent() {
        let a = agent("x");
        let b = agent("y");
        let agents = &[a, b];
        let flags = SetupFlags {
            all: true,
            ..SetupFlags::default()
        };
        let picked = select_from_flags(agents, &flags, &[], false).unwrap();
        assert_eq!(picked.len(), 2);
    }

    #[test]
    fn yes_uses_detection() {
        let a = agent("x");
        let b = agent("y");
        let agents = &[a, b];
        let detection = vec![
            (&agents[0], DetectionLevel::Installed),
            (&agents[1], DetectionLevel::Unknown),
        ];
        let flags = SetupFlags {
            yes: true,
            ..SetupFlags::default()
        };
        let picked = select_from_flags(agents, &flags, &detection, false).unwrap();
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].id, "x");
    }

    #[test]
    fn no_tty_behaves_like_yes() {
        let a = agent("x");
        let b = agent("y");
        let agents = &[a, b];
        let detection = vec![
            (&agents[0], DetectionLevel::Active),
            (&agents[1], DetectionLevel::Unknown),
        ];
        let flags = SetupFlags::default();
        let picked = select_from_flags(agents, &flags, &detection, false).unwrap();
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].id, "x");
    }

    #[test]
    fn interactive_returns_needs_prompt() {
        let a = agent("x");
        let flags = SetupFlags::default();
        let err = select_from_flags(&[a], &flags, &[], true).unwrap_err();
        assert!(matches!(err, SelectionError::NeedsPrompt));
    }
}
```

- [ ] **Step 2: Register the module**

In `src/agents/mod.rs`: `pub mod selection;`

- [ ] **Step 3: Run tests**

Run: `cargo test --lib -- agents::selection`
Expected: 6 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/agents/selection.rs src/agents/mod.rs
git commit -m "feat(agents): add pure selection logic for setup flags"
```

---

### Task 10: Interactive multi-select prompt

**Files:**
- Create: `src/agents/prompt.rs`
- Modify: `src/agents/mod.rs`
- Modify: `Cargo.toml`

- [ ] **Step 1: Add `dialoguer` to `Cargo.toml`**

Under `[dependencies]`:

```toml
dialoguer = { version = "0.11", default-features = false }
```

- [ ] **Step 2: Create `src/agents/prompt.rs`**

```rust
//! Interactive multi-select prompt for agent selection.

use super::{Agent, DetectionLevel};

pub fn prompt_agents<'a>(
    detection: &[(&'a Agent, DetectionLevel, String)],
) -> Result<Vec<&'a Agent>, Box<dyn std::error::Error>> {
    use dialoguer::MultiSelect;
    use dialoguer::theme::ColorfulTheme;

    let labels: Vec<String> = detection
        .iter()
        .map(|(a, lvl, reason)| match lvl {
            DetectionLevel::Active | DetectionLevel::Installed => {
                format!("{:<24} (detected: {})", a.display_name, reason)
            }
            DetectionLevel::Unknown => format!("{:<24} (not detected)", a.display_name),
        })
        .collect();

    let defaults: Vec<bool> = detection
        .iter()
        .map(|(_, lvl, _)| {
            matches!(lvl, DetectionLevel::Active | DetectionLevel::Installed)
        })
        .collect();

    let selected_indices = MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt("Which agents should illu configure?")
        .items(&labels)
        .defaults(&defaults)
        .interact()?;

    Ok(selected_indices
        .into_iter()
        .map(|i| detection[i].0)
        .collect())
}

pub fn has_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}
```

- [ ] **Step 3: Register the module**

In `src/agents/mod.rs`: `pub mod prompt;`

- [ ] **Step 4: Build and confirm no warnings**

Run: `cargo build`
Expected: compiles. No test for prompt rendering itself (TUI); the pure selection path is already covered.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/agents/prompt.rs src/agents/mod.rs
git commit -m "feat(agents): add dialoguer-based multi-select prompt"
```

---

### Task 11: Orchestrator entry points

**Files:**
- Modify: `src/agents/mod.rs`

- [ ] **Step 1: Append orchestrator code to `src/agents/mod.rs`**

Add below the type definitions and `pub static AGENTS`:

```rust
pub mod agent_files;
pub mod allow_list;
pub mod detect;
pub mod formats;
pub mod instruction_md;
pub mod paths;
pub mod prompt;
pub mod selection;

use std::path::{Path, PathBuf};

/// Detect every agent with a per-repo config target.
pub fn detect_repo_agents<'a>(
    ctx: &dyn detect::DetectionContext,
) -> Vec<(&'a Agent, DetectionLevel, String)> {
    detect_scoped(ctx, |a| a.repo_config.is_some())
}

/// Detect every agent with a global config target.
pub fn detect_global_agents<'a>(
    ctx: &dyn detect::DetectionContext,
) -> Vec<(&'a Agent, DetectionLevel, String)> {
    detect_scoped(ctx, |a| a.global_config.is_some())
}

fn detect_scoped<'a>(
    ctx: &dyn detect::DetectionContext,
    filter: impl Fn(&&Agent) -> bool,
) -> Vec<(&'a Agent, DetectionLevel, String)> {
    AGENTS
        .iter()
        .filter(filter)
        .map(|a| {
            let lvl = detect::detect_level(a, ctx);
            let reason = detection_reason(a, ctx, lvl);
            (a, lvl, reason)
        })
        .collect()
}

fn detection_reason(agent: &Agent, ctx: &dyn detect::DetectionContext, lvl: DetectionLevel) -> String {
    match lvl {
        DetectionLevel::Active => agent
            .detection
            .env_vars
            .iter()
            .find(|v| ctx.env_var(v).is_some())
            .map_or_else(|| "env".to_string(), |v| format!("env:{v}")),
        DetectionLevel::Installed => {
            if let Some(b) = agent.detection.binaries.iter().find(|b| ctx.binary_on_path(b)) {
                format!("binary:{b}")
            } else if let Some(d) = agent
                .detection
                .config_dirs
                .iter()
                .find(|d| ctx.path_exists(&ctx.home().join(d)))
            {
                format!("~/{d}")
            } else if let Some(b) = agent
                .detection
                .app_bundles
                .iter()
                .find(|b| ctx.path_exists(Path::new(b)))
            {
                (*b).to_string()
            } else {
                "installed".to_string()
            }
        }
        DetectionLevel::Unknown => String::new(),
    }
}

/// Configure per-repo agents in `repo_path` according to `flags`.
pub fn configure_repo(
    repo_path: &Path,
    flags: &SetupFlags,
) -> Result<Vec<AgentWriteReport>, Box<dyn std::error::Error>> {
    let ctx = detect::RealContext::new()?;
    let detection = detect_repo_agents(&ctx);
    let chosen = resolve_selection(&detection, flags)?;
    let cmd = IlluCommand::serve();
    let mut reports = Vec::with_capacity(chosen.len());
    for agent in chosen {
        let report = write_repo_for(agent, repo_path, &cmd, flags.dry_run)?;
        reports.push(report);
    }
    Ok(reports)
}

/// Configure global agents in `home` according to `flags`.
pub fn configure_global(
    home: &Path,
    flags: &SetupFlags,
) -> Result<Vec<AgentWriteReport>, Box<dyn std::error::Error>> {
    let ctx = detect::RealContext::new()?;
    let detection = detect_global_agents(&ctx);
    let chosen = resolve_selection(&detection, flags)?;
    let cmd = IlluCommand::serve();
    let mut reports = Vec::with_capacity(chosen.len());
    for agent in chosen {
        let report = write_global_for(agent, home, ctx.os(), &cmd, flags.dry_run)?;
        reports.push(report);
    }
    Ok(reports)
}

/// Detect via env vars only and write configs for any `Active` agent.
/// Called by `illu serve` on startup.
pub fn self_heal_on_serve(
    repo_path: Option<&Path>,
    home: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let ctx = detect::RealContext::new()?;
    let cmd = IlluCommand::serve();
    for agent in AGENTS {
        if detect::detect_level(agent, &ctx) != DetectionLevel::Active {
            continue;
        }
        if let (Some(repo), Some(_)) = (repo_path, &agent.repo_config) {
            let _ = write_repo_for(agent, repo, &cmd, false);
        }
        if agent.global_config.is_some() {
            let _ = write_global_for(agent, home, ctx.os(), &cmd, false);
        }
    }
    Ok(())
}

fn resolve_selection<'a>(
    detection: &[(&'a Agent, DetectionLevel, String)],
    flags: &SetupFlags,
) -> Result<Vec<&'a Agent>, Box<dyn std::error::Error>> {
    let pairs: Vec<(&Agent, DetectionLevel)> =
        detection.iter().map(|(a, l, _)| (*a, *l)).collect();
    match selection::select_from_flags(AGENTS, flags, &pairs, prompt::has_tty()) {
        Ok(picked) => Ok(picked),
        Err(selection::SelectionError::UnknownId(id)) => {
            Err(format!("unknown agent id: {id}").into())
        }
        Err(selection::SelectionError::NeedsPrompt) => {
            prompt::prompt_agents(detection).map_err(Into::into)
        }
    }
}

fn write_repo_for(
    agent: &Agent,
    repo_path: &Path,
    cmd: &IlluCommand,
    dry_run: bool,
) -> Result<AgentWriteReport, Box<dyn std::error::Error>> {
    let mut written = Vec::new();
    let Some(cfg) = &agent.repo_config else {
        return Ok(AgentWriteReport {
            agent_id: agent.id,
            written_paths: written,
            skipped: true,
        });
    };
    let mcp_path = repo_path.join(cfg.mcp_config_path);
    if !dry_run {
        formats::write(&mcp_path, cfg.mcp_format, cmd)?;
    }
    written.push(mcp_path);
    if let Some(md_rel) = cfg.instruction_file {
        let heading = format!("# {md_rel}");
        if !dry_run {
            instruction_md::write_md_section(
                repo_path,
                md_rel,
                &heading,
                &instruction_md::illu_agent_section(agent.tool_prefix),
            )?;
        }
        written.push(repo_path.join(md_rel));
    }
    if let Some(ad_rel) = cfg.agents_dir {
        let target = repo_path.join(ad_rel);
        if !dry_run {
            agent_files::generate_agent_files(&target, agent.tool_prefix)?;
        }
        written.push(target);
    }
    if let Some(al_rel) = cfg.allow_list_path {
        let target = repo_path.join(al_rel);
        if !dry_run {
            allow_list::ensure_tools_allowed(&target)?;
        }
        written.push(target);
    }
    Ok(AgentWriteReport {
        agent_id: agent.id,
        written_paths: written,
        skipped: false,
    })
}

fn write_global_for(
    agent: &Agent,
    home: &Path,
    os: detect::TargetOs,
    cmd: &IlluCommand,
    dry_run: bool,
) -> Result<AgentWriteReport, Box<dyn std::error::Error>> {
    let mut written = Vec::new();
    let Some(cfg) = &agent.global_config else {
        return Ok(AgentWriteReport {
            agent_id: agent.id,
            written_paths: written,
            skipped: true,
        });
    };
    let mcp_path = paths::resolve(&cfg.mcp_config_path, os, home);
    if !dry_run {
        formats::write(&mcp_path, cfg.mcp_format, cmd)?;
    }
    written.push(mcp_path);
    if let Some(md_gp) = &cfg.instruction_file {
        let md_path = paths::resolve(md_gp, os, home);
        let file_name = md_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("instruction file has no name")?;
        let heading = format!("# {file_name}");
        if !dry_run {
            let parent = md_path.parent().ok_or("instruction file has no parent")?;
            std::fs::create_dir_all(parent)?;
            instruction_md::write_md_section(
                parent,
                file_name,
                &heading,
                &instruction_md::illu_agent_section(agent.tool_prefix),
            )?;
        }
        written.push(md_path);
    }
    if let Some(ad_gp) = &cfg.agents_dir {
        let target = paths::resolve(ad_gp, os, home);
        if !dry_run {
            agent_files::generate_agent_files(&target, agent.tool_prefix)?;
        }
        written.push(target);
    }
    if let Some(al_gp) = &cfg.allow_list_path {
        let target = paths::resolve(al_gp, os, home);
        if !dry_run {
            allow_list::ensure_tools_allowed(&target)?;
        }
        written.push(target);
    }
    let _ = cmd;
    Ok(AgentWriteReport {
        agent_id: agent.id,
        written_paths: written,
        skipped: false,
    })
}
```

- [ ] **Step 2: Move the `pub mod` declarations to the top of the file**

Ensure the `pub mod ...` declarations come before the `use std::path::...` in the new orchestrator block. (If Task 6/7/8/9/10 already placed them at the top, delete the duplicates in Step 1.)

- [ ] **Step 3: Build**

Run: `cargo build`
Expected: compiles with no warnings. `AGENTS` is still `&[]` so orchestrators trivially do nothing.

- [ ] **Step 4: Commit**

```bash
git add src/agents/mod.rs
git commit -m "feat(agents): add configure_repo/configure_global/self_heal_on_serve entry points"
```

---

## Phase 4 — CLI integration

### Task 12: Extend `Command::Init` and `Command::Install` with flags

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Replace the `Init` and `Install` variants in the `Command` enum**

Locate lines ~77-80 of `src/main.rs` and replace:

```rust
/// Set up illu in a repo (detects/prompts per-repo agents, builds index)
Init {
    /// Configure a specific agent (repeatable). Example: --agent claude-code
    #[arg(long)]
    agent: Vec<String>,
    /// Configure every supported per-repo agent without prompting.
    #[arg(long)]
    all: bool,
    /// Skip the prompt and accept detected agents.
    #[arg(long, short = 'y')]
    yes: bool,
    /// Print what would be written without touching the filesystem.
    #[arg(long)]
    dry_run: bool,
},
/// Install illu globally (detects/prompts global agents)
Install {
    #[arg(long)]
    agent: Vec<String>,
    #[arg(long)]
    all: bool,
    #[arg(long, short = 'y')]
    yes: bool,
    #[arg(long)]
    dry_run: bool,
},
```

- [ ] **Step 2: Build**

Run: `cargo build`
Expected: fails in `main()` because the match arms `Some(Command::Init) =>` and `Some(Command::Install) =>` no longer match. That's fine — Task 13/14 fixes them.

- [ ] **Step 3: Do NOT commit yet** — tasks 13 and 14 depend on this change.

---

### Task 13: Rewrite `init_repo` to use the orchestrator

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Replace `fn init_repo` body**

Replace the full function (currently lines ~462-516) with:

```rust
#[expect(clippy::print_stdout, reason = "CLI output")]
fn init_repo(
    repo_path: &Path,
    flags: illu_rs::agents::SetupFlags,
) -> Result<(), Box<dyn std::error::Error>> {
    let repo_path = repo_path.canonicalize()?;

    let has_cargo = repo_path.join("Cargo.toml").exists();
    let has_ts =
        repo_path.join("tsconfig.json").exists() || repo_path.join("package.json").exists();
    let has_python = illu_rs::indexer::has_python_project(&repo_path);
    if !has_cargo && !has_ts && !has_python {
        return Err(format!(
            "No Cargo.toml, tsconfig.json, package.json, or Python project found in {}",
            repo_path.display()
        )
        .into());
    }

    println!("Setting up illu in {}", repo_path.display());

    let reports = illu_rs::agents::configure_repo(&repo_path, &flags)?;
    for report in &reports {
        if report.skipped {
            continue;
        }
        println!("  configured {}", report.agent_id);
        for path in &report.written_paths {
            println!("    -> {}", path.display());
        }
    }
    if reports.is_empty() || reports.iter().all(|r| r.skipped) {
        println!("  no agents configured (nothing detected, nothing passed via --agent)");
    }

    if flags.dry_run {
        println!("\n(dry run — no files written)");
        return Ok(());
    }

    println!("  indexing...");
    illu_rs::status::init(&repo_path);
    ensure_indexed(&repo_path)?;
    println!("  index built");

    if ensure_gitignore(&repo_path)? {
        println!("  updated .gitignore with illu entries");
    }

    println!("\nDone.");
    Ok(())
}
```

- [ ] **Step 2: Update the `Some(Command::Init)` match arm in `main`**

Locate the current `Some(Command::Init) => { init_repo(repo_path)?; }` (approx line 1149) and replace with:

```rust
Some(Command::Init { agent, all, yes, dry_run }) => {
    let flags = illu_rs::agents::SetupFlags {
        explicit_agents: agent,
        all,
        yes,
        dry_run,
    };
    init_repo(repo_path, flags)?;
}
```

- [ ] **Step 3: Build**

Run: `cargo build`
Expected: compiles. `Command::Install` still broken — that's Task 14.

- [ ] **Step 4: Do NOT commit yet** — Task 14 is the paired change.

---

### Task 14: Rewrite `install_global` to use the orchestrator

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Replace `fn install_global` body**

Replace the full function (currently lines ~675-717) with:

```rust
#[expect(clippy::print_stdout, reason = "CLI output")]
fn install_global(flags: illu_rs::agents::SetupFlags) -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::var("HOME").map_err(|_| "HOME environment variable not set")?;
    let home = PathBuf::from(home);

    println!("Installing illu globally...");

    let reports = illu_rs::agents::configure_global(&home, &flags)?;
    for report in &reports {
        if report.skipped {
            continue;
        }
        println!("  configured {}", report.agent_id);
        for path in &report.written_paths {
            println!("    -> {}", path.display());
        }
    }
    if reports.is_empty() || reports.iter().all(|r| r.skipped) {
        println!("  no agents configured (nothing detected, nothing passed via --agent)");
    }

    if flags.dry_run {
        println!("\n(dry run — no files written)");
        return Ok(());
    }

    install_statusline(&home)?;
    ensure_global_gitignore(&home)?;

    println!("\nDone.");
    Ok(())
}
```

- [ ] **Step 2: Update the `Some(Command::Install)` match arm in `main`**

Replace with:

```rust
Some(Command::Install { agent, all, yes, dry_run }) => {
    let flags = illu_rs::agents::SetupFlags {
        explicit_agents: agent,
        all,
        yes,
        dry_run,
    };
    install_global(flags)?;
}
```

- [ ] **Step 3: Build**

Run: `cargo build`
Expected: compiles.

- [ ] **Step 4: Commit tasks 12 + 13 + 14 together**

```bash
git add src/main.rs
git commit -m "feat(cli): route init and install through agents orchestrator"
```

---

### Task 15: Rewrite the `Serve` branch's self-heal to use `self_heal_on_serve`

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Locate the current unconditional write block**

Inside `main()`'s `None | Some(Command::Serve)` branch, find the section that currently calls `write_mcp_config`, `write_claude_md_section`, `write_gemini_config`, `write_gemini_md_section`, `generate_agent_files`, and `ensure_tools_allowed` in sequence (approx lines 944-955).

- [ ] **Step 2: Replace with the orchestrator call**

```rust
let home = std::env::var("HOME").ok().map(PathBuf::from);
let repo_opt = if has_project { Some(repo_path.as_path()) } else { None };
if let Some(home) = &home {
    if let Err(e) = illu_rs::agents::self_heal_on_serve(repo_opt, home) {
        tracing::warn!("Agent self-heal failed: {e}");
    }
}
```

- [ ] **Step 3: Remove now-unused local helper imports** (`write_mcp_config`, `write_gemini_config`, etc. — they are about to be deleted in Task 16.)

- [ ] **Step 4: Build and test**

Run: `cargo build`
Expected: compiles.

Run: `cargo test`
Expected: all tests pass (nothing changes behavior yet since `AGENTS` is still empty).

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat(cli): route serve self-heal through agents orchestrator"
```

---

### Task 16: Delete now-dead functions in `main.rs`

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Delete the following private functions (all unused after tasks 13–15):**

- `write_mcp_config_to` (lines ~99-125)
- `write_mcp_server_config` (lines ~127-129)
- `write_mcp_config` (lines ~131-133)
- `write_gemini_config` (lines ~135-137)
- `write_global_mcp_config` (lines ~581-583)

- [ ] **Step 2: Build**

Run: `cargo build`
Expected: compiles with no warnings about unused functions.

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: passes.

- [ ] **Step 3: Commit**

```bash
git add src/main.rs
git commit -m "refactor(cli): delete superseded config writers"
```

---

## Phase 5 — Populate the agent registry

Each task below adds one or two rows to `AGENTS` in `src/agents/mod.rs`. Verification notes apply — do not commit any row until the vendor-specific identifiers are confirmed against current docs (search `https://docs.<vendor>.com` or equivalent). Write the verification comment inline in the code so future readers know where it came from.

### Task 17: Add Claude Code and Gemini CLI rows (no verification needed)

These match existing behavior exactly.

**Files:**
- Modify: `src/agents/mod.rs`

- [ ] **Step 1: Replace `pub static AGENTS: &[Agent] = &[];` with:**

```rust
pub static AGENTS: &[Agent] = &[
    Agent {
        id: "claude-code",
        display_name: "Claude Code",
        detection: Detection {
            env_vars: &["CLAUDECODE"],
            binaries: &["claude"],
            config_dirs: &[".claude"],
            app_bundles: &[],
        },
        repo_config: Some(RepoConfig {
            mcp_config_path: ".mcp.json",
            mcp_format: McpFormat::ClaudeCodeJson,
            instruction_file: Some("CLAUDE.md"),
            agents_dir: Some(".claude/agents"),
            allow_list_path: Some(".claude/settings.local.json"),
        }),
        global_config: Some(GlobalConfig {
            mcp_config_path: GlobalPath::Home(".claude/settings.json"),
            mcp_format: McpFormat::ClaudeCodeJson,
            instruction_file: Some(GlobalPath::Home(".claude/CLAUDE.md")),
            agents_dir: Some(GlobalPath::Home(".claude/agents")),
            allow_list_path: Some(GlobalPath::Home(".claude/settings.json")),
        }),
        tool_prefix: "mcp__illu__",
    },
    Agent {
        id: "gemini-cli",
        display_name: "Gemini CLI",
        // Verify env var name against the official Gemini CLI docs before release.
        detection: Detection {
            env_vars: &[],
            binaries: &["gemini"],
            config_dirs: &[".gemini"],
            app_bundles: &[],
        },
        repo_config: Some(RepoConfig {
            mcp_config_path: ".gemini/settings.json",
            mcp_format: McpFormat::GeminiJson,
            instruction_file: Some("GEMINI.md"),
            agents_dir: Some(".gemini/agents"),
            allow_list_path: None,
        }),
        global_config: Some(GlobalConfig {
            mcp_config_path: GlobalPath::Home(".gemini/settings.json"),
            mcp_format: McpFormat::GeminiJson,
            instruction_file: Some(GlobalPath::Home(".gemini/GEMINI.md")),
            agents_dir: Some(GlobalPath::Home(".gemini/agents")),
            allow_list_path: None,
        }),
        tool_prefix: "mcp_illu_",
    },
];
```

- [ ] **Step 2: Build and run existing tests**

Run: `cargo test`
Expected: all tests pass. No behavior change for users who previously ran `illu init` without flags (they will now see the prompt but can still accept defaults).

- [ ] **Step 3: Manual smoke test**

```bash
cargo run -- -r /tmp/new-repo init --yes --agent claude-code --dry-run
```
Expected: output lists `configured claude-code` and the four paths it would touch.

- [ ] **Step 4: Commit**

```bash
git add src/agents/mod.rs
git commit -m "feat(agents): register Claude Code and Gemini CLI in registry"
```

---

### Task 18: Add Codex CLI row

**Verification required before commit:**
- Confirm binary name (`codex`), config path (`~/.codex/config.toml`), and that the MCP section key is `[mcp_servers.<id>]` against current Codex CLI docs.

**Files:**
- Modify: `src/agents/mod.rs`

- [ ] **Step 1: Append a row to `AGENTS`** (before the trailing `]`):

```rust
Agent {
    id: "codex-cli",
    display_name: "Codex CLI",
    detection: Detection {
        env_vars: &[],
        binaries: &["codex"],
        config_dirs: &[".codex"],
        app_bundles: &[],
    },
    repo_config: None,
    global_config: Some(GlobalConfig {
        mcp_config_path: GlobalPath::Home(".codex/config.toml"),
        mcp_format: McpFormat::CodexToml,
        instruction_file: None,
        agents_dir: None,
        allow_list_path: None,
    }),
    tool_prefix: "mcp__illu__",
},
```

- [ ] **Step 2: Run tests and a dry-run smoke test**

```bash
cargo test
cargo run -- install --yes --agent codex-cli --dry-run
```
Expected: reports `configured codex-cli` with path `~/.codex/config.toml`.

- [ ] **Step 3: Verify identifiers against Codex CLI docs**

Open the current Codex CLI reference page and confirm the config path and section name. If they differ, update the row inline.

- [ ] **Step 4: Commit**

```bash
git add src/agents/mod.rs
git commit -m "feat(agents): register Codex CLI in registry"
```

---

### Task 19: Add Claude Desktop row

**Verification required before commit:**
- macOS path: `~/Library/Application Support/Claude/claude_desktop_config.json` (known).
- Windows path: `%APPDATA%\Claude\claude_desktop_config.json` (known).
- Linux path: verify against vendor docs (likely `~/.config/Claude/claude_desktop_config.json`).
- App bundle on macOS: `/Applications/Claude.app` (known).

**Files:**
- Modify: `src/agents/mod.rs`

- [ ] **Step 1: Append to `AGENTS`:**

```rust
Agent {
    id: "claude-desktop",
    display_name: "Claude Desktop",
    detection: Detection {
        env_vars: &[],
        binaries: &[],
        config_dirs: &["Library/Application Support/Claude"],
        app_bundles: &["/Applications/Claude.app"],
    },
    repo_config: None,
    global_config: Some(GlobalConfig {
        mcp_config_path: GlobalPath::AppSupport("Claude", "claude_desktop_config.json"),
        mcp_format: McpFormat::ClaudeDesktopJson,
        instruction_file: None,
        agents_dir: None,
        allow_list_path: None,
    }),
    tool_prefix: "mcp__illu__",
},
```

- [ ] **Step 2: Smoke test**

```bash
cargo test
cargo run -- install --yes --agent claude-desktop --dry-run
```
Expected on macOS: path `~/Library/Application Support/Claude/claude_desktop_config.json`. On Linux: `~/.config/Claude/claude_desktop_config.json`.

- [ ] **Step 3: Commit**

```bash
git add src/agents/mod.rs
git commit -m "feat(agents): register Claude Desktop in registry"
```

---

### Task 20: Add Cursor row

**Verification required before commit:**
- Env var: confirm `CURSOR_TRACE_ID` (or similar) against current Cursor docs; leave env_vars empty if unsure.
- Config paths: `.cursor/mcp.json` (repo) and `~/.cursor/mcp.json` (global).

**Files:**
- Modify: `src/agents/mod.rs`

- [ ] **Step 1: Append to `AGENTS`:**

```rust
Agent {
    id: "cursor",
    display_name: "Cursor",
    detection: Detection {
        env_vars: &[],  // fill in verified env var if any
        binaries: &["cursor"],
        config_dirs: &[".cursor"],
        app_bundles: &["/Applications/Cursor.app"],
    },
    repo_config: Some(RepoConfig {
        mcp_config_path: ".cursor/mcp.json",
        mcp_format: McpFormat::CursorJson,
        instruction_file: None,
        agents_dir: None,
        allow_list_path: None,
    }),
    global_config: Some(GlobalConfig {
        mcp_config_path: GlobalPath::Home(".cursor/mcp.json"),
        mcp_format: McpFormat::CursorJson,
        instruction_file: None,
        agents_dir: None,
        allow_list_path: None,
    }),
    tool_prefix: "mcp__illu__",
},
```

- [ ] **Step 2: Smoke test + commit** (same pattern as Task 18/19):

```bash
cargo test
cargo run -- -r /tmp/new-repo init --yes --agent cursor --dry-run
git add src/agents/mod.rs
git commit -m "feat(agents): register Cursor in registry"
```

---

### Task 21: Add VS Code + Copilot row

**Verification required before commit:**
- Schema: confirm `{ "servers": { "illu": { "type": "stdio", "command": ..., "args": [...] } } }` against current VS Code MCP docs.
- Config path: `.vscode/mcp.json` (repo-level). VS Code also supports user settings but we only target the repo-level file.
- Env vars: `VSCODE_PID`, `TERM_PROGRAM=vscode`. Prefer `VSCODE_PID` for reliability.

**Files:**
- Modify: `src/agents/mod.rs`

- [ ] **Step 1: Append to `AGENTS`:**

```rust
Agent {
    id: "vscode-copilot",
    display_name: "VS Code + Copilot",
    detection: Detection {
        env_vars: &["VSCODE_PID"],
        binaries: &["code"],
        config_dirs: &[],
        app_bundles: &["/Applications/Visual Studio Code.app"],
    },
    repo_config: Some(RepoConfig {
        mcp_config_path: ".vscode/mcp.json",
        mcp_format: McpFormat::VsCodeJson,
        instruction_file: None,
        agents_dir: None,
        allow_list_path: None,
    }),
    global_config: None,
    tool_prefix: "mcp__illu__",
},
```

- [ ] **Step 2: Smoke test + commit**

```bash
cargo test
cargo run -- -r /tmp/new-repo init --yes --agent vscode-copilot --dry-run
git add src/agents/mod.rs
git commit -m "feat(agents): register VS Code + Copilot in registry"
```

---

### Task 22: Add Codex Desktop row

**Verification required before commit:**
- Whether Codex Desktop reuses `~/.codex/config.toml` or has its own config (e.g. `~/Library/Application Support/Codex/…`). Check vendor docs. If it reuses Codex CLI's config, this row has the same `global_config` as Codex CLI and only the detection differs (app bundle).
- App bundle name on macOS: confirm.

**Files:**
- Modify: `src/agents/mod.rs`

- [ ] **Step 1: Append to `AGENTS`:**

```rust
Agent {
    id: "codex-desktop",
    display_name: "Codex Desktop",
    detection: Detection {
        env_vars: &[],
        binaries: &[],
        config_dirs: &[".codex"],
        app_bundles: &["/Applications/Codex.app", "/Applications/ChatGPT.app"],
    },
    repo_config: None,
    global_config: Some(GlobalConfig {
        // If Codex Desktop uses a separate location, update both fields.
        mcp_config_path: GlobalPath::Home(".codex/config.toml"),
        mcp_format: McpFormat::CodexToml,
        instruction_file: None,
        agents_dir: None,
        allow_list_path: None,
    }),
    tool_prefix: "mcp__illu__",
},
```

- [ ] **Step 2: Smoke test + commit**

```bash
cargo test
cargo run -- install --yes --agent codex-desktop --dry-run
git add src/agents/mod.rs
git commit -m "feat(agents): register Codex Desktop in registry"
```

---

### Task 23: Add Antigravity row

**Verification required before commit:**
- Binary name (`antigravity`?), config path (`~/.antigravity/…`?), schema (assumed `mcpServers`-style), app-bundle name (`Antigravity.app`?). All marked "verify" in the spec.
- If any identifier can't be confirmed, leave the field empty rather than guessing. An agent with no detection heuristics is still usable via `--agent antigravity`.

**Files:**
- Modify: `src/agents/mod.rs`

- [ ] **Step 1: Append to `AGENTS`:**

```rust
Agent {
    id: "antigravity",
    display_name: "Antigravity",
    detection: Detection {
        env_vars: &[],
        binaries: &["antigravity"],
        config_dirs: &[".antigravity"],
        app_bundles: &["/Applications/Antigravity.app"],
    },
    repo_config: None,
    global_config: Some(GlobalConfig {
        mcp_config_path: GlobalPath::Home(".antigravity/mcp.json"),
        mcp_format: McpFormat::AntigravityJson,
        instruction_file: None,
        agents_dir: None,
        allow_list_path: None,
    }),
    tool_prefix: "mcp__illu__",
},
```

- [ ] **Step 2: Smoke test + commit**

```bash
cargo test
cargo run -- install --yes --agent antigravity --dry-run
git add src/agents/mod.rs
git commit -m "feat(agents): register Antigravity in registry"
```

---

## Phase 6 — Integration tests

### Task 24: End-to-end test for `init`

**Files:**
- Create: `tests/agents_init.rs`

- [ ] **Step 1: Create `tests/agents_init.rs`**

```rust
use illu_rs::agents::{AGENTS, SetupFlags, configure_repo};
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
    assert!(dir.path().join(".mcp.json").exists());
    assert!(dir.path().join(".cursor/mcp.json").exists());
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
}
```

- [ ] **Step 2: Run the test file**

Run: `cargo test --test agents_init`
Expected: 6 tests pass.

- [ ] **Step 3: Commit**

```bash
git add tests/agents_init.rs
git commit -m "test(agents): add end-to-end tests for configure_repo"
```

---

### Task 25: End-to-end test for `install`

**Files:**
- Create: `tests/agents_install.rs`

- [ ] **Step 1: Create `tests/agents_install.rs`**

```rust
use illu_rs::agents::{SetupFlags, configure_global};
use std::fs;
use tempfile::tempdir;

#[test]
fn install_codex_cli_writes_toml_under_home() {
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
fn install_claude_code_writes_global_settings() {
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
}
```

- [ ] **Step 2: Run**

Run: `cargo test --test agents_install`
Expected: 2 tests pass.

> Note: `std::env::set_var` inside tests is unsafe in Rust 2024 and shares the process environment. If test parallelism causes flakiness, serialize these two tests with a `static MUTEX: Mutex<()> = Mutex::new(());` guard.

- [ ] **Step 3: Commit**

```bash
git add tests/agents_install.rs
git commit -m "test(agents): add end-to-end tests for configure_global"
```

---

## Phase 7 — Final checks

### Task 26: Clippy, fmt, and full test suite

- [ ] **Step 1: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: clean.

- [ ] **Step 2: Run fmt check**

Run: `cargo fmt --all -- --check`
Expected: clean.

- [ ] **Step 3: Run full test suite**

Run: `cargo test`
Expected: all unit + integration tests pass.

- [ ] **Step 4: Run a real `illu init --yes --dry-run` against the current repo**

```bash
cargo run -- init --yes --dry-run
```
Expected: prints what would be configured for whichever agents are detected on the developer's machine; does not write any files.

- [ ] **Step 5: If anything fails, fix inline, then commit**

```bash
git add -A
git commit -m "chore(agents): satisfy clippy and fmt"
```

---

### Task 27: Update `Command` doc strings to reflect the new behavior

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Update the doc comments on `Init` and `Install`**

Find the `/// Set up illu ...` and `/// Install illu globally ...` lines and replace:

```rust
/// Set up illu in a repo (detects or prompts for per-repo agents, builds index)
Init { ... }
/// Install illu globally (detects or prompts for desktop/IDE agents system-wide)
Install { ... }
```

Also update the top-level binary description in the `#[command(about = ...)]` attribute if it still references "Claude Code + Gemini CLI" (check and remove the hard-coded agent list).

- [ ] **Step 2: Build**

Run: `cargo build`
Expected: compiles.

- [ ] **Step 3: Commit**

```bash
git add src/main.rs
git commit -m "docs(cli): update init/install help text for multi-agent setup"
```

---

## Self-review

**Spec coverage:**
- Eight supported agents — Tasks 17-23 cover all of them.
- Detection with Active/Installed/Unknown levels — Task 6.
- Multi-select prompt with `dialoguer`, no-TTY fallback — Task 10 + Task 9 (selection logic).
- CLI flags `--agent`, `--all`, `--yes`, `--dry-run` — Task 12.
- `init` per-repo / `install` global split — Tasks 13/14 + `repo_config`/`global_config` filter in Task 11.
- `serve` env-var-only self-heal — Task 15.
- Data-driven `Agent` registry — Task 1 + Task 11 wiring + Phase 5 rows.
- Seven `McpFormat` writers, read-modify-write semantics, unrelated-entry preservation — Task 8.
- Platform-aware `GlobalPath` resolver — Task 7.
- Migration/removal of old functions — Tasks 2-4 (moves) + Task 16 (deletes).
- No backwards-compat shim — confirmed: Task 16 deletes the old writers outright.
- Testing coverage: unit tests in Tasks 6, 7, 8, 9; integration tests in Tasks 24-25.
- Lint gates respected throughout (no unwrap, no print_stdout outside `#[expect]`, etc.).

**Placeholder scan:** every step contains real code or an exact command. The registry rows in Tasks 20-23 that depend on vendor identifier confirmation list the specific fields to verify and a deterministic fallback for each.

**Type consistency check:**
- `IlluCommand { command: String, args: Vec<String> }` is the same across Tasks 1, 8, and 11.
- `SetupFlags { explicit_agents, all, yes, dry_run }` is used identically in Tasks 1, 9, 12, 13, 14.
- `AgentWriteReport { agent_id, written_paths, skipped }` is returned by `configure_repo`/`configure_global` in Task 11 and consumed in Tasks 13/14 in a matching shape.
- `GlobalPath` variants used in registry rows (Phase 5) all exist in Task 1.
- `McpFormat` variants used in registry rows all exist in Task 1 and have writer branches in Task 8.
- `DetectionLevel` is referenced by `detect_level`, `detect_scoped`, `detection_reason`, `select_from_flags` — all match Task 1's definition.

**Scope check:** single feature (detect-or-prompt multi-agent setup), single spec, no unrelated refactoring. Scoped appropriately for one plan.
