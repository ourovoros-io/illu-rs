# Global Multi-Repo + Worktree Support

Date: 2026-03-21

## Problem

illu-rs currently requires per-repo setup (`illu init`) and serves one repo per MCP server instance. This creates friction for users who:

1. Work across multiple repos that communicate or depend on each other
2. Use git worktrees with parallel Claude/Gemini agents (each worktree needs separate setup)
3. Want code intelligence available in every Rust repo without manual setup

## Design

### 1. Auto-Detection

`illu serve` without `--repo` auto-detects the repo from CWD:

```rust
// Resolve repo root from current directory
fn detect_repo() -> Result<PathBuf> {
    // git rev-parse --show-toplevel
    // Returns worktree root for worktrees, repo root otherwise
}
```

`--repo` still works as an explicit override. This is backward compatible.

**Worktrees:** `git rev-parse --show-toplevel` returns the worktree root, not the main repo. Each worktree gets its own `.illu/index.db`. Complete isolation, zero config.

### 2. Global Install

`illu install` writes global config for both Claude Code and Gemini CLI — one command, both agents set up:

| Target | MCP Config | Instructions |
|--------|-----------|--------------|
| Claude Code | `~/.claude/settings.json` | `~/.claude/CLAUDE.md` |
| Gemini CLI | `~/.gemini/settings.json` | `~/.gemini/GEMINI.md` |

MCP config uses no `--repo` — relies on CWD auto-detection:

```json
{
  "mcpServers": {
    "illu": {
      "command": "/path/to/illu",
      "args": ["serve"],
      "env": { "RUST_LOG": "warn" }
    }
  }
}
```

`illu init` remains for per-repo overrides (repo-local `.mcp.json`, repo-local CLAUDE.md/GEMINI.md sections).

### 3. Registry

Every `illu serve` startup auto-registers the repo in `~/.illu/registry.toml`:

```toml
[[repos]]
name = "illu-rs"
path = "/Users/georgiosdelkos/Documents/GitHub/illu-rs"
git_remote = "git@github.com:GeorgiosDelkos/illu-rs.git"
last_indexed = "2026-03-21T14:30:00Z"
```

**Identity:** Canonical path. Worktrees of the same repo (detected via `git rev-parse --git-common-dir`) share identity — worktrees don't create duplicate registry entries.

**Auto-cleanup:** Dead paths pruned on startup.

**No manual management commands.** Registry is fully automatic.

### 4. Server Architecture

```rust
struct IlluServer {
    primary: Arc<Mutex<Database>>,       // CWD repo, always open
    primary_config: Arc<IndexConfig>,
    registry: Arc<Registry>,             // ~/.illu/registry.toml
}
```

- All 31 existing tools operate on `primary` — no API changes
- Cross-repo tools open other repos' DBs on demand (read-only, ephemeral)
- No ATTACH DATABASE — separate connections are simpler and avoid SQL rewrites

### 5. Cross-Repo Tools

**`repos`** — Dashboard of all registered repos with status (current/stale/missing), symbol counts.

**`cross_query`** — Search symbols across all registered repos. Same params as `query`, results grouped by repo.

**`cross_impact`** — "If I change Symbol in this repo, what breaks in other repos?" Name-based matching across repos' `symbol_refs`.

**`cross_deps`** — Inter-repo dependency graph. Scans `Cargo.toml` across repos for path deps and shared crate deps.

**`cross_callpath`** — Call chains spanning repo boundaries via shared dependencies.

**Constraint:** Cross-repo matching is name-based (no shared index for qualified resolution). `impl_type` disambiguation reduces false positives.

### 6. Non-Rust Repos

illu detects Rust repos by `Cargo.toml` presence. In a non-Rust repo:

- `illu serve` starts but returns a clear error on tool calls: "No Cargo.toml found — illu requires a Rust project"
- No crash, no indexing attempt
- Cross-repo tools still work (querying other registered repos)

## Implementation Phases

### Phase 1: Auto-Detection + Global Install
1. `detect_repo()` via `git rev-parse --show-toplevel` when `--repo` not provided
2. `illu install` command: writes global MCP configs + instruction files for Claude & Gemini
3. Backward compatible — `--repo` still works

### Phase 2: Registry
1. `Registry` struct: read/write/prune `~/.illu/registry.toml`
2. Auto-register on every `illu serve` startup
3. Worktree dedup via `git rev-parse --git-common-dir`
4. Dead path cleanup

### Phase 3: Cross-Repo Tools
1. `Database::open_readonly()` method
2. `repos` tool
3. `cross_query` tool
4. `cross_impact` tool
5. `cross_deps` tool
6. `cross_callpath` tool

### Phase 4: Worktree Polish
1. Global gitignore entry for `.illu/` (`~/.config/git/ignore`)
2. Verify git operations (blame, history, diff_impact) work from worktree context

## Testing

- Integration tests with temp repos + temp registries (override `~/.illu` path)
- Cross-repo tests: two temp repos, one depending on the other via path dep
- Worktree tests: create a worktree, verify independent indexing
