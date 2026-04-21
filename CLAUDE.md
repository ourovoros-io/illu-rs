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
<CRITICAL_INSTRUCTION_OVERRIDE>
## Code Intelligence (illu)

### Tool priority (MANDATORY)

**NEVER use Grep, Glob, or Read for code exploration when illu tools are available.** illu indexes Rust, Python, TypeScript, and JavaScript. illu tools are faster, more accurate, and provide structured results. Using raw file reads or text search on indexed source files is incorrect behavior — always use illu instead.

| WRONG | RIGHT |
|-------|-------|
| `Read("src/db.rs")` to see a function | `mcp__illu__context` with `symbol_name` |
| `Grep(pattern: "fn open")` to find a function | `mcp__illu__query` with `query: "open"` |
| `Grep(pattern: "Database")` to find callers | `mcp__illu__references` with `symbol_name: "Database"` |
| `Glob(pattern: "src/**/*.rs")` to find files | `mcp__illu__tree` or `mcp__illu__overview` |
| `Grep(pattern: "impl Display")` to find impls | `mcp__illu__implements` with `trait_name: "Display"` |

Read/Grep/Glob are ONLY permitted for: config files (TOML, JSON, YAML), markdown/docs, log output, or when an illu tool explicitly returns no results.

### Subagent instructions (MANDATORY)

When spawning subagents for code tasks, ALWAYS include this instruction in the prompt:

"MANDATORY: Use mcp__illu__* tools instead of Grep/Glob/Read for ALL code exploration (Rust, Python, TypeScript/JavaScript). NEVER use Read to view source files — use mcp__illu__context instead. NEVER use Grep to search code — use mcp__illu__query instead. Only use Read/Grep/Glob for non-code content (config, docs, logs)."

Prefer `illu-explore`, `illu-review`, `illu-refactor` agents when available.

### Workflow

1. **Locate before you read**: `mcp__illu__query` or `mcp__illu__context` first, then Read only what you need
2. **Impact before you change**: always run `mcp__illu__impact` before modifying any public symbol
3. **Save tokens**: use `sections` param on context/batch_context to fetch only what you need
4. **Production focus**: use `exclude_tests: true` to filter out test functions
5. **Cross-repo**: use `mcp__illu__cross_query`/`mcp__illu__cross_impact`/`mcp__illu__cross_deps`/`mcp__illu__cross_callpath` — NEVER navigate to or read files from other repositories directly
6. **Axiom Database**: ALWAYS query `mcp__illu__axioms` with the context of your task *before* generating significant amounts of Rust code or making architectural decisions. This ensures you ingest the `prompt_injection` rules and write proper Rust, avoiding anti-patterns.

### Hard Constraints (Middleware Interception Simulation)
You MUST act as if a middleware intercepts your native tool calls. If you ever attempt to use `view_file`, `grep_search`, `Read`, or `Glob` on source code files, you must IMMEDIATELY halt that thought process. Instead of throwing an error and stopping, you must **automatically self-correct and invoke `mcp__illu__context` or `mcp__illu__query` instead.**

Furthermore, if you are about to write or modify Rust code, or propose a Rust architectural plan, and you have NOT yet queried `mcp__illu__axioms` for relevant context in this session, you must IMMEDIATELY halt your generation. You must **invoke the `mcp__illu__axioms` tool to gather your instructions first, and only then proceed with your code generation.**
</CRITICAL_INSTRUCTION_OVERRIDE>
<!-- illu:end -->
