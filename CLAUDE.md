# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

illu-rs is an MCP (Model Context Protocol) server that indexes Rust codebases and exposes code intelligence tools. It parses source files with tree-sitter, stores symbols/refs/deps in SQLite, and serves 7 MCP tools over stdio: `query`, `context`, `impact`, `diff_impact`, `docs`, `overview`, `tree`.

## Commands

```bash
# Build
cargo build
cargo build --release

# Test
cargo test                              # All tests
cargo test --lib                        # Unit tests only
cargo test --test integration           # Integration tests only
cargo test --lib -- db::tests           # Tests in a specific module
cargo test --lib -- test_index_workspace  # Single test by name

# Lint and format
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check

# Run against a repo (indexes then starts MCP server on stdio)
RUST_LOG=info cargo run -- /path/to/repo
```

## Architecture

**Startup flow:** `main.rs` opens/creates `{repo}/.illu/index.db`, runs `index_repo` synchronously, then starts the MCP server on stdio via rmcp.

**Three modules** (`src/lib.rs` exports `db`, `indexer`, `server`):

### `db` — SQLite layer (`src/db.rs`)
Single file, owns `rusqlite::Connection`. All SQL lives here. Key tables:
- `files` (with `crate_id` FK), `symbols` (with `impl_type`), `symbol_refs` — code index
- `trait_impls` — trait-to-type mapping
- `crates`, `crate_deps` — workspace graph
- `dependencies`, `docs` (with `module`) — external deps and cached docs
- FTS5 virtual tables (`symbols_fts`, `docs_fts`) for full-text search

### `indexer` — Indexing pipeline (`src/indexer/`)
- `mod.rs` — Orchestrator. `index_repo` detects workspace vs single-crate, dispatches to `index_workspace` or `index_single_crate`. Shared phases: symbol ref extraction, skill file generation, metadata update.
- `workspace.rs` — `parse_workspace_toml`, `resolve_member_deps` (handles `workspace = true` inheritance), `extract_path_deps` (inter-crate deps).
- `parser.rs` — Tree-sitter parsing. `parse_rust_source` extracts symbols (with `impl_type` for methods); `extract_refs` uses import maps and crate maps for qualified path resolution. Detects `self.method()` calls for impl-type-aware ref resolution.
- `dependencies.rs` — Parses `Cargo.toml`/`Cargo.lock`, resolves direct vs transitive deps.
- `store.rs` — Writes parsed symbols/deps to DB.
- `docs.rs` — Fetches docs (cargo doc JSON → docs.rs → GitHub README). Two-tier storage: crate summary + per-module detail.
- `cargo_doc.rs` — Parses nightly rustdoc JSON into structured per-module docs.

### `server` — MCP server (`src/server/`)
- `mod.rs` — `IlluServer` wraps `Arc<Mutex<Database>>`. Uses rmcp's `#[tool_router]`, `#[tool_handler]`, `#[tool]` macros with `Parameters<T>` wrapper. Tool param structs derive `JsonSchema` via `rmcp::schemars` re-export.
- `tools/` — Each tool handler is a pure function `handle_*(db, ...) -> Result<String>`: `query.rs`, `context.rs`, `impact.rs`, `diff_impact.rs`, `docs.rs`, `overview.rs`, `tree.rs`.

## Key Patterns

- **`Database` is not `Sync`** — `rusqlite::Connection` requires `Mutex` wrapping for the MCP server's async context.
- **`rmcp::schemars`** — Tool param structs must use the `schemars` re-exported by rmcp, not a separate schemars crate.
- **Symbol refs use qualified resolution** — `extract_refs` builds an import map from `use` declarations and a crate map from the workspace. Refs resolve via import → same-file → same-crate → global name fallback. `self.method()` resolves via `impl_type` matching.
- **Workspace detection** — Presence of `[workspace]` in root `Cargo.toml` triggers multi-crate indexing. Single-crate repos get one implicit row in `crates`.
- **Impact tool** — Uses recursive CTE on `symbol_refs` (depth limit 5) for symbol-level impact. For workspaces with >1 crate, prepends an "Affected Crates" section using `crate_deps` transitive query.

## Lint Configuration

Rust 2024 edition with strict clippy (see `Cargo.toml [lints.clippy]`):
- `unwrap_used = "deny"` — use `?`, `unwrap_or`, or `let...else`
- `print_stdout/print_stderr = "deny"` — use `tracing` macros
- `panic/todo/unimplemented = "deny"`
- `allow_attributes = "deny"` — use `#[expect(lint, reason = "...")]` instead
- Tests opt out via `#[expect(clippy::unwrap_used, reason = "tests")]` on the test module

<!-- illu:start -->
## Code Intelligence (illu)

This repo is indexed by illu. **Use illu tools as your first step** — before reading files, before grep, before guessing at code structure.

### When to use illu

- **Starting any task**: `illu query` the relevant symbols to understand what exists
- **Before modifying a function/struct/trait**: `illu impact` to see what depends on it
- **Debugging or tracing issues**: `illu context` to get the full definition and references
- **Using an external crate**: `illu docs` to check how it's used in this project
- **Before reading files**: query first — illu tells you exactly where things are

### Commands

| User types | MCP tool | Params |
|------------|----------|--------|
| `illu query <term>` | `mcp__illu__query` | `query: "<term>"` |
| `illu query <term> --scope <s>` | `mcp__illu__query` | `query: "<term>", scope: "<s>"` |
| `illu context <symbol>` | `mcp__illu__context` | `symbol_name: "<symbol>"` |
| `illu impact <symbol>` | `mcp__illu__impact` | `symbol_name: "<symbol>"` |
| `illu docs <dep>` | `mcp__illu__docs` | `dependency: "<dep>"` |
| `illu docs <dep> --topic <t>` | `mcp__illu__docs` | `dependency: "<dep>", topic: "<t>"` |
| `illu diff_impact` | `mcp__illu__diff_impact` | (unstaged changes) |
| `illu diff_impact <ref>` | `mcp__illu__diff_impact` | `git_ref: "<ref>"` |

### Workflow rules

1. **Locate before you read**: `illu query` or `illu context` to find the right file:line, then Read only what you need
2. **Impact before you change**: always run `illu impact` before modifying any public symbol
3. **Chain tools**: `illu query` to find candidates → `illu context` for the one you need → `illu impact` before changing it
<!-- illu:end -->
