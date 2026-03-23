# Global Multi-Repo + Worktree Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make illu globally available across all Rust repos without per-repo setup, support git worktrees, and enable cross-repo code intelligence.

**Architecture:** Global install writes MCP config to user-level settings (Claude + Gemini). Server auto-detects repo from CWD via `git rev-parse --show-toplevel`. A registry at `~/.illu/registry.toml` tracks all known repos. Five new cross-repo tools open other repos' indexes read-only on demand.

**Tech Stack:** rusqlite (existing), toml (existing), clap (existing), std::process::Command for git, std::time for timestamps.

**Design doc:** `docs/plans/2026-03-21-global-multi-repo-design.md`

---

### Task 1: Add `detect_repo()` Function

**Files:**
- Create: `src/git.rs`
- Modify: `src/lib.rs`
- Modify: `src/main.rs`

**Step 1: Create `src/git.rs` with repo detection**

```rust
use std::path::{Path, PathBuf};
use std::process::Command;

/// Detect the repo root from a directory using `git rev-parse --show-toplevel`.
/// Works for both regular repos and worktrees.
pub fn detect_repo_root(from: &Path) -> Result<PathBuf, String> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(from)
        .output()
        .map_err(|e| format!("Failed to run git: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Not a git repository: {stderr}"));
    }

    let path = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_string();
    Ok(PathBuf::from(path))
}

/// Get the shared git directory (for worktree dedup).
/// Returns the `.git` dir for regular repos, or the common dir for worktrees.
pub fn git_common_dir(from: &Path) -> Result<PathBuf, String> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .current_dir(from)
        .output()
        .map_err(|e| format!("Failed to run git: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Not a git repository: {stderr}"));
    }

    let path = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_string();
    let abs = if Path::new(&path).is_relative() {
        from.join(&path).canonicalize().map_err(|e| e.to_string())?
    } else {
        PathBuf::from(&path)
    };
    Ok(abs)
}

/// Get the primary remote URL (for registry metadata).
pub fn git_remote_url(from: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(from)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let url = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_string();
    if url.is_empty() { None } else { Some(url) }
}
```

**Step 2: Register module in `src/lib.rs`**

Add `pub mod git;` to `src/lib.rs`.

**Step 3: Update `main.rs` to use auto-detection**

In the `Cli` struct, change `repo` default handling. After `Cli::parse()`, resolve repo:

```rust
let repo_path = if cli.repo == Path::new(".") {
    // No explicit --repo, auto-detect
    match illu_rs::git::detect_repo_root(&std::env::current_dir()?) {
        Ok(root) => root,
        Err(_) => std::env::current_dir()?,
    }
} else {
    cli.repo.clone()
};
```

**Step 4: Run tests**

Run: `cargo test --lib`
Run: `cargo clippy --all-targets --all-features -- -D warnings`

**Step 5: Commit**

```
feat: add git repo auto-detection from CWD
```

---

### Task 2: Add `illu install` Command

**Files:**
- Modify: `src/main.rs`

**Step 1: Add `Install` variant to the `Command` enum**

```rust
/// Install illu globally (configures Claude Code + Gemini CLI for all repos)
Install,
```

**Step 2: Add `write_global_mcp_config` function**

Reuse `write_mcp_server_config` pattern but with no `--repo` arg:

```rust
fn write_global_mcp_config(config_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let binary = std::env::current_exe()?
        .canonicalize()?
        .to_string_lossy()
        .into_owned();

    let illu_entry = serde_json::json!({
        "command": binary,
        "args": ["serve"],
        "env": { "RUST_LOG": "warn" }
    });

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut config: serde_json::Value = std::fs::read_to_string(config_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({"mcpServers": {}}));

    config["mcpServers"]["illu"] = illu_entry;

    std::fs::write(config_path, serde_json::to_string_pretty(&config)?)?;
    Ok(())
}
```

**Step 3: Add `install_global` function**

```rust
fn install_global() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::var("HOME")
        .map_err(|_| "HOME environment variable not set")?;
    let home = PathBuf::from(home);

    // MCP configs
    let claude_settings = home.join(".claude/settings.json");
    write_global_mcp_config(&claude_settings)?;
    println!("  wrote {}", claude_settings.display());

    let gemini_settings = home.join(".gemini/settings.json");
    write_global_mcp_config(&gemini_settings)?;
    println!("  wrote {}", gemini_settings.display());

    // Instruction files
    let claude_md = home.join(".claude/CLAUDE.md");
    write_md_section(&home, ".claude/CLAUDE.md", "# CLAUDE.md",
        &illu_agent_section("illu", "mcp__illu__"))?;
    println!("  updated {}", claude_md.display());

    let gemini_md = home.join(".gemini/GEMINI.md");
    write_md_section(&home, ".gemini/GEMINI.md", "# GEMINI.md",
        &illu_agent_section("@illu", "mcp_illu_"))?;
    println!("  updated {}", gemini_md.display());

    // Global gitignore for .illu/
    ensure_global_gitignore(&home)?;

    println!("\nDone. illu will auto-start in any Rust repo.");
    Ok(())
}
```

**Step 4: Add `ensure_global_gitignore` function**

```rust
fn ensure_global_gitignore(home: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let gitignore_path = home.join(".config/git/ignore");
    if let Some(parent) = gitignore_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = std::fs::read_to_string(&gitignore_path).unwrap_or_default();
    if content.lines().any(|l| l.trim() == ".illu/" || l.trim() == ".illu") {
        return Ok(());
    }
    let mut new_content = content;
    if !new_content.is_empty() && !new_content.ends_with('\n') {
        new_content.push('\n');
    }
    new_content.push_str(".illu/\n");
    std::fs::write(&gitignore_path, new_content)?;
    println!("  added .illu/ to global gitignore");
    Ok(())
}
```

**Step 5: Wire up in `main()` match**

```rust
Some(Command::Install) => {
    install_global()?;
}
```

**Step 6: Fix `write_md_section` to accept arbitrary base paths**

Currently `write_md_section` takes `repo_path` and joins `file_name`. This already works for global paths — just pass `home` as the base and `.claude/CLAUDE.md` as the file_name. Verify no assumptions about repo structure in that function.

**Step 7: Run tests and clippy**

Run: `cargo test --lib`
Run: `cargo clippy --all-targets --all-features -- -D warnings`

**Step 8: Commit**

```
feat: add `illu install` for global Claude + Gemini setup
```

---

### Task 3: Non-Rust Repo Handling

**Files:**
- Modify: `src/main.rs` (serve command)

**Step 1: Handle missing Cargo.toml gracefully in serve**

In the `Serve` branch of `main()`, after resolving `repo_path`, check for Cargo.toml. If missing, use in-memory DB and skip indexing:

```rust
let has_cargo = repo_path.join("Cargo.toml").exists();

let (db, config) = if has_cargo {
    // ... existing indexing code ...
    (db, config)
} else {
    tracing::warn!("No Cargo.toml found — cross-repo tools only");
    let db = Database::open_in_memory()?;
    let config = IndexConfig { repo_path: repo_path.clone() };
    (db, config)
};
```

**Step 2: Run tests**

Run: `cargo test --lib`

**Step 3: Commit**

```
feat: graceful startup in non-Rust repos
```

---

### Task 4: Registry Module

**Files:**
- Create: `src/registry.rs`
- Modify: `src/lib.rs`

**Step 1: Write unit tests for Registry**

Add to `src/registry.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_new_repo() {
        let dir = tempfile::TempDir::new().unwrap();
        let registry_path = dir.path().join("registry.toml");
        let mut registry = Registry::load(&registry_path).unwrap();

        registry.register(RepoEntry {
            name: "test-repo".into(),
            path: PathBuf::from("/tmp/test-repo"),
            git_remote: Some("git@github.com:user/test.git".into()),
            git_common_dir: PathBuf::from("/tmp/test-repo/.git"),
            last_indexed: "2026-03-21T14:00:00Z".into(),
        });

        assert_eq!(registry.repos.len(), 1);
        assert_eq!(registry.repos[0].name, "test-repo");
        registry.save().unwrap();

        // Reload and verify persistence
        let reloaded = Registry::load(&registry_path).unwrap();
        assert_eq!(reloaded.repos.len(), 1);
    }

    #[test]
    fn register_updates_existing() {
        let dir = tempfile::TempDir::new().unwrap();
        let registry_path = dir.path().join("registry.toml");
        let mut registry = Registry::load(&registry_path).unwrap();

        let entry = RepoEntry {
            name: "test-repo".into(),
            path: PathBuf::from("/tmp/test-repo"),
            git_remote: None,
            git_common_dir: PathBuf::from("/tmp/test-repo/.git"),
            last_indexed: "2026-03-21T14:00:00Z".into(),
        };
        registry.register(entry.clone());
        registry.register(RepoEntry {
            last_indexed: "2026-03-21T15:00:00Z".into(),
            ..entry
        });

        assert_eq!(registry.repos.len(), 1);
        assert_eq!(registry.repos[0].last_indexed, "2026-03-21T15:00:00Z");
    }

    #[test]
    fn worktree_dedup_by_common_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let registry_path = dir.path().join("registry.toml");
        let mut registry = Registry::load(&registry_path).unwrap();

        // Main repo
        registry.register(RepoEntry {
            name: "my-repo".into(),
            path: PathBuf::from("/tmp/my-repo"),
            git_remote: None,
            git_common_dir: PathBuf::from("/tmp/my-repo/.git"),
            last_indexed: "2026-03-21T14:00:00Z".into(),
        });

        // Worktree of same repo — same git_common_dir
        registry.register(RepoEntry {
            name: "my-repo".into(),
            path: PathBuf::from("/tmp/my-repo-wt-feature"),
            git_remote: None,
            git_common_dir: PathBuf::from("/tmp/my-repo/.git"),
            last_indexed: "2026-03-21T15:00:00Z".into(),
        });

        // Should NOT create a duplicate — updates existing
        assert_eq!(registry.repos.len(), 1);
        // Keeps the main path, updates timestamp
        assert_eq!(registry.repos[0].path, PathBuf::from("/tmp/my-repo"));
        assert_eq!(registry.repos[0].last_indexed, "2026-03-21T15:00:00Z");
    }

    #[test]
    fn prune_removes_dead_paths() {
        let dir = tempfile::TempDir::new().unwrap();
        let registry_path = dir.path().join("registry.toml");
        let mut registry = Registry::load(&registry_path).unwrap();

        // Existing dir
        registry.register(RepoEntry {
            name: "exists".into(),
            path: dir.path().to_path_buf(),
            git_remote: None,
            git_common_dir: dir.path().join(".git"),
            last_indexed: "2026-03-21T14:00:00Z".into(),
        });
        // Dead path
        registry.register(RepoEntry {
            name: "dead".into(),
            path: PathBuf::from("/nonexistent/path"),
            git_remote: None,
            git_common_dir: PathBuf::from("/nonexistent/path/.git"),
            last_indexed: "2026-03-21T14:00:00Z".into(),
        });

        assert_eq!(registry.repos.len(), 2);
        registry.prune();
        assert_eq!(registry.repos.len(), 1);
        assert_eq!(registry.repos[0].name, "exists");
    }

    #[test]
    fn other_repos_excludes_primary() {
        let dir = tempfile::TempDir::new().unwrap();
        let registry_path = dir.path().join("registry.toml");
        let mut registry = Registry::load(&registry_path).unwrap();

        let primary = PathBuf::from("/tmp/repo-a");
        registry.register(RepoEntry {
            name: "repo-a".into(),
            path: primary.clone(),
            git_remote: None,
            git_common_dir: PathBuf::from("/tmp/repo-a/.git"),
            last_indexed: "now".into(),
        });
        registry.register(RepoEntry {
            name: "repo-b".into(),
            path: PathBuf::from("/tmp/repo-b"),
            git_remote: None,
            git_common_dir: PathBuf::from("/tmp/repo-b/.git"),
            last_indexed: "now".into(),
        });

        let others = registry.other_repos(&primary);
        assert_eq!(others.len(), 1);
        assert_eq!(others[0].name, "repo-b");
    }
}
```

**Step 2: Implement Registry struct**

```rust
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepoEntry {
    pub name: String,
    pub path: PathBuf,
    pub git_remote: Option<String>,
    pub git_common_dir: PathBuf,
    pub last_indexed: String,
}

#[derive(Debug)]
pub struct Registry {
    file_path: PathBuf,
    pub repos: Vec<RepoEntry>,
}

#[derive(Deserialize, Serialize, Default)]
struct RegistryFile {
    #[serde(default)]
    repos: Vec<RepoEntry>,
}

impl Registry {
    /// Load registry from file, or create empty if file doesn't exist.
    pub fn load(path: &Path) -> Result<Self, String> {
        let repos = if path.exists() {
            let content = std::fs::read_to_string(path)
                .map_err(|e| format!("Failed to read registry: {e}"))?;
            let file: RegistryFile = toml::from_str(&content)
                .map_err(|e| format!("Failed to parse registry: {e}"))?;
            file.repos
        } else {
            Vec::new()
        };
        Ok(Self {
            file_path: path.to_path_buf(),
            repos,
        })
    }

    /// Register or update a repo entry. Deduplicates by `git_common_dir`.
    pub fn register(&mut self, entry: RepoEntry) {
        if let Some(existing) = self.repos.iter_mut()
            .find(|r| r.git_common_dir == entry.git_common_dir)
        {
            // Update timestamp and remote, keep the original path (main repo)
            existing.last_indexed = entry.last_indexed;
            if entry.git_remote.is_some() {
                existing.git_remote = entry.git_remote;
            }
        } else {
            self.repos.push(entry);
        }
    }

    /// Remove entries whose paths no longer exist on disk.
    pub fn prune(&mut self) {
        self.repos.retain(|r| r.path.exists());
    }

    /// Save registry to disk.
    pub fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.file_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create registry dir: {e}"))?;
        }
        let file = RegistryFile {
            repos: self.repos.clone(),
        };
        let content = toml::to_string_pretty(&file)
            .map_err(|e| format!("Failed to serialize registry: {e}"))?;
        std::fs::write(&self.file_path, content)
            .map_err(|e| format!("Failed to write registry: {e}"))?;
        Ok(())
    }

    /// Get all repos except the one at `primary_path`.
    pub fn other_repos(&self, primary_path: &Path) -> Vec<&RepoEntry> {
        self.repos.iter().filter(|r| r.path != primary_path).collect()
    }

    /// Default registry path: `~/.illu/registry.toml`.
    pub fn default_path() -> Result<PathBuf, String> {
        let home = std::env::var("HOME")
            .map_err(|_| "HOME environment variable not set".to_string())?;
        Ok(PathBuf::from(home).join(".illu/registry.toml"))
    }
}
```

**Step 3: Register module in `src/lib.rs`**

Add `pub mod registry;` to `src/lib.rs`.

**Step 4: Run tests**

Run: `cargo test --lib -- registry::tests`
Expected: All 5 tests pass.

**Step 5: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`

**Step 6: Commit**

```
feat: add registry module for tracking repos globally
```

---

### Task 5: Auto-Register on Serve + Wire Registry into Server

**Files:**
- Modify: `src/main.rs`
- Modify: `src/server/mod.rs`

**Step 1: Add `register_repo` helper to `main.rs`**

```rust
fn register_repo(repo_path: &Path) {
    let Ok(registry_path) = illu_rs::registry::Registry::default_path() else {
        return;
    };
    let Ok(mut registry) = illu_rs::registry::Registry::load(&registry_path) else {
        return;
    };

    let name = repo_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".into());

    let git_remote = illu_rs::git::git_remote_url(repo_path);
    let git_common_dir = illu_rs::git::git_common_dir(repo_path)
        .unwrap_or_else(|_| repo_path.join(".git"));

    let now = {
        use std::time::SystemTime;
        let secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        format!("{secs}")
    };

    registry.register(illu_rs::registry::RepoEntry {
        name,
        path: repo_path.to_path_buf(),
        git_remote,
        git_common_dir,
        last_indexed: now,
    });
    registry.prune();
    let _ = registry.save();
}
```

**Step 2: Call `register_repo` in the `Serve` branch after indexing**

Insert after the refresh/index block, before creating `IlluServer`:

```rust
register_repo(&repo_path);
```

**Step 3: Add `registry` field to `IlluServer`**

In `src/server/mod.rs`, update the struct:

```rust
pub struct IlluServer {
    db: std::sync::Arc<Mutex<Database>>,
    config: std::sync::Arc<IndexConfig>,
    registry: std::sync::Arc<crate::registry::Registry>,
    tool_router: ToolRouter<Self>,
}
```

Update `new()`:

```rust
pub fn new(db: Database, config: IndexConfig, registry: crate::registry::Registry) -> Self {
    Self {
        db: std::sync::Arc::new(Mutex::new(db)),
        config: std::sync::Arc::new(config),
        registry: std::sync::Arc::new(registry),
        tool_router: Self::tool_router(),
    }
}
```

**Step 4: Update `main.rs` to load and pass registry**

Before creating `IlluServer`:

```rust
let registry = {
    let path = illu_rs::registry::Registry::default_path()
        .unwrap_or_else(|_| repo_path.join(".illu/registry.toml"));
    illu_rs::registry::Registry::load(&path).unwrap_or_else(|_| {
        illu_rs::registry::Registry::load(&repo_path.join(".illu/registry.toml"))
            .unwrap_or_else(|_| illu_rs::registry::Registry {
                repos: Vec::new(),
                // empty registry as fallback
            })
    })
};

let server = IlluServer::new(db, config.clone(), registry);
```

Note: `Registry` needs a public constructor for the fallback case. Add to registry.rs:

```rust
pub fn empty() -> Self {
    Self {
        file_path: PathBuf::new(),
        repos: Vec::new(),
    }
}
```

**Step 5: Run tests and clippy**

Run: `cargo test`
Run: `cargo clippy --all-targets --all-features -- -D warnings`

**Step 6: Commit**

```
feat: auto-register repos and wire registry into server
```

---

### Task 6: Add `Database::open_readonly`

**Files:**
- Modify: `src/db.rs`

**Step 1: Write test**

Add to the existing test module in `src/db.rs`:

```rust
#[test]
fn test_open_readonly() {
    let dir = tempfile::TempDir::new().unwrap();
    let illu_dir = dir.path().join(".illu");
    std::fs::create_dir_all(&illu_dir).unwrap();
    let db_path = illu_dir.join("index.db");

    // Create a DB with data
    {
        let db = Database::open(&db_path).unwrap();
        db.conn.execute(
            "INSERT INTO files (path, content_hash) VALUES (?1, ?2)",
            ["src/lib.rs", "abc123"],
        ).unwrap();
    }

    // Open readonly and verify reads work
    let db = Database::open_readonly(&db_path).unwrap();
    let count: i64 = db.conn.query_row(
        "SELECT COUNT(*) FROM files", [], |r| r.get(0)
    ).unwrap();
    assert_eq!(count, 1);

    // Writes should fail
    let result = db.conn.execute(
        "INSERT INTO files (path, content_hash) VALUES (?1, ?2)",
        ["src/main.rs", "def456"],
    );
    assert!(result.is_err());
}
```

**Step 2: Implement `open_readonly`**

```rust
pub fn open_readonly(path: &std::path::Path) -> SqlResult<Self> {
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let repo_root = path
        .parent()
        .filter(|p| p.file_name().is_some_and(|n| n == ".illu"))
        .and_then(|p| p.parent())
        .map(std::path::Path::to_path_buf);
    Ok(Self { conn, repo_root })
}
```

No `migrate()` call — readonly DB should already be migrated. No PRAGMAs for write optimization.

**Step 3: Run tests**

Run: `cargo test --lib -- db::tests::test_open_readonly`

**Step 4: Commit**

```
feat: add Database::open_readonly for cross-repo queries
```

---

### Task 7: `repos` Tool

**Files:**
- Create: `src/server/tools/repos.rs`
- Modify: `src/server/tools/mod.rs`
- Modify: `src/server/mod.rs`

**Step 1: Create `src/server/tools/repos.rs`**

```rust
use crate::db::Database;
use crate::registry::Registry;
use std::path::Path;

pub fn handle_repos(
    registry: &Registry,
    primary_path: &Path,
) -> Result<String, Box<dyn std::error::Error>> {
    if registry.repos.is_empty() {
        return Ok("No repos registered. Start illu in a repo to auto-register it.".into());
    }

    let mut out = String::from("## Registered Repos\n\n");
    out.push_str("| Repo | Path | Status | Symbols |\n");
    out.push_str("|------|------|--------|---------|\n");

    for repo in &registry.repos {
        let is_primary = repo.path == primary_path;
        let db_path = repo.path.join(".illu/index.db");

        let (status, symbols) = if !repo.path.exists() {
            ("missing".to_string(), "—".to_string())
        } else if !db_path.exists() {
            ("no index".to_string(), "—".to_string())
        } else if let Ok(db) = Database::open_readonly(&db_path) {
            let count: i64 = db.conn
                .query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))
                .unwrap_or(0);
            let status = if is_primary {
                "active".to_string()
            } else {
                "indexed".to_string()
            };
            (status, count.to_string())
        } else {
            ("error".to_string(), "—".to_string())
        };

        let marker = if is_primary { " *" } else { "" };
        out.push_str(&format!(
            "| {}{marker} | {} | {status} | {symbols} |\n",
            repo.name,
            repo.path.display(),
        ));
    }

    out.push_str("\n\\* = active (current session)\n");
    Ok(out)
}
```

**Step 2: Register module in `src/server/tools/mod.rs`**

Add `pub mod repos;` alongside other tool modules.

**Step 3: Add tool to `src/server/mod.rs`**

Add params struct:

```rust
#[derive(Deserialize, JsonSchema)]
struct ReposParams {}
```

Add tool handler in the `#[tool_router] impl IlluServer` block:

```rust
#[tool(
    name = "repos",
    description = "List all registered repos with status, symbol counts, and which is the active session repo."
)]
async fn repos(
    &self,
    Parameters(_params): Parameters<ReposParams>,
) -> Result<CallToolResult, McpError> {
    tracing::info!("Tool call: repos");
    let _guard = crate::status::StatusGuard::new("repos");
    let result = tools::repos::handle_repos(
        &self.registry,
        &self.config.repo_path,
    )
    .map_err(to_mcp_err)?;
    Ok(text_result(result))
}
```

**Step 4: Run tests and clippy**

Run: `cargo test`
Run: `cargo clippy --all-targets --all-features -- -D warnings`

**Step 5: Commit**

```
feat: add `repos` tool for listing registered repos
```

---

### Task 8: `cross_query` Tool

**Files:**
- Create: `src/server/tools/cross_query.rs`
- Modify: `src/server/tools/mod.rs`
- Modify: `src/server/mod.rs`

**Step 1: Create `src/server/tools/cross_query.rs`**

```rust
use crate::db::Database;
use crate::registry::Registry;
use crate::server::tools::query::handle_query;
use std::path::Path;

pub fn handle_cross_query(
    registry: &Registry,
    primary_path: &Path,
    query: &str,
    scope: Option<&str>,
    kind: Option<&str>,
    attribute: Option<&str>,
    signature: Option<&str>,
    path: Option<&str>,
    limit: Option<i64>,
) -> Result<String, Box<dyn std::error::Error>> {
    let other_repos = registry.other_repos(primary_path);
    if other_repos.is_empty() {
        return Ok(
            "No other repos registered. Start illu in other repos to enable cross-repo queries."
                .into(),
        );
    }

    let mut out = String::from("## Cross-Repo Query Results\n\n");
    let mut found_any = false;

    for repo in &other_repos {
        let db_path = repo.path.join(".illu/index.db");
        let Ok(db) = Database::open_readonly(&db_path) else {
            continue;
        };

        let result = handle_query(&db, query, scope, kind, attribute, signature, path, limit);
        let Ok(result) = result else { continue };

        if !result.contains("No results") && !result.is_empty() {
            found_any = true;
            out.push_str(&format!("### {} ({})\n\n", repo.name, repo.path.display()));
            out.push_str(&result);
            out.push_str("\n\n");
        }
    }

    if !found_any {
        out.push_str("No matches found in other repos.\n");
    }

    Ok(out)
}
```

**Step 2: Register module and add tool handler**

In `src/server/tools/mod.rs`: add `pub mod cross_query;`

In `src/server/mod.rs`, add params:

```rust
#[derive(Deserialize, JsonSchema)]
struct CrossQueryParams {
    /// Search term
    query: String,
    /// Search scope: symbols (default), docs, files, all
    scope: Option<String>,
    kind: Option<String>,
    attribute: Option<String>,
    signature: Option<String>,
    path: Option<String>,
    limit: Option<i64>,
}
```

Add tool handler:

```rust
#[tool(
    name = "cross_query",
    description = "Search symbols across all registered repos (excluding the current one). Same parameters as `query`, results grouped by repo."
)]
async fn cross_query(
    &self,
    Parameters(params): Parameters<CrossQueryParams>,
) -> Result<CallToolResult, McpError> {
    tracing::info!(query = %params.query, "Tool call: cross_query");
    let _guard = crate::status::StatusGuard::new(&format!("cross_query ▸ {}", params.query));
    let result = tools::cross_query::handle_cross_query(
        &self.registry,
        &self.config.repo_path,
        &params.query,
        params.scope.as_deref(),
        params.kind.as_deref(),
        params.attribute.as_deref(),
        params.signature.as_deref(),
        params.path.as_deref(),
        params.limit,
    )
    .map_err(to_mcp_err)?;
    Ok(text_result(result))
}
```

**Step 3: Run tests and clippy**

Run: `cargo test`
Run: `cargo clippy --all-targets --all-features -- -D warnings`

**Step 4: Commit**

```
feat: add `cross_query` tool for multi-repo symbol search
```

---

### Task 9: `cross_impact` Tool

**Files:**
- Create: `src/server/tools/cross_impact.rs`
- Modify: `src/server/tools/mod.rs`
- Modify: `src/server/mod.rs`

**Step 1: Create `src/server/tools/cross_impact.rs`**

This tool searches other repos for symbols that reference the given symbol name. Since cross-repo refs are name-based, it queries `symbol_refs` joined with `symbols` for matching target names.

```rust
use crate::db::Database;
use crate::registry::Registry;
use std::path::Path;

pub fn handle_cross_impact(
    registry: &Registry,
    primary_path: &Path,
    symbol_name: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let other_repos = registry.other_repos(primary_path);
    if other_repos.is_empty() {
        return Ok("No other repos registered.".into());
    }

    // Split Type::method if present
    let (name, impl_type) = if let Some(idx) = symbol_name.find("::") {
        (&symbol_name[idx + 2..], Some(&symbol_name[..idx]))
    } else {
        (symbol_name, None)
    };

    let mut out = format!("## Cross-Repo Impact: `{symbol_name}`\n\n");
    let mut found_any = false;

    for repo in &other_repos {
        let db_path = repo.path.join(".illu/index.db");
        let Ok(db) = Database::open_readonly(&db_path) else {
            continue;
        };

        let query = if let Some(it) = impl_type {
            format!(
                "SELECT DISTINCT s.name, s.impl_type, f.path, s.line_start
                 FROM symbol_refs sr
                 JOIN symbols s ON sr.source_symbol_id = s.id
                 JOIN symbols t ON sr.target_symbol_id = t.id
                 JOIN files f ON s.file_id = f.id
                 WHERE t.name = ?1 AND t.impl_type = ?2
                 ORDER BY f.path, s.line_start
                 LIMIT 50"
            );
            db.conn.prepare(&query)
                .and_then(|mut stmt| {
                    stmt.query_map(rusqlite::params![name, it], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    })
                    .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
                })
        } else {
            let query = "SELECT DISTINCT s.name, s.impl_type, f.path, s.line_start
                 FROM symbol_refs sr
                 JOIN symbols s ON sr.source_symbol_id = s.id
                 JOIN symbols t ON sr.target_symbol_id = t.id
                 JOIN files f ON s.file_id = f.id
                 WHERE t.name = ?1
                 ORDER BY f.path, s.line_start
                 LIMIT 50";
            db.conn.prepare(query)
                .and_then(|mut stmt| {
                    stmt.query_map(rusqlite::params![name], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    })
                    .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
                })
        };

        let Ok(refs) = query else { continue };

        if !refs.is_empty() {
            found_any = true;
            out.push_str(&format!(
                "### {} ({}) — {} reference(s)\n\n",
                repo.name,
                repo.path.display(),
                refs.len()
            ));
            for (sname, simpl, file, line) in &refs {
                let qualified = match simpl {
                    Some(it) => format!("{it}::{sname}"),
                    None => sname.clone(),
                };
                out.push_str(&format!("- `{qualified}` ({file}:{line})\n"));
            }
            out.push('\n');
        }
    }

    if !found_any {
        out.push_str("No references to this symbol found in other repos.\n");
    }

    Ok(out)
}
```

**Step 2: Register module and add tool handler**

In `src/server/tools/mod.rs`: add `pub mod cross_impact;`

In `src/server/mod.rs`, add params:

```rust
#[derive(Deserialize, JsonSchema)]
struct CrossImpactParams {
    /// Symbol name to check cross-repo impact for (supports `Type::method` syntax)
    symbol_name: String,
}
```

Add tool handler:

```rust
#[tool(
    name = "cross_impact",
    description = "Find references to a symbol in other registered repos. Shows which code in other repos would be affected by changing this symbol."
)]
async fn cross_impact(
    &self,
    Parameters(params): Parameters<CrossImpactParams>,
) -> Result<CallToolResult, McpError> {
    tracing::info!(symbol = %params.symbol_name, "Tool call: cross_impact");
    let _guard = crate::status::StatusGuard::new(&format!("cross_impact ▸ {}", params.symbol_name));
    let result = tools::cross_impact::handle_cross_impact(
        &self.registry,
        &self.config.repo_path,
        &params.symbol_name,
    )
    .map_err(to_mcp_err)?;
    Ok(text_result(result))
}
```

**Step 3: Run tests and clippy**

Run: `cargo test`
Run: `cargo clippy --all-targets --all-features -- -D warnings`

**Step 4: Commit**

```
feat: add `cross_impact` tool for multi-repo change analysis
```

---

### Task 10: `cross_deps` Tool

**Files:**
- Create: `src/server/tools/cross_deps.rs`
- Modify: `src/server/tools/mod.rs`
- Modify: `src/server/mod.rs`

**Step 1: Create `src/server/tools/cross_deps.rs`**

Scans `Cargo.toml` of all registered repos for path deps pointing to other registered repos and shared crate dependencies.

```rust
use crate::registry::Registry;
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub fn handle_cross_deps(
    registry: &Registry,
) -> Result<String, Box<dyn std::error::Error>> {
    if registry.repos.len() < 2 {
        return Ok("Need at least 2 registered repos for cross-dep analysis.".into());
    }

    let mut out = String::from("## Cross-Repo Dependencies\n\n");

    // Collect deps per repo
    let mut repo_deps: HashMap<String, HashSet<String>> = HashMap::new();
    let mut path_deps: Vec<(String, String, String)> = Vec::new(); // (from_repo, dep_name, to_path)

    for repo in &registry.repos {
        let cargo_toml = repo.path.join("Cargo.toml");
        let Ok(content) = std::fs::read_to_string(&cargo_toml) else {
            continue;
        };
        let Ok(parsed) = content.parse::<toml::Table>() else {
            continue;
        };

        let mut deps = HashSet::new();

        for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
            let Some(table) = parsed.get(section).and_then(|v| v.as_table()) else {
                continue;
            };

            for (name, value) in table {
                deps.insert(name.clone());

                // Check for path deps
                let path_val = match value {
                    toml::Value::Table(t) => t.get("path").and_then(|v| v.as_str()),
                    _ => None,
                };

                if let Some(p) = path_val {
                    let abs = repo.path.join(p);
                    if let Ok(canonical) = abs.canonicalize() {
                        path_deps.push((
                            repo.name.clone(),
                            name.clone(),
                            canonical.to_string_lossy().into_owned(),
                        ));
                    }
                }
            }
        }

        repo_deps.insert(repo.name.clone(), deps);
    }

    // Path deps between registered repos
    let registered_paths: HashSet<String> = registry
        .repos
        .iter()
        .filter_map(|r| r.path.canonicalize().ok())
        .map(|p| p.to_string_lossy().into_owned())
        .collect();

    let cross_path_deps: Vec<_> = path_deps
        .iter()
        .filter(|(_, _, to)| registered_paths.contains(to))
        .collect();

    if !cross_path_deps.is_empty() {
        out.push_str("### Path Dependencies (direct source links)\n\n");
        for (from, name, to) in &cross_path_deps {
            let to_name = registry
                .repos
                .iter()
                .find(|r| r.path.canonicalize().ok()
                    .is_some_and(|p| p.to_string_lossy() == *to))
                .map(|r| r.name.as_str())
                .unwrap_or("?");
            out.push_str(&format!("- **{from}** → `{name}` → **{to_name}**\n"));
        }
        out.push('\n');
    }

    // Shared crate dependencies
    if repo_deps.len() >= 2 {
        let all_deps: HashSet<&String> = repo_deps.values().flatten().collect();
        let mut shared: Vec<(&String, Vec<&String>)> = Vec::new();

        for dep in &all_deps {
            let users: Vec<&String> = repo_deps
                .iter()
                .filter(|(_, deps)| deps.contains(*dep))
                .map(|(name, _)| name)
                .collect();
            if users.len() >= 2 {
                shared.push((dep, users));
            }
        }

        if !shared.is_empty() {
            shared.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
            out.push_str("### Shared Dependencies\n\n");
            out.push_str("| Crate | Used By |\n");
            out.push_str("|-------|---------|\n");
            for (dep, users) in shared.iter().take(30) {
                let names: Vec<&str> = users.iter().map(|s| s.as_str()).collect();
                out.push_str(&format!("| {} | {} |\n", dep, names.join(", ")));
            }
            out.push('\n');
        }
    }

    Ok(out)
}
```

**Step 2: Register module and add tool handler**

In `src/server/tools/mod.rs`: add `pub mod cross_deps;`

In `src/server/mod.rs`, add params:

```rust
#[derive(Deserialize, JsonSchema)]
struct CrossDepsParams {}
```

Add tool handler:

```rust
#[tool(
    name = "cross_deps",
    description = "Show how registered repos depend on each other: path dependencies (direct source links) and shared crate dependencies."
)]
async fn cross_deps(
    &self,
    Parameters(_params): Parameters<CrossDepsParams>,
) -> Result<CallToolResult, McpError> {
    tracing::info!("Tool call: cross_deps");
    let _guard = crate::status::StatusGuard::new("cross_deps");
    let result = tools::cross_deps::handle_cross_deps(&self.registry)
        .map_err(to_mcp_err)?;
    Ok(text_result(result))
}
```

**Step 3: Run tests and clippy**

Run: `cargo test`
Run: `cargo clippy --all-targets --all-features -- -D warnings`

**Step 4: Commit**

```
feat: add `cross_deps` tool for inter-repo dependency graph
```

---

### Task 11: `cross_callpath` Tool

**Files:**
- Create: `src/server/tools/cross_callpath.rs`
- Modify: `src/server/tools/mod.rs`
- Modify: `src/server/mod.rs`

**Step 1: Create `src/server/tools/cross_callpath.rs`**

Finds call chains that span repos. Strategy: find the symbol in the source repo's callees, check if any callee name exists in the target repo, then find paths within the target repo.

```rust
use crate::db::Database;
use crate::registry::Registry;
use crate::server::tools::callpath::handle_callpath;
use std::path::Path;

pub fn handle_cross_callpath(
    primary_db: &Database,
    registry: &Registry,
    primary_path: &Path,
    from: &str,
    to: &str,
    target_repo: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let other_repos = registry.other_repos(primary_path);
    if other_repos.is_empty() {
        return Ok("No other repos registered.".into());
    }

    let mut out = format!("## Cross-Repo Callpath: `{from}` → `{to}`\n\n");

    // First, try to find `from` in the primary DB
    let from_exists = primary_db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE name = ?1",
            [from],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;

    if !from_exists {
        return Ok(format!("`{from}` not found in the current repo."));
    }

    // Search for `to` in other repos
    let repos_to_search: Vec<_> = if let Some(name) = target_repo {
        other_repos
            .into_iter()
            .filter(|r| r.name == name)
            .collect()
    } else {
        other_repos
    };

    let mut found = false;
    for repo in &repos_to_search {
        let db_path = repo.path.join(".illu/index.db");
        let Ok(db) = Database::open_readonly(&db_path) else {
            continue;
        };

        // Check if `to` exists in this repo
        let to_exists = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM symbols WHERE name = ?1",
                [to],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;

        if !to_exists {
            continue;
        }

        found = true;
        out.push_str(&format!(
            "### Via {} ({})\n\n",
            repo.name,
            repo.path.display()
        ));

        // Find shared symbols: callees of `from` in primary that exist in target
        let shared = find_bridge_symbols(primary_db, &db, from)?;
        if shared.is_empty() {
            out.push_str(&format!(
                "- `{from}` (this repo) → ? → `{to}` ({}) — no shared symbols found\n\n",
                repo.name
            ));
        } else {
            for bridge in &shared {
                out.push_str(&format!(
                    "- `{from}` (this repo) → `{bridge}` → `{to}` ({})\n",
                    repo.name
                ));
            }
            out.push('\n');
        }
    }

    if !found {
        out.push_str(&format!(
            "`{to}` not found in any other registered repo.\n"
        ));
    }

    Ok(out)
}

fn find_bridge_symbols(
    source_db: &Database,
    target_db: &Database,
    from: &str,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    // Get callees of `from` in source
    let mut stmt = source_db.conn.prepare(
        "SELECT DISTINCT t.name
         FROM symbol_refs sr
         JOIN symbols s ON sr.source_symbol_id = s.id
         JOIN symbols t ON sr.target_symbol_id = t.id
         WHERE s.name = ?1 AND sr.kind = 'call'
         LIMIT 100",
    )?;
    let callees: Vec<String> = stmt
        .query_map([from], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    // Check which callees exist in target
    let mut bridges = Vec::new();
    for callee in &callees {
        let exists: i64 = target_db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM symbols WHERE name = ?1",
                [callee.as_str()],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if exists > 0 {
            bridges.push(callee.clone());
        }
    }

    Ok(bridges)
}
```

**Step 2: Register module and add tool handler**

In `src/server/tools/mod.rs`: add `pub mod cross_callpath;`

In `src/server/mod.rs`, add params:

```rust
#[derive(Deserialize, JsonSchema)]
struct CrossCallpathParams {
    /// Source symbol name (in the current repo)
    from: String,
    /// Target symbol name (in another repo)
    to: String,
    /// Specific target repo name (optional — searches all if omitted)
    target_repo: Option<String>,
}
```

Add tool handler:

```rust
#[tool(
    name = "cross_callpath",
    description = "Find call chains that span repo boundaries. Traces from a symbol in the current repo through shared dependencies to a symbol in another repo."
)]
async fn cross_callpath(
    &self,
    Parameters(params): Parameters<CrossCallpathParams>,
) -> Result<CallToolResult, McpError> {
    tracing::info!(from = %params.from, to = %params.to, "Tool call: cross_callpath");
    let _guard = crate::status::StatusGuard::new(&format!(
        "cross_callpath ▸ {} → {}",
        params.from, params.to
    ));
    self.refresh()?;
    let db = self.lock_db()?;
    let result = tools::cross_callpath::handle_cross_callpath(
        &db,
        &self.registry,
        &self.config.repo_path,
        &params.from,
        &params.to,
        params.target_repo.as_deref(),
    )
    .map_err(to_mcp_err)?;
    Ok(text_result(result))
}
```

**Step 3: Run tests and clippy**

Run: `cargo test`
Run: `cargo clippy --all-targets --all-features -- -D warnings`

**Step 4: Commit**

```
feat: add `cross_callpath` tool for multi-repo call chain tracing
```

---

### Task 12: Integration Test for Cross-Repo

**Files:**
- Create: `tests/cross_repo.rs`

**Step 1: Write integration test**

```rust
#![expect(clippy::unwrap_used, reason = "integration tests")]

use illu_rs::db::Database;
use illu_rs::indexer::{IndexConfig, index_repo};
use illu_rs::registry::{Registry, RepoEntry};
use illu_rs::server::tools::{cross_query, repos};
use std::path::PathBuf;

fn setup_two_repos() -> (tempfile::TempDir, tempfile::TempDir, Database, Database) {
    // Repo A: a library with a public function
    let dir_a = tempfile::TempDir::new().unwrap();
    let src_a = dir_a.path().join("src");
    std::fs::create_dir_all(&src_a).unwrap();
    std::fs::write(
        dir_a.path().join("Cargo.toml"),
        "[package]\nname = \"repo-a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    ).unwrap();
    std::fs::write(
        src_a.join("lib.rs"),
        "pub fn shared_helper(x: i32) -> i32 { x + 1 }\npub struct SharedType { pub value: i32 }\n",
    ).unwrap();

    // Repo B: uses the same function name (simulating shared dep)
    let dir_b = tempfile::TempDir::new().unwrap();
    let src_b = dir_b.path().join("src");
    std::fs::create_dir_all(&src_b).unwrap();
    std::fs::write(
        dir_b.path().join("Cargo.toml"),
        "[package]\nname = \"repo-b\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    ).unwrap();
    std::fs::write(
        src_b.join("lib.rs"),
        "fn caller() { let _ = shared_helper(42); }\nfn shared_helper(x: i32) -> i32 { x * 2 }\n",
    ).unwrap();

    // Index both
    let db_dir_a = dir_a.path().join(".illu");
    std::fs::create_dir_all(&db_dir_a).unwrap();
    let db_a = Database::open(&db_dir_a.join("index.db")).unwrap();
    let config_a = IndexConfig { repo_path: dir_a.path().to_path_buf() };
    index_repo(&db_a, &config_a).unwrap();

    let db_dir_b = dir_b.path().join(".illu");
    std::fs::create_dir_all(&db_dir_b).unwrap();
    let db_b = Database::open(&db_dir_b.join("index.db")).unwrap();
    let config_b = IndexConfig { repo_path: dir_b.path().to_path_buf() };
    index_repo(&db_b, &config_b).unwrap();

    (dir_a, dir_b, db_a, db_b)
}

#[test]
fn cross_query_finds_symbols_in_other_repos() {
    let (dir_a, dir_b, _db_a, _db_b) = setup_two_repos();
    let reg_dir = tempfile::TempDir::new().unwrap();
    let reg_path = reg_dir.path().join("registry.toml");
    let mut registry = Registry::load(&reg_path).unwrap();

    registry.register(RepoEntry {
        name: "repo-a".into(),
        path: dir_a.path().to_path_buf(),
        git_remote: None,
        git_common_dir: dir_a.path().join(".git"),
        last_indexed: "now".into(),
    });
    registry.register(RepoEntry {
        name: "repo-b".into(),
        path: dir_b.path().to_path_buf(),
        git_remote: None,
        git_common_dir: dir_b.path().join(".git"),
        last_indexed: "now".into(),
    });

    // Query from repo-a's perspective — should find shared_helper in repo-b
    let result = cross_query::handle_cross_query(
        &registry,
        dir_a.path(),
        "shared_helper",
        None, None, None, None, None, None,
    ).unwrap();

    assert!(result.contains("repo-b"), "Should find results in repo-b");
    assert!(result.contains("shared_helper"), "Should find the function");
}

#[test]
fn repos_shows_registered_repos() {
    let (dir_a, dir_b, _db_a, _db_b) = setup_two_repos();
    let reg_dir = tempfile::TempDir::new().unwrap();
    let reg_path = reg_dir.path().join("registry.toml");
    let mut registry = Registry::load(&reg_path).unwrap();

    registry.register(RepoEntry {
        name: "repo-a".into(),
        path: dir_a.path().to_path_buf(),
        git_remote: None,
        git_common_dir: dir_a.path().join(".git"),
        last_indexed: "now".into(),
    });
    registry.register(RepoEntry {
        name: "repo-b".into(),
        path: dir_b.path().to_path_buf(),
        git_remote: None,
        git_common_dir: dir_b.path().join(".git"),
        last_indexed: "now".into(),
    });

    let result = repos::handle_repos(&registry, dir_a.path()).unwrap();
    assert!(result.contains("repo-a"));
    assert!(result.contains("repo-b"));
    assert!(result.contains("active"));
}
```

**Step 2: Run integration tests**

Run: `cargo test --test cross_repo`

**Step 3: Commit**

```
test: add cross-repo integration tests
```

---

### Task 13: Update CLAUDE.md and GEMINI.md

**Files:**
- Modify: `src/main.rs` (update `illu_agent_section` and tool count)
- Modify: `CLAUDE.md` (add new tools, update tool count)

**Step 1: Update `illu_agent_section` in `main.rs`**

Update the tool count from 31 to 36. Add the new tools to the command table in `illu_agent_section`:

```
| `{cmd_prefix} repos` | `{tool_prefix}repos` | |
| `{cmd_prefix} cross_query <term>` | `{tool_prefix}cross_query` | `query: "<term>"` |
| `{cmd_prefix} cross_impact <symbol>` | `{tool_prefix}cross_impact` | `symbol_name: "<symbol>"` |
| `{cmd_prefix} cross_deps` | `{tool_prefix}cross_deps` | |
| `{cmd_prefix} cross_callpath <from> <to>` | `{tool_prefix}cross_callpath` | `from: "<from>", to: "<to>"` |
```

**Step 2: Update CLAUDE.md**

Add new key patterns for registry and cross-repo tools. Update the tool count from 31 to 36. Add new tool entries to the command table.

Add to Key Patterns section:
```
- **Registry** — Auto-populated at `~/.illu/registry.toml` on every `illu serve` startup. Tracks repo name, path, git remote, and last indexed timestamp. Worktrees dedup by `git_common_dir`.
- **Cross-repo tools** — Open other repos' `.illu/index.db` read-only on demand via `Database::open_readonly`. Name-based matching across repos (no shared index). `cross_query`, `cross_impact`, `cross_deps`, `cross_callpath` all use the registry to find other repos.
- **Global install** — `illu install` writes MCP config + instruction sections to `~/.claude/` and `~/.gemini/` globally. Uses CWD auto-detection (no `--repo` flag).
```

**Step 3: Update `ServerInfo` instructions in `src/server/mod.rs`**

Add the 5 new tools to the instructions string.

**Step 4: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`

**Step 5: Commit**

```
docs: update CLAUDE.md, GEMINI.md, and tool descriptions for new tools
```

---

### Task 14: Final Verification

**Step 1: Run full test suite**

Run: `cargo test`
Expected: All tests pass.

**Step 2: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: No warnings.

**Step 3: Check formatting**

Run: `cargo fmt --all -- --check`
Expected: No formatting issues.

**Step 4: Manual smoke test**

Build and test the install command:
```bash
cargo build
./target/debug/illu-rs install
cat ~/.claude/settings.json | grep illu
```

Test auto-detection (from repo root without --repo):
```bash
cd /path/to/some-rust-repo
/path/to/illu-rs serve  # should auto-detect and index
```

**Step 5: Commit any fixes, then final commit**

```
chore: final verification pass
```
