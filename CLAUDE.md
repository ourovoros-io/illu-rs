# CLAUDE.md

illu-rs is an MCP server that indexes Rust, Python, and TypeScript/JavaScript codebases with tree-sitter, stores symbols/refs/deps in SQLite, and optionally connects to rust-analyzer. Serves 53 MCP tools over stdio (40 core + 13 rust-analyzer).

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

### Rust Design Discipline (MANDATORY)

Before you write, modify, or meaningfully recommend Rust code, you MUST do the following in order:

1. **Run Rust preflight**: call `mcp__illu__rust_preflight` with the task, local symbols, std items, dependencies, and optional git ref. Treat its output as evidence to use, not as a design it invented.
2. **Plan before code**: write a short plan first. Name the data flow, invariants, failure cases, and the exact structs/enums/newtypes/collections you intend to use.
3. **Choose data structures deliberately**: justify each major type by ownership, mutability, ordering, lookup, and lifetime needs. Prefer representations that make invalid states unrepresentable.
4. **Read docs before use**: verify the actual semantics of each non-trivial type, trait, method, macro, or standard-library API before relying on it. Use `mcp__illu__std_docs` for standard-library items, `mcp__illu__docs` for dependencies, and `mcp__illu__context` for local types. NEVER assume behavior from memory or name similarity.
5. **Axiom pass before Rust**: query `mcp__illu__axioms` twice before significant Rust generation if `mcp__illu__rust_preflight` did not already supply both:
   - baseline quality query: `planning data structures documentation comments idiomatic rust verification performance`
   - task query: the concrete feature, bug, or API you are working on
6. **Write idiomatic Rust**: follow The Rust Book, Rust for Rustaceans, and illu axioms. Prefer ownership/borrowing, enums, iterators, explicit error handling, and compile-time modeling over ported Java/C++/Python patterns.
7. **Comments are first-class**: comments must explain invariants, safety conditions, concurrency assumptions, ownership rationale, or why a design exists. Delete comments that merely narrate syntax.
8. **Gate before final**: before final answer or commit for any Rust diff, call `mcp__illu__quality_gate` with the plan, docs verified, impact checked, tests run, and safety/performance evidence when relevant. Treat `BLOCKED` as not ready.

### Subagent instructions (MANDATORY)

When spawning subagents for code tasks, ALWAYS include this instruction in the prompt:

"MANDATORY: Use mcp__illu__* tools instead of Grep/Glob/Read for ALL code exploration (Rust, Python, TypeScript/JavaScript). NEVER use Read to view source files — use mcp__illu__context instead. NEVER use Grep to search code — use mcp__illu__query instead. Only use Read/Grep/Glob for non-code content (config, docs, logs). Before giving Rust implementation advice, first call mcp__illu__rust_preflight, make a short plan, choose data structures deliberately, verify docs for every non-trivial API with mcp__illu__std_docs/mcp__illu__docs/mcp__illu__context, and run mcp__illu__quality_gate before final answer or commit."

Prefer `illu-explore`, `illu-review`, `illu-refactor` agents when available.

### Workflow

1. **Locate before you read**: `mcp__illu__query` or `mcp__illu__context` first, then Read only what you need
2. **Rust preflight before you generate Rust**: use `mcp__illu__rust_preflight` to gather axioms, local context, impact hints, std docs, dependency docs, and model-failure reminders
3. **Plan before you generate Rust**: produce a short design plan naming data structures, ownership/mutability, invariants, and error handling
4. **Impact before you change**: always run `mcp__illu__impact` before modifying any public symbol
5. **Save tokens**: use `sections` param on context/batch_context to fetch only what you need
6. **Production focus**: use `exclude_tests: true` to filter out test functions
7. **Cross-repo**: use `mcp__illu__cross_query`/`mcp__illu__cross_impact`/`mcp__illu__cross_deps`/`mcp__illu__cross_callpath` — NEVER navigate to or read files from other repositories directly
8. **Axiom pass**: use `mcp__illu__axioms` with `planning data structures documentation comments idiomatic rust verification performance` first, then with the concrete task context before meaningful Rust design/code
9. **Documentation pass**: use `mcp__illu__context` with `sections: ["docs", "source"]` for local types, `mcp__illu__docs` for dependency types, and `mcp__illu__std_docs` for standard-library behavior
10. **Quality gate**: use `mcp__illu__quality_gate` before final answer or commit; `BLOCKED` means keep working

### Hard Constraints (Middleware Interception Simulation)
You MUST act as if a middleware intercepts your native tool calls. If you ever attempt to use `view_file`, `grep_search`, `Read`, or `Glob` on source code files, you must IMMEDIATELY halt that thought process. Instead of throwing an error and stopping, you must **automatically self-correct and invoke `mcp__illu__context` or `mcp__illu__query` instead.**

If you are about to write or modify Rust code, or propose a Rust architectural plan, and you have NOT yet called `mcp__illu__rust_preflight` for the task, you must IMMEDIATELY halt your generation. You must call `mcp__illu__rust_preflight` first.

If you are about to write or modify Rust code, or propose a Rust architectural plan, and you have NOT yet produced a short plan naming the data structures, ownership/mutability model, and error strategy, you must IMMEDIATELY halt your generation. You must write that plan first.

If you have NOT yet queried `mcp__illu__axioms` using both the baseline quality query `planning data structures documentation comments idiomatic rust verification performance` and the task-specific context in this session, you must IMMEDIATELY halt your generation. You must **invoke `mcp__illu__axioms` first, and only then proceed with Rust generation.**

If you are about to rely on the behavior of a type, trait, method, macro, or standard-library API whose semantics you have not verified from documentation or authoritative code context, you must IMMEDIATELY halt and read the docs first. Standard library items require `mcp__illu__std_docs` and are NOT exempt.

If you are about to final-answer or commit a Rust diff and have NOT called `mcp__illu__quality_gate` with your plan, docs verified, impact checked, and tests run, you must IMMEDIATELY halt and run `mcp__illu__quality_gate`. If it returns `BLOCKED`, do not present the work as complete.

If you are about to add comments that merely restate what the code already says, you must delete or rewrite them so they capture invariants, why, safety, or other non-obvious context.
</CRITICAL_INSTRUCTION_OVERRIDE>
<!-- illu:end -->
