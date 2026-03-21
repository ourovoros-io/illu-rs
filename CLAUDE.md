# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

illu-rs is an MCP (Model Context Protocol) server that indexes Rust codebases and exposes code intelligence tools. It parses source files with tree-sitter, stores symbols/refs/deps in SQLite, and serves 31 MCP tools over stdio: `query`, `context`, `batch_context`, `impact`, `diff_impact`, `callpath`, `unused`, `freshness`, `docs`, `overview`, `tree`, `crate_graph`, `implements`, `neighborhood`, `type_usage`, `file_graph`, `symbols_at`, `stats`, `hotspots`, `rename_plan`, `similar`, `boundary`, `health`, `blame`, `history`, `references`, `doc_coverage`, `test_impact`, `orphaned`, `graph_export`, `crate_impact`.

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
- `parser.rs` — Tree-sitter parsing. `parse_rust_source` extracts symbols (with `impl_type` for methods and enum variants); `extract_refs` uses import maps and crate maps for qualified path resolution. Detects `self.method()` calls for impl-type-aware ref resolution. Enum variants are indexed as `EnumVariant` symbols with `impl_type` set to the parent enum name.
- `dependencies.rs` — Parses `Cargo.toml`/`Cargo.lock`, resolves direct vs transitive deps.
- `store.rs` — Writes parsed symbols/deps to DB.
- `docs.rs` — Fetches docs (cargo doc JSON → docs.rs → GitHub README). Two-tier storage: crate summary + per-module detail.
- `cargo_doc.rs` — Parses nightly rustdoc JSON into structured per-module docs.

### `server` — MCP server (`src/server/`)
- `mod.rs` — `IlluServer` wraps `Arc<Mutex<Database>>`. Uses rmcp's `#[tool_router]`, `#[tool_handler]`, `#[tool]` macros with `Parameters<T>` wrapper. Tool param structs derive `JsonSchema` via `rmcp::schemars` re-export.
- `tools/` — Each tool handler is a pure function `handle_*(db, ...) -> Result<String>`: `query.rs`, `context.rs`, `batch_context.rs`, `impact.rs`, `diff_impact.rs`, `callpath.rs`, `unused.rs`, `freshness.rs`, `docs.rs`, `overview.rs`, `tree.rs`, `crate_graph.rs`, `implements.rs`, `neighborhood.rs`, `type_usage.rs`, `file_graph.rs`, `symbols_at.rs`, `stats.rs`, `hotspots.rs`, `rename_plan.rs`, `similar.rs`, `boundary.rs`, `health.rs`, `blame.rs`.

## Key Patterns

- **`Database` is not `Sync`** — `rusqlite::Connection` requires `Mutex` wrapping for the MCP server's async context.
- **`rmcp::schemars`** — Tool param structs must use the `schemars` re-exported by rmcp, not a separate schemars crate.
- **Symbol refs use qualified resolution** — `extract_refs` builds an import map from `use` declarations and a crate map from the workspace. Refs resolve via import → same-file → same-crate → global name fallback. `self.method()` resolves via `impl_type` matching.
- **Workspace detection** — Presence of `[workspace]` in root `Cargo.toml` triggers multi-crate indexing. Single-crate repos get one implicit row in `crates`.
- **Impact tool** — Uses recursive CTE on `symbol_refs` (depth limit 5) for symbol-level impact. Supports `Type::method` syntax (e.g. `Database::open`) — splits into name + impl_type for CTE seed. For workspaces with >1 crate, prepends an "Affected Crates" section using `crate_deps` transitive query. Appends a "Related Tests" section listing `#[test]` functions that transitively call the symbol.
- **Context tool** — Shows callers ("Called By" section) alongside callees. Supports `Type::method` syntax (e.g. `Database::new`) via `impl_type` column lookup, and optional `file` parameter for scoped results.
- **Diff impact tool** — After listing changed symbols and downstream impact, appends a "Related Tests" section with a suggested `cargo test` command.
- **Callpath tool** — BFS on `symbol_refs` from source to target symbol. Returns shortest call chain with file locations.
- **Batch context tool** — Iterates over a list of symbol names, calling `handle_context` for each. Returns concatenated results.
- **Unused tool** — LEFT JOIN `symbol_refs` to find symbols with zero incoming refs. Excludes entry points (`main`, `#[test]`), `use`/`mod`/`impl` kinds, and `EnumVariant` symbols.
- **Freshness tool** — Compares `get_commit_hash` against `git rev-parse HEAD`. Lists changed files via `git diff --name-only`. Does NOT call `refresh()` — reports current state only.
- **Crate graph tool** — Formats `crate_deps` as an adjacency list. Identifies root crates (no dependents) and leaf crates (no deps).
- **Query filters** — `attribute`, `signature`, `kind`, and `path` filters are all combinable. The broadest filter is used for the initial DB query, then remaining filters are applied as `.retain()` post-filters. `doc_comments` scope searches doc comment content. Default scope is `symbols` (not `all`) — use `scope: "all"` or `scope: "docs"` to include dependency docs.
- **Context sections** — Optional `sections` parameter controls which sections render: `source`, `callers`, `callees`, `tested_by`, `traits`, `docs`. Omit for all sections. Header always renders.
- **Implements tool** — Uses `trait_impls` table to query trait/type relationships bidirectionally.
- **Neighborhood tool** — Bidirectional BFS using `get_callees_by_name` (downstream) and `get_callers_by_name` (upstream) within N hops.
- **Callpath all_paths** — When `all_paths=true`, uses DFS with backtracking to find up to `max_paths` paths (default 5).
- **Diff impact changes_only** — When `changes_only=true`, skips downstream impact and test coverage, returns only changed symbols.
- **Type usage tool** — Best-effort text search on `signature` and `details` columns to find where a type is used as params, returns, and struct fields.
- **File graph tool** — Derives file-level dependencies from `symbol_refs` table (no new tables needed). If symbol A in file X references symbol B in file Y, X depends on Y.
- **Symbols_at tool** — Wraps existing `get_symbols_at_lines` DB method for file:line lookup. Returns all symbols whose line range contains the given line.
- **Query bodies scope** — `scope: "bodies"` searches within function body text via LIKE on the `body` column. Also supports `limit` parameter to cap result count.
- **Query/overview limit** — Optional `limit` parameter truncates results. Overview distributes limit across files breadth-first (each file gets at least 1 symbol), shows "(limited to N, M total)" when truncated.
- **Context callers_path** — Optional `callers_path` parameter filters callers and callees to a path prefix (e.g. `"src/"` to exclude test callers).
- **Context related section** — Shows sibling symbols in the same file with matching `impl_type`. Labels as "Related (impl X)" for methods or "Related (same file)" for top-level.
- **Stats tool** — Aggregates file/symbol counts, kind breakdown, test coverage ratio, most-referenced symbols, and largest files into a compact dashboard.
- **Hotspots tool** — Identifies high-risk symbols: most-referenced (fragile to change), most-referencing (high complexity), and largest functions (by line span).
- **Rename_plan tool** — Unified preview of all locations referencing a symbol: definition, call sites, type usage in signatures, struct fields, trait implementations, and doc comments.
- **Similar tool** — Finds symbols with similar signatures and callee patterns. Scores by return type match, shared parameter types, and shared callees.
- **Full signature capture** — Parser captures complete multi-line signatures (up to `{`), collapsed to single line. No more first-line truncation.
- **Ref confidence scoring** — `symbol_refs` has a `confidence` column: `high` for impl/file-qualified resolution, `low` for name-only fallback. `file_graph` uses high-confidence only by default.
- **Caller/callee line numbers** — Context callers and callees now show `name (file:line)` for direct navigation.
- **Neighborhood tree format** — `format: "tree"` + `direction: "down"/"up"` renders hierarchical call tree with `├──`/`└──` characters.
- **Body search snippets** — `scope: "bodies"` results now show the first matching line from the body as a snippet.
- **Boundary tool** — Classifies symbols as "Public API" (called from outside path) or "Internal Only" (safe to refactor).
- **Health tool** — Reports ref confidence distribution, signature quality, noise sources, and coverage metrics.
- **Blame tool** — Runs `git blame` on a symbol's line range, summarizes author, date, and commit message.
- **Constructor tracking** — `new`, `from`, `into`, `clone`, `default`, `build`, `init` are tracked as symbol refs (removed from `NOISY_SYMBOL_NAMES`). `impl_type` disambiguation prevents cross-type collisions.
- **References tool** — Unified view of all references to a symbol: definition, call sites, type usage in signatures, trait implementations. Consolidates data from callers, type_usage, and implements.
- **Doc coverage tool** — Finds symbols missing doc comments. Shows coverage percentage and lists undocumented symbols grouped by file.
- **Test impact tool** — Shows which tests break when changing a symbol. Combines impact analysis with test discovery, returns suggested `cargo test` command.
- **Orphaned tool** — Finds symbols with no callers AND no test coverage (intersection of unused + untested). These are safe to remove.
- **Graph export tool** — Exports call graphs or file dependency graphs in DOT/Graphviz format. Provide `symbol_name` for call graph or `path` for file graph.
- **Crate impact tool** — Shows which workspace crates are affected by changing a symbol. Bridges symbol-level impact with crate-level dependencies.
- **is_test column** — Symbols table has `is_test` column (boolean), set at index time from attributes. Used by `get_related_tests()` for efficient test lookups. `get_related_tests` accepts optional `impl_type` for `Type::method` resolution.
- **Symbol resolution priority** — `resolve_symbol` tries: (1) `Type::method` qualified lookup, (2) exact name match, (3) FTS/fuzzy search. Exact match prevents noise from partial matches.
- **Stats confidence filter** — `handle_stats` most-referenced uses `confidence = "high"` to avoid inflated counts from name-only fallback refs. Shows qualified `ImplType::name` format.
- **LIKE escape** — All path-based LIKE queries use `escape_like()` + `ESCAPE '\\'` clause to handle paths with `%` or `_`.
- **Qualified caller/callee names** — Context callers/callees show `ImplType::method` format when impl_type is available, preventing ambiguity.
- **Macro body ref extraction** — `collect_body_refs()` descends into `macro_invocation` token trees to extract potential symbol references.

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
- **Finding call paths**: `illu callpath` to trace how one symbol reaches another
- **Dead code detection**: `illu unused` to find unreferenced symbols
- **Index health**: `illu freshness` to check if the index is current

### Commands

| User types | MCP tool | Params |
|------------|----------|--------|
| `illu query <term>` | `mcp__illu__query` | `query: "<term>"` |
| `illu query <term> --scope <s>` | `mcp__illu__query` | `query: "<term>", scope: "<s>"` |
| `illu context <symbol>` | `mcp__illu__context` | `symbol_name: "<symbol>"` |
| `illu context Type::method` | `mcp__illu__context` | `symbol_name: "Type::method"` |
| `illu context <symbol> --file <f>` | `mcp__illu__context` | `symbol_name: "<symbol>", file: "<f>"` |
| `illu impact <symbol>` | `mcp__illu__impact` | `symbol_name: "<symbol>"` |
| `illu impact <symbol> --depth 1` | `mcp__illu__impact` | `symbol_name: "<symbol>", depth: 1` |
| `illu docs <dep>` | `mcp__illu__docs` | `dependency: "<dep>"` |
| `illu docs <dep> --topic <t>` | `mcp__illu__docs` | `dependency: "<dep>", topic: "<t>"` |
| `illu callpath <from> <to>` | `mcp__illu__callpath` | `from: "<from>", to: "<to>"` |
| `illu batch_context <sym1> <sym2>` | `mcp__illu__batch_context` | `symbols: ["<sym1>", "<sym2>"]` |
| `illu unused` | `mcp__illu__unused` | |
| `illu unused --path src/server/` | `mcp__illu__unused` | `path: "src/server/"` |
| `illu freshness` | `mcp__illu__freshness` | |
| `illu crate_graph` | `mcp__illu__crate_graph` | |

### Workflow rules

1. **Locate before you read**: `illu query` or `illu context` to find the right file:line, then Read only what you need
2. **Impact before you change**: always run `illu impact` before modifying any public symbol
3. **Chain tools**: `illu query` to find candidates → `illu context` for the one you need → `illu impact` before changing it
<!-- illu:end -->
