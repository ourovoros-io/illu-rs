# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

illu-rs is an MCP (Model Context Protocol) server that indexes Rust codebases and exposes code intelligence tools. It parses source files with tree-sitter, stores symbols/refs/deps in SQLite, and serves 4 MCP tools over stdio: `query`, `context`, `impact`, `docs`.

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
- `files` (with `crate_id` FK), `symbols`, `symbol_refs` — code index
- `crates`, `crate_deps` — workspace graph
- `dependencies`, `dependency_docs` — external deps
- FTS5 virtual tables (`symbols_fts`, `docs_fts`) for full-text search

### `indexer` — Indexing pipeline (`src/indexer/`)
- `mod.rs` — Orchestrator. `index_repo` detects workspace vs single-crate, dispatches to `index_workspace` or `index_single_crate`. Shared phases: symbol ref extraction, skill file generation, metadata update.
- `workspace.rs` — `parse_workspace_toml`, `resolve_member_deps` (handles `workspace = true` inheritance), `extract_path_deps` (inter-crate deps).
- `parser.rs` — Tree-sitter parsing. `parse_rust_source` extracts symbols; `extract_refs` scans function bodies for identifiers matching known symbol names.
- `dependencies.rs` — Parses `Cargo.toml`/`Cargo.lock`, resolves direct vs transitive deps.
- `store.rs` — Writes parsed symbols/deps to DB.
- `docs.rs` — Fetches docs from docs.rs/GitHub (currently skipped via `skip_doc_fetch`).

### `server` — MCP server (`src/server/`)
- `mod.rs` — `IlluServer` wraps `Arc<Mutex<Database>>`. Uses rmcp's `#[tool_router]`, `#[tool_handler]`, `#[tool]` macros with `Parameters<T>` wrapper. Tool param structs derive `JsonSchema` via `rmcp::schemars` re-export.
- `tools/` — Each tool handler is a pure function `handle_*(db, ...) -> Result<String>`: `query.rs`, `context.rs`, `impact.rs`, `docs.rs`.

## Key Patterns

- **`Database` is not `Sync`** — `rusqlite::Connection` requires `Mutex` wrapping for the MCP server's async context.
- **`rmcp::schemars`** — Tool param structs must use the `schemars` re-exported by rmcp, not a separate schemars crate.
- **Symbol refs are name-based** — `extract_refs` matches AST identifiers against a global `HashSet<String>` of known symbol names. No semantic resolution.
- **Workspace detection** — Presence of `[workspace]` in root `Cargo.toml` triggers multi-crate indexing. Single-crate repos get one implicit row in `crates`.
- **Impact tool** — Uses recursive CTE on `symbol_refs` (depth limit 5) for symbol-level impact. For workspaces with >1 crate, prepends an "Affected Crates" section using `crate_deps` transitive query.

## Lint Configuration

Rust 2024 edition with strict clippy (see `Cargo.toml [lints.clippy]`):
- `unwrap_used = "deny"` — use `?`, `unwrap_or`, or `let...else`
- `print_stdout/print_stderr = "deny"` — use `tracing` macros
- `panic/todo/unimplemented = "deny"`
- `allow_attributes = "deny"` — use `#[expect(lint, reason = "...")]` instead
- Tests opt out via `#[expect(clippy::unwrap_used, reason = "tests")]` on the test module
