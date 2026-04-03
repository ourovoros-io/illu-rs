# CLAUDE.md

illu-rs is an MCP server that indexes Rust, Python, and TypeScript/JavaScript codebases with tree-sitter, stores symbols/refs/deps in SQLite, and optionally connects to rust-analyzer. Serves 49 MCP tools over stdio (36 core + 13 rust-analyzer).

## Commands

```bash
cargo build                             # Debug build
cargo build --release                   # Release build
cargo test                              # All tests
cargo test --lib                        # Unit tests only
cargo test --test integration           # Integration tests only
cargo test --lib -- db::tests           # Tests in a specific module
cargo test --lib -- test_index_workspace  # Single test by name
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
RUST_LOG=info cargo run -- /path/to/repo  # Index + serve
```

## Architecture

`main.rs` opens `{repo}/.illu/index.db`, runs `index_repo`, optionally spawns rust-analyzer, then starts MCP on stdio via rmcp. Use `--no-ra` to skip rust-analyzer.

Four modules (`src/lib.rs` exports `db`, `indexer`, `ra`, `server`):

- **`db`** (`src/db.rs`) — Single file, owns `rusqlite::Connection`. All SQL lives here. Tables: `files`, `symbols` (with `impl_type`, `is_test`), `symbol_refs` (with `confidence`, `ref_line`), `trait_impls`, `crates`, `crate_deps`, `dependencies`, `docs`. FTS5 virtual tables for search.
- **`indexer`** (`src/indexer/`) — `mod.rs` orchestrates. `parser.rs` (Rust), `ts_parser.rs` (TS/JS), `py_parser.rs` (Python) do tree-sitter parsing. `workspace.rs` handles Cargo workspaces. `store.rs` writes to DB. `dependencies.rs` parses lockfiles. `docs.rs`/`cargo_doc.rs` fetch external docs.
- **`server`** (`src/server/`) — `mod.rs` has `IlluServer` wrapping `Arc<Mutex<Database>>` + `Option<Arc<RaClient>>`. Uses rmcp macros. `tools/` has one file per tool handler as pure functions.
- **`ra`** (`src/ra/`) — Optional rust-analyzer LSP client. `client.rs` spawns process, `transport.rs` handles notifications, `ops.rs` composes LSP operations, `lsp.rs` wraps LSP methods.

## Non-Obvious Conventions

- **`Database` is not `Sync`** — `rusqlite::Connection` requires `Mutex` wrapping for async context.
- **`rmcp::schemars`** — Tool param structs must use the `schemars` re-exported by rmcp, not a separate crate.
- **`Type::method` syntax** — Many tools accept `Database::open` style names, split into name + `impl_type` for lookup.
- **Confidence scoring** — `symbol_refs.confidence` is `high` (qualified resolution) or `low` (name-only fallback). Most queries filter to `high` only; `boundary` uses all levels.
- **RA tools use `file:line:col`** — All `ra_*` tools take position strings, not symbol names. Write operations (`ra_rename`, `ra_safe_rename`, `ra_ssr`) gate on RA readiness; reads don't.

## Lint Configuration

Rust 2024 edition with strict clippy (see `Cargo.toml [lints.clippy]`):
- `unwrap_used = "deny"` — use `?`, `unwrap_or`, or `let...else`
- `print_stdout/print_stderr = "deny"` — use `tracing` macros
- `panic/todo/unimplemented = "deny"`
- `allow_attributes = "deny"` — use `#[expect(lint, reason = "...")]` instead
- Tests opt out via `#[expect(clippy::unwrap_used, reason = "tests")]` on the test module

<!-- illu:start -->
<!-- illu:end -->
