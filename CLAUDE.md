# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

illu-rs is an MCP (Model Context Protocol) server that indexes Rust codebases and exposes code intelligence tools. It parses source files with tree-sitter, stores symbols/refs/deps in SQLite, and optionally connects to rust-analyzer for compiler-accurate operations. Serves 49 MCP tools over stdio: 36 core tools (`query`, `context`, `batch_context`, `impact`, `diff_impact`, `callpath`, `unused`, `freshness`, `docs`, `overview`, `tree`, `crate_graph`, `implements`, `neighborhood`, `type_usage`, `file_graph`, `symbols_at`, `stats`, `hotspots`, `rename_plan`, `similar`, `boundary`, `health`, `blame`, `history`, `references`, `doc_coverage`, `test_impact`, `orphaned`, `graph_export`, `crate_impact`, `repos`, `cross_query`, `cross_impact`, `cross_deps`, `cross_callpath`) + 13 rust-analyzer tools (`ra_definition`, `ra_hover`, `ra_diagnostics`, `ra_call_hierarchy`, `ra_type_hierarchy`, `ra_rename`, `ra_safe_rename`, `ra_code_actions`, `ra_expand_macro`, `ra_ssr`, `ra_context`, `ra_syntax_tree`, `ra_related_tests`).

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

**Startup flow:** `main.rs` opens/creates `{repo}/.illu/index.db`, runs `index_repo` synchronously, optionally spawns rust-analyzer in the background, then starts the MCP server on stdio via rmcp. Use `--no-ra` to skip rust-analyzer.

**Four modules** (`src/lib.rs` exports `db`, `indexer`, `ra`, `server`):

### `db` — SQLite layer (`src/db.rs`)
Single file, owns `rusqlite::Connection`. All SQL lives here. Key tables:
- `files` (with `crate_id` FK), `symbols` (with `impl_type`), `symbol_refs` — code index
- `trait_impls` — trait-to-type mapping
- `crates`, `crate_deps` — workspace graph
- `dependencies`, `docs` (with `module`) — external deps and cached docs
- FTS5 virtual tables (`symbols_fts`, `docs_fts`) for full-text search

### `indexer` — Indexing pipeline (`src/indexer/`)
- `mod.rs` — Orchestrator. `index_repo` detects workspace vs single-crate, dispatches to `index_workspace` or `index_single_crate`. Both index `src/` plus `tests/`, `benches/`, and `examples/` directories when present. Shared phases: symbol ref extraction, skill file generation, metadata update.
- `workspace.rs` — `parse_workspace_toml`, `resolve_member_deps` (handles `workspace = true` inheritance), `extract_path_deps` (inter-crate deps).
- `parser.rs` — Tree-sitter parsing. `parse_rust_source` extracts symbols (with `impl_type` for methods and enum variants); `extract_refs` uses import maps and crate maps for qualified path resolution. Detects `self.method()` calls for impl-type-aware ref resolution. Enum variants are indexed as `EnumVariant` symbols with `impl_type` set to the parent enum name.
- `dependencies.rs` — Parses `Cargo.toml`/`Cargo.lock`, resolves direct vs transitive deps.
- `store.rs` — Writes parsed symbols/deps to DB.
- `docs.rs` — Fetches docs (cargo doc JSON → docs.rs → GitHub README). Two-tier storage: crate summary + per-module detail.
- `cargo_doc.rs` — Parses nightly rustdoc JSON into structured per-module docs.

### `server` — MCP server (`src/server/`)
- `mod.rs` — `IlluServer` wraps `Arc<Mutex<Database>>` + `Option<Arc<RaClient>>`. Uses rmcp's `#[tool_router]`, `#[tool_handler]`, `#[tool]` macros with `Parameters<T>` wrapper. Tool param structs derive `JsonSchema` via `rmcp::schemars` re-export. The 13 `ra_*` tool handlers are inline in `mod.rs` (they delegate to `RaClient` methods, no DB access needed).
- `tools/` — Each core tool handler is a pure function `handle_*(db, ...) -> Result<String>`: `query.rs`, `context.rs`, `batch_context.rs`, `impact.rs`, `diff_impact.rs`, `callpath.rs`, `unused.rs`, `freshness.rs`, `docs.rs`, `overview.rs`, `tree.rs`, `crate_graph.rs`, `implements.rs`, `neighborhood.rs`, `type_usage.rs`, `file_graph.rs`, `symbols_at.rs`, `stats.rs`, `hotspots.rs`, `rename_plan.rs`, `similar.rs`, `boundary.rs`, `health.rs`, `blame.rs`.

### `ra` — rust-analyzer LSP client (`src/ra/`)
Optional at runtime — if rust-analyzer is not installed or `--no-ra` is passed, core tools work normally and RA tools return "not available".
- `client.rs` — `RaClient`: spawns rust-analyzer child process, LSP initialize/shutdown, readiness polling via progress tokens. `kill_on_drop(true)` ensures cleanup.
- `transport.rs` — `ServerState` + `build_router()`: async-lsp notification router handling `Progress`, `PublishDiagnostics`, `ShowMessage`. Tracks indexing readiness via progress token matching.
- `document.rs` — `DocumentTracker`: manages `didOpen`/`didChange` for files accessed via LSP. Version-tracked to prevent stale state.
- `retry.rs` — `with_retry()`: exponential backoff (100ms → 1.6s, 5 retries) on `CONTENT_MODIFIED` LSP errors.
- `lsp.rs` — Typed 1:1 wrappers around LSP methods: definition, implementation, references, hover, document/workspace symbols, call hierarchy (prepare/incoming/outgoing), type hierarchy (prepare/supertypes/subtypes), prepare rename, rename, code actions, resolve code action, diagnostics.
- `extensions.rs` — rust-analyzer custom requests (non-standard LSP): `ExpandMacro`, `Ssr`, `RelatedTests`, `SyntaxTree`. Each defines request type + params + impl on `RaClient`.
- `ops.rs` — Composed operations: `symbol_context()` fans out hover/refs/calls/impls/tests in sequence. `rename_preview()` + `safe_rename()` with workspace edit application. `apply_workspace_edit()` handles both `changes` and `documentChanges` formats, applies edits bottom-to-top to preserve positions.
- `types.rs` — `PositionSpec` (parses `file:line:col`), `RichLocation`, `LspSymbolInfo`, `CallInfo`, `SymbolContext`, `RenameImpact`, `RenameResult`, `DiagnosticInfo`. Includes `url_to_path_string()` and `symbol_kind_name()` converters.
- `error.rs` — `RaError` enum with `thiserror`: `NotAvailable`, `InitializationFailed`, `Timeout`, `RequestFailed`, `ContentModified`, `FileNotFound`, `InvalidPosition`, `SymbolNotFound`, `ServerShutdown`, `Io`.

## Key Patterns

- **`Database` is not `Sync`** — `rusqlite::Connection` requires `Mutex` wrapping for the MCP server's async context.
- **`rmcp::schemars`** — Tool param structs must use the `schemars` re-exported by rmcp, not a separate schemars crate.
- **Symbol refs use qualified resolution** — `extract_refs` builds an import map from `use` declarations and a crate map from the workspace. Refs resolve via import → same-file → same-crate → global name fallback. `self.method()` resolves via `impl_type` matching.
- **Workspace detection** — Presence of `[workspace]` in root `Cargo.toml` triggers multi-crate indexing. Single-crate repos get one implicit row in `crates`.
- **Impact tool** — Uses recursive CTE on `symbol_refs` (depth limit 5) for symbol-level impact. Supports `Type::method` syntax (e.g. `Database::open`) — splits into name + impl_type for CTE seed. For workspaces with >1 crate, prepends an "Affected Crates" section using `crate_deps` transitive query. Appends a "Related Tests" section listing `#[test]` functions that transitively call the symbol.
- **Context tool** — Shows callers ("Called By" section) alongside callees. Supports `Type::method` syntax (e.g. `Database::new`) via `impl_type` column lookup, and optional `file` parameter for scoped results.
- **Diff impact tool** — After listing changed symbols and downstream impact, appends a "Related Tests" section with a suggested `cargo test` command.
- **Callpath tool** — BFS on `(name, file)` pairs using `get_callees` (file-specific) to avoid ambiguity when multiple symbols share the same name. Returns shortest call chain with file locations.
- **Batch context tool** — Iterates over a list of symbol names, calling `handle_context` for each. Returns concatenated results.
- **Unused tool** — LEFT JOIN `symbol_refs` to find symbols with zero incoming refs. Excludes entry points (`main`, `#[test]`), `use`/`mod`/`impl` kinds, and `EnumVariant` symbols.
- **Freshness tool** — Compares `get_commit_hash` against `git rev-parse HEAD` and `get_index_version` against `INDEX_VERSION` (from `CARGO_PKG_VERSION`). Lists changed files via `git diff --name-only`. Does NOT call `refresh()` — reports current state only. Shows both index and binary versions.
- **Version-aware refresh** — `refresh_index` checks stored `index_version` against `INDEX_VERSION`. On mismatch, triggers full re-index instead of incremental refresh. Prevents stale data from older binary versions.
- **Crate graph tool** — Formats `crate_deps` as an adjacency list. Identifies root crates (no dependents) and leaf crates (no deps).
- **Query filters** — `attribute`, `signature`, `kind`, and `path` filters are all combinable. The broadest filter is used for the initial DB query, then remaining filters are applied as `.retain()` post-filters. `doc_comments` scope searches doc comment content. Default scope is `symbols` (not `all`) — use `scope: "all"` or `scope: "docs"` to include dependency docs. Wildcard query (`*` or empty string) with filters searches by filter only (e.g. `query: "*", signature: "-> Vec"` finds all functions returning Vec).
- **Context sections** — Optional `sections` parameter controls which sections render: `source`, `callers`, `callees`, `tested_by`, `traits`, `docs`. Omit for all sections. Header always renders.
- **Implements tool** — Uses `trait_impls` table to query trait/type relationships bidirectionally.
- **Neighborhood tool** — Bidirectional BFS using `get_callees_by_name` (downstream) and `get_callers_by_name` (upstream) within N hops. Filters to `ref_kind = 'call'` only — excludes type refs and enum variants for clean call graphs. List format shows file paths alongside symbol names.
- **Callpath all_paths** — When `all_paths=true`, uses DFS with backtracking to find up to `max_paths` paths (default 5).
- **Diff impact changes_only** — When `changes_only=true`, skips downstream impact and test coverage, returns only changed symbols. Output is capped at ~8000 chars with truncation summary to prevent token overflow.
- **Type usage tool** — Best-effort text search on `signature` and `details` columns to find where a type is used as params, returns, and struct fields.
- **File graph tool** — Derives file-level dependencies from `symbol_refs` table (no new tables needed). If symbol A in file X references symbol B in file Y, X depends on Y.
- **Symbols_at tool** — Wraps existing `get_symbols_at_lines` DB method for file:line lookup. Returns all symbols whose line range contains the given line.
- **Query bodies scope** — `scope: "bodies"` searches within function body text via LIKE on the `body` column. Also supports `limit` parameter to cap result count.
- **Query/overview limit** — Optional `limit` parameter truncates results. Overview distributes limit across files breadth-first; when limit < file count, drops files entirely to respect the limit. Shows "(limited to N, M total)" when truncated.
- **Context callers_path** — Optional `callers_path` parameter filters callers and callees to a path prefix (e.g. `"src/"` to exclude test callers).
- **Context related section** — Shows sibling symbols in the same file with matching `impl_type`. Labels as "Related (impl X)" for methods or "Related (same file)" for top-level. Capped at 10 items with "(N more)" overflow note.
- **Stats tool** — Aggregates file/symbol counts, kind breakdown, test coverage ratio, most-referenced symbols, and largest files into a compact dashboard.
- **Hotspots tool** — Identifies high-risk symbols: most-referenced (fragile to change), most-referencing (high complexity), and largest functions (by line span).
- **Rename_plan tool** — Unified preview of all locations referencing a symbol: definition, call sites, type usage in signatures, struct fields, trait implementations, and doc comments.
- **Similar tool** — Finds symbols with similar signatures and callee patterns. Scores by return type match, shared parameter types, and shared callees.
- **Full signature capture** — Parser captures complete multi-line signatures (up to `{`), collapsed to single line. No more first-line truncation.
- **Ref confidence scoring** — `symbol_refs` has a `confidence` column: `high` for impl/file-qualified resolution, `low` for name-only fallback. `file_graph` uses high-confidence only by default.
- **Caller/callee line numbers** — Context callers and callees now show `name (file:line)` for direct navigation.
- **Neighborhood tree format** — `format: "tree"` + `direction: "down"/"up"` renders hierarchical call tree with `├──`/`└──` characters.
- **Body search snippets** — `scope: "bodies"` results now show the first matching line from the body as a snippet.
- **Boundary tool** — Classifies symbols as "Public API" (called from outside path) or "Internal Only" (safe to refactor). Public API shows summarized usage counts (e.g. "5 site(s) across 3 files") instead of listing every call site.
- **Health tool** — Reports ref confidence distribution, signature quality, noise sources, and coverage metrics.
- **Blame tool** — Runs `git blame` on a symbol's line range, summarizes author, date, and commit message.
- **Constructor tracking** — `new`, `from`, `into`, `clone`, `default`, `build`, `init` are tracked as symbol refs (removed from `NOISY_SYMBOL_NAMES`). `impl_type` disambiguation prevents cross-type collisions.
- **References tool** — Unified view of all references to a symbol: definition, call sites, type usage in signatures, trait implementations. Consolidates data from callers, type_usage, and implements.
- **Doc coverage tool** — Finds symbols missing doc comments. Shows coverage percentage and lists undocumented symbols grouped by file.
- **Test impact tool** — Shows which tests break when changing a symbol. Combines impact analysis with test discovery, returns suggested `cargo test` command.
- **Orphaned tool** — Finds symbols with no callers AND no test coverage (intersection of unused + untested). These are safe to remove.
- **Graph export tool** — Exports call graphs or file dependency graphs in DOT/Graphviz format. Provide `symbol_name` for call graph or `path` for file graph.
- **Crate impact tool** — Shows which workspace crates are affected by changing a symbol. Bridges symbol-level impact with crate-level dependencies.
- **is_test column** — Symbols table has `is_test` column (boolean), set at index time via `is_test_attribute()` helper in `store.rs`. Matches `test`, `*::test` (e.g. `tokio::test`), `test(...)`, `rstest`, `test_case(...)` as complete attribute tokens — NOT substring matching. Rejects `tool(name = "test_impact", ...)`. Used by `get_related_tests()` for efficient test lookups. `get_related_tests` accepts optional `impl_type` for `Type::method` resolution.
- **Symbol resolution priority** — `resolve_symbol` tries: (1) `Type::method` qualified lookup, (2) exact name match, (3) FTS/fuzzy search. Exact match prevents noise from partial matches.
- **Stats confidence filter** — `handle_stats` most-referenced uses `confidence = "high"` to avoid inflated counts from name-only fallback refs. Shows qualified `ImplType::name` format.
- **LIKE escape** — All path-based LIKE queries use `escape_like()` + `ESCAPE '\\'` clause to handle paths with `%` or `_`.
- **Qualified caller/callee names** — Context callers/callees show `ImplType::method` format when impl_type is available, preventing ambiguity.
- **Macro body ref extraction** — `collect_body_refs()` descends into `macro_invocation` token trees to extract potential symbol references.
- **Module body ref extraction** — `collect_refs()` recurses into `mod_item` nodes to extract refs from functions inside modules (e.g. `#[cfg(test)] mod tests { ... }`).
- **Calls-only graph traversal** — `get_callees_by_name` and `get_callers_by_name` filter to `sr.kind = 'call'` and exclude `Const`/`Static`/`EnumVariant` target kinds, keeping call graphs focused on function calls.
- **Cargo test cap** — When >20 tests are related, impact/diff_impact/test_impact suggest `cargo test` without filter names instead of an unusably long command.
- **FTS name-only for short queries** — `search_symbols` uses FTS column filter `name:"query"*` for queries <= 5 chars to prevent doc_comment noise. Longer queries still search across all FTS columns.
- **Ref line tracking** — `symbol_refs` has a `ref_line` column storing the 1-based line where each reference occurs (captured from tree-sitter node positions). Callers in `context` and `references` show the call-site line, not the calling function's definition line. `CalleeInfo.ref_line` is `Option<i64>`; display falls back to `line_start` when NULL.
- **Exclude tests filter** — `context`, `neighborhood`, and `callpath` accept `exclude_tests: bool` parameter. When true, test functions (`is_test = 1`) are filtered from callers/callees, keeping graph output focused on production code.
- **Type usage compact mode** — `type_usage` accepts `compact: bool` parameter. When true, groups results by file with counts instead of listing every entry with full signatures.
- **History show_diff** — `history` accepts `show_diff: bool` parameter. When true, uses `git log -L<start>,<end>:<file>` to show function-level code diffs per commit. Output capped at ~4000 chars.
- **Overview filters mod/use** — `handle_overview` excludes `Mod` and `Use` symbol kinds from output to focus on actual API surface (functions, structs, enums, traits).
- **Wildcard + kind filter** — `query: "*", kind: "struct"` (wildcard with kind-only filter) now seeds results via `get_symbols_by_path_prefix("")` instead of returning empty. All kind-only wildcard queries work without needing a path filter.
- **Relevance-ranked results** — `format_symbols` sorts results by high-confidence incoming reference count (descending), then by name. Most-referenced symbols appear first. Uses `Database::count_refs_for_symbol` per result (max 50 queries, trivially fast).
- **Similar noise filtering** — `score_one` excludes `NOISY_SIMILAR_CALLEES` (`new`, `from`, `into`, `default`, `clone`, `build`, `init`, `fmt`, `write`, `writeln`, `push`, `len`, `is_empty`, `to_string`, `to_owned`, `as_str`, `as_ref`, `iter`, `collect`, `map`, `filter`) from shared callee scoring to prevent ubiquitous constructors/iterators from inflating similarity scores.
- **Similar param matching** — `score_one` scores ALL matching parameter types (not just the first), differentiates `&mut self` (score 2) from `&self` (score 1), and also searches candidates by parameter types (not just return type). Minimum type length is 3 chars.
- **Type usage/rename_plan Use filter** — `handle_type_usage` and `write_signature_usage` in rename_plan filter out `SymbolKind::Use` symbols to prevent `use` import statements from appearing as type usage in signatures.
- **Docs topic LIKE fallback** — `handle_docs_with_topic` falls back to `Database::search_docs_content` (LIKE-based content search) when FTS search returns no results. Catches terms FTS tokenization can't handle (e.g. "FTS5").
- **Crate-path ref resolution** — `collect_body_refs` intercepts `scoped_identifier` nodes starting with `crate::` (e.g. `crate::status::set(...)`, `crate::status::StatusGuard::new(...)`). Extracts the terminal name, bypasses the noisy-symbol filter (since qualified paths are unambiguous), resolves target_file from the module path, and detects uppercase penultimate segments as type context.
- **Scoped identifier type context** — Non-`crate::` scoped identifiers like `Database::new()` extract the type qualifier as `target_context`. Constructor names (`new`, `from`, `into`, `default`, `clone`, `build`, `init`) on unknown types (e.g. `Vec::new()`) are filtered out to prevent false positive callees. Complex paths (`a::b::c`) still descend into children.
- **Crate name normalization** — `crate_map` replaces hyphens with underscores in crate names (Cargo normalizes `foo-bar` → `foo_bar` for Rust module paths). Both `extract_all_symbol_refs` and `rebuild_refs_for_files` apply this. `qualified_path_to_files_with_crates` normalizes `"."` crate path to `""` to avoid `./src/` prefix mismatch.
- **Qualified calls bypass noisy filter** — `try_add` skips `is_noisy_symbol` check when `target_context` is present. `Status::clear()` is unambiguous even though bare `clear()` is noisy. Matches how `try_add_qualified` already works for `crate::` paths.
- **Seen dedup includes context** — `BodyRefCollector.seen` uses `"Type::name"` as key when `target_context` is present, so `Foo::new()` and `Bar::new()` in the same function are both captured instead of being deduplicated.
- **Wildcard query path-first seed** — `format_symbols` prefers `path` as the seed query over `signature` for wildcard queries. Prevents LIMIT 50 truncation in `search_symbols_by_signature` from discarding results before path filtering. Signature is always applied as a post-filter when present.
- **Callers cap in context** — `render_callers` caps the caller list at 30 entries. Shows overflow note "(N more — use `references` for the full list)" when truncated.
- **Impact test deduplication** — `handle_impact` tracks symbol names that appear in depth entries and excludes them from the "Related Tests" section. Prevents massive duplication for utility functions. The `cargo test` suggestion still includes all test names.
- **Callees confidence filter** — `get_callees` (used by context/batch_context) filters to `confidence = 'high'` only, matching `file_graph` behavior. Eliminates low-confidence name-only fallback noise from callee lists.
- **Callers confidence filter** — `get_callers` also filters to `confidence = 'high'` only. Prevents false positives where `HashMap::insert` callers are misattributed to `FileTree::insert`.
- **Impact/test CTE confidence filter** — `impact_dependents_with_depth` and `get_related_tests` recursive CTEs filter `sr.confidence = 'high'` in the recursive step. Prevents common-name false positives from inflating impact and test counts.
- **Composite confidence indexes** — `idx_symbol_refs_target` and `idx_symbol_refs_source` are composite indexes on `(target_symbol_id, confidence)` and `(source_symbol_id, confidence)` respectively.
- **Rename_plan dedup** — `write_call_sites` deduplicates callers across multiple symbol definitions (struct + impl) using `(name, file)` HashSet.
- **symbol_exists Type::method** — `symbol_exists` and `get_direct_callees` support `Type::method` syntax (e.g., `HcfsClient::upload`) by splitting into name + impl_type.
- **Type usage word boundary** — `contains_whole_word` post-filter prevents `SyncPlan` from matching `SyncPlanConflict`. Applied to both `handle_type_usage` (signatures and details) and `write_signature_usage` in rename_plan.
- **Constants excluded from call graphs** — `get_callees_by_name` and `get_callers_by_name` filter out `Const` and `Static` symbol kinds. Constants are value references, not function calls.
- **Batch context sections** — `batch_context` accepts optional `sections` parameter (same values as `context`: `source`, `callers`, `callees`, `tested_by`, `traits`, `related`, `docs`). Reduces output verbosity when only specific sections are needed.
- **Production-first callers** — `render_callers` sorts non-test callers before test callers using `CalleeInfo.is_test`. Blank line separator between production and test callers.
- **Docs topic fallback summary** — When docs topic search fails (FTS + LIKE), the response includes a truncated (500 char) crate summary excerpt so the user gets useful context instead of a dead-end error.
- **References dedup** — `handle_references` deduplicates call sites across multiple definition rows (e.g. enum + impl blocks) using `(name, file, line)` HashSet. Filters `Use` kind from type usage section. Separates production callers before test callers with blank line.
- **Diff impact cross-seed dedup** — `render_downstream_impact` tracks `(name, file, depth)` across all seed symbols, skipping entries already shown by a previous seed. Eliminates duplicate downstream entries when multiple changed symbols share dependents.
- **Impact exclude_tests** — `handle_impact` accepts `exclude_tests: bool`. When true, filters `is_test=true` entries from depth results (Related Tests section unaffected). `ImpactEntry` now includes `is_test: bool` from the CTE.
- **Hotspots/stats exclude_tests** — Both accept `exclude_tests: bool`. When true, `get_most_referenced_symbols_filtered` excludes refs where the source symbol is a test function (`ss.is_test = 0`), showing only production-code reference counts.
- **Impact summary thresholds** — `SUMMARY_DEPTH=2`, `SUMMARY_THRESHOLD=5` (was 3/10). Depths >= 2 with > 5 entries are summarized by file instead of listed individually.
- **Docs case-insensitive LIKE** — `search_docs_content` uses `lower(d.content) LIKE lower(query)` for case-insensitive topic matching in the LIKE fallback path.
- **Diff impact compact** — `handle_diff_impact` accepts `compact: bool`. When true, skips downstream impact analysis but still shows untested changes and related tests. Sweet spot between `changes_only` (too minimal) and full mode (too verbose for large diffs).
- **Hotspots exclude_tests on largest** — `get_largest_functions` accepts `exclude_tests: bool`. When true, filters `is_test = 0` so test functions don't dominate the "Largest Functions" section.
- **References exclude_tests** — `handle_references` accepts `exclude_tests: bool`. When true, test callers are excluded from the call sites section entirely (not just separated). Summary count reflects production callers only.
- **Test list tiering** — `render_test_list` in `mod.rs` controls output size for Related Tests sections across `impact`, `diff_impact`, and `test_impact`. Three tiers: ≤20 tests listed individually, 21–50 grouped by file with names, >50 file-grouped summary with counts only. Thresholds: `TEST_LIST_GROUP_THRESHOLD=20`, `TEST_LIST_SUMMARY_THRESHOLD=50`.
- **Registry** — Auto-populated at `~/.illu/registry.toml` on every `illu serve` startup. Tracks repo name, path, git remote, and last indexed timestamp. Worktrees dedup by `git_common_dir`.
- **Cross-repo tools** — Open other repos' `.illu/index.db` read-only on demand via `Database::open_readonly`. Name-based matching across repos (no shared index). `cross_query`, `cross_impact`, `cross_deps`, `cross_callpath` all use the registry to find other repos.
- **Global install** — `illu install` writes MCP config + instruction sections to `~/.claude/` and `~/.gemini/` globally, installs statusline to `~/.illu/statusline.sh`, and configures `statusLine` in Claude settings (skips if already configured). Uses CWD auto-detection (no `--repo` flag). `illu init` remains for per-repo overrides.
- **Auto-detection** — `illu serve` without `--repo` detects repo root via `git rev-parse --show-toplevel`. Works with git worktrees — each worktree gets its own index.
- **repos tool** — Dashboard showing all registered repos with status (active/indexed/missing/no index), symbol counts.
- **cross_query tool** — Searches symbols across all registered repos except the primary. Same params as `query`, results grouped by repo.
- **cross_impact tool** — Name-based impact search across other repos' `symbol_refs`. Shows which code in other repos references the given symbol.
- **cross_deps tool** — Scans `Cargo.toml` across repos for path dependencies (direct source links) and shared crate dependencies.
- **cross_callpath tool** — Finds bridge symbols between repos: callees of `from` in primary that also exist in target repo.
- **Refresh detects committed changes** — `refresh_index` compares stored commit hash to HEAD. If they differ, runs `git diff --name-only <stored>..HEAD` to find committed .rs changes and merges them into the candidate list from `git status`. Always updates metadata (commit hash) after refresh, even when no .rs files changed.
- **"Did you mean?" suggestions** — `symbol_not_found(db, name)` runs FTS fuzzy search and suggests top 3 matches. For `Type::method` names, also searches the method part alone. All tool handlers pass `db` to `symbol_not_found`.
- **Boundary uses all confidence levels** — `get_callers` accepts `min_confidence: Option<&str>`. `handle_boundary` passes `None` (all confidences) for inclusive external-caller detection. `render_callers` (context) and `render_call_sites` (references) pass `Some("high")` for precision.
- **Hotspots exclude_tests on all sections** — `get_most_referencing_symbols` accepts `exclude_tests: bool`. When true, filters `ss.is_test = 0` so test functions don't dominate "Most Referencing".
- **Levenshtein fallback** — `symbol_not_found` falls back to edit-distance matching when FTS/trigram returns nothing. Scans all distinct symbol names, filters by threshold (40% of query length, min 2 edits), returns top 3. Case-insensitive.
- **Signature suffix matching** — `signature_matches` checks if filter type words appear as suffixes in signature type words. `-> Result` matches `-> SqlResult<Self>` because `SqlResult` ends with `Result`. Minimum word length: 3 chars.
- **Overview external callers** — `render_external_callers` shows top 3 callers from outside the symbol's file. Shows overflow count. Complements `render_same_file_callees`.
- **Diff impact filters structural noise** — `render_diff_output` skips `Mod` and `Impl` kinds from changed symbol listings. Downstream impact and test coverage still see all symbols.
- **String literal search** — `scope: "strings"` searches within quoted string literals in function bodies. Post-filters body search results by extracting `"..."` content. Shows matching string literal as snippet. Known limitation: raw strings (`r"..."`, `r#"..."#`) not handled.
- **Smart body truncation** — `extract_body` preserves first 50 lines + last 10 lines with `// ... N lines omitted ...` marker. Previous format: first 100 lines + `// ... truncated`.
- **Graph export formats** — `handle_graph_export` accepts `format`: `"dot"` (default Graphviz), `"edges"` (compact `A -> B` lines for AI), `"summary"` (node/edge counts, roots, leaves).
- **RA tools use `file:line:col` positions** — All `ra_*` tools take positions as `"file:line:col"` strings (1-indexed), parsed by `PositionSpec::from_str`. This differs from core tools which take symbol names.
- **RA lifecycle** — `RaClient::start()` spawns rust-analyzer, sends LSP `initialize`, waits for `initialized`. `wait_for_ready()` polls progress tokens until indexing completes (timeout 120s). Background task in `main.rs` handles readiness polling.
- **RA readiness gate** — `require_ra_ready()` checks `is_ready()` before write operations (`ra_rename`, `ra_safe_rename`, `ra_ssr`). Read-only tools (`ra_hover`, `ra_definition`, etc.) use `ra()` which only checks availability, not readiness — partial results during indexing are acceptable for reads.
- **RA tools are independent of DB** — `ra_*` tools don't acquire the `Mutex<Database>` lock. No deadlock risk between RA and core tools.
- **RA + core tools coexist** — Both tool sets are always registered. If RA is unavailable, `ra_*` tools return "rust-analyzer not available" error. Core tools always work.
- **`apply_workspace_edit` handles `\r\n`** — Detects line ending style from file content and uses correct byte offsets for text edits. Edits applied bottom-to-top to preserve positions.

## Lint Configuration

Rust 2024 edition with strict clippy (see `Cargo.toml [lints.clippy]`):
- `unwrap_used = "deny"` — use `?`, `unwrap_or`, or `let...else`
- `print_stdout/print_stderr = "deny"` — use `tracing` macros
- `panic/todo/unimplemented = "deny"`
- `allow_attributes = "deny"` — use `#[expect(lint, reason = "...")]` instead
- Tests opt out via `#[expect(clippy::unwrap_used, reason = "tests")]` on the test module

<!-- illu:start -->
## Code Intelligence (illu)

This repo is indexed by illu (49 tools: 36 core + 13 rust-analyzer). **Use illu tools as your first step** — before reading files, before grep, before guessing at code structure.

### When to use illu

- **Starting any task**: `illu query` the relevant symbols to understand what exists
- **Before modifying a function/struct/trait**: `illu impact` to see what depends on it
- **Debugging or tracing issues**: `illu context` to get the full definition and references
- **Understanding call flow**: `illu neighborhood` or `illu callpath` to explore the call graph
- **Before refactoring a module**: `illu boundary` to see what's public API vs internal
- **Using an external crate**: `illu docs` to check how it's used in this project
- **Before reading files**: query first — illu tells you exactly where things are
- **Finding which tests to run**: `illu test_impact` after changing a symbol
- **Dead code detection**: `illu unused` or `illu orphaned` to find unreferenced symbols
- **Index health**: `illu freshness` to check if the index is current
- **Cross-repo analysis**: `illu cross_query` to find symbols in other repos, `illu cross_impact` to check cross-repo effects
- **Repo overview**: `illu repos` to see all registered repos
- **Compiler-accurate definition**: `illu ra_definition` when core `context` can't resolve through macros or generics
- **Type info at a position**: `illu ra_hover` for full compiler-resolved type signatures and docs
- **Compilation errors**: `illu ra_diagnostics` to see real errors, not just parse issues
- **Safe refactoring**: `illu ra_safe_rename` for compiler-verified rename across the workspace
- **Macro debugging**: `illu ra_expand_macro` to see what a macro generates
- **Pattern refactoring**: `illu ra_ssr` for syntax-aware search and replace

### Commands

| User types | MCP tool | Params |
|------------|----------|--------|
| `illu query <term>` | `mcp__illu__query` | `query: "<term>"` |
| `illu query <term> --scope <s>` | `mcp__illu__query` | `query: "<term>", scope: "<s>"` |
| `illu query * --kind struct` | `mcp__illu__query` | `query: "*", kind: "struct"` |
| `illu query * --sig "-> Result"` | `mcp__illu__query` | `query: "*", signature: "-> Result"` |
| `illu context <symbol>` | `mcp__illu__context` | `symbol_name: "<symbol>"` |
| `illu context Type::method` | `mcp__illu__context` | `symbol_name: "Type::method"` |
| `illu context <sym> --sections source,callers` | `mcp__illu__context` | `symbol_name: "<sym>", sections: ["source", "callers"]` |
| `illu context <sym> --exclude-tests` | `mcp__illu__context` | `symbol_name: "<sym>", exclude_tests: true` |
| `illu batch_context <sym1> <sym2>` | `mcp__illu__batch_context` | `symbols: ["<sym1>", "<sym2>"]` |
| `illu impact <symbol>` | `mcp__illu__impact` | `symbol_name: "<symbol>"` |
| `illu impact <symbol> --depth 1` | `mcp__illu__impact` | `symbol_name: "<symbol>", depth: 1` |
| `illu diff_impact` | `mcp__illu__diff_impact` | *(unstaged changes)* |
| `illu diff_impact main` | `mcp__illu__diff_impact` | `git_ref: "main"` |
| `illu test_impact <symbol>` | `mcp__illu__test_impact` | `symbol_name: "<symbol>"` |
| `illu callpath <from> <to>` | `mcp__illu__callpath` | `from: "<from>", to: "<to>"` |
| `illu neighborhood <symbol>` | `mcp__illu__neighborhood` | `symbol_name: "<symbol>"` |
| `illu neighborhood <sym> --format tree` | `mcp__illu__neighborhood` | `symbol_name: "<sym>", format: "tree"` |
| `illu references <symbol>` | `mcp__illu__references` | `symbol_name: "<symbol>"` |
| `illu boundary src/server/` | `mcp__illu__boundary` | `path: "src/server/"` |
| `illu unused` | `mcp__illu__unused` | |
| `illu unused --path src/server/` | `mcp__illu__unused` | `path: "src/server/"` |
| `illu orphaned` | `mcp__illu__orphaned` | |
| `illu overview src/` | `mcp__illu__overview` | `path: "src/"` |
| `illu stats` | `mcp__illu__stats` | |
| `illu hotspots` | `mcp__illu__hotspots` | |
| `illu implements --trait Display` | `mcp__illu__implements` | `trait_name: "Display"` |
| `illu docs <dep>` | `mcp__illu__docs` | `dependency: "<dep>"` |
| `illu docs <dep> --topic <t>` | `mcp__illu__docs` | `dependency: "<dep>", topic: "<t>"` |
| `illu freshness` | `mcp__illu__freshness` | |
| `illu crate_graph` | `mcp__illu__crate_graph` | |
| `illu blame <symbol>` | `mcp__illu__blame` | `symbol_name: "<symbol>"` |
| `illu history <symbol>` | `mcp__illu__history` | `symbol_name: "<symbol>"` |
| `illu repos` | `mcp__illu__repos` | |
| `illu cross_query <term>` | `mcp__illu__cross_query` | `query: "<term>"` |
| `illu cross_impact <symbol>` | `mcp__illu__cross_impact` | `symbol_name: "<symbol>"` |
| `illu cross_deps` | `mcp__illu__cross_deps` | |
| `illu cross_callpath <from> <to>` | `mcp__illu__cross_callpath` | `from: "<from>", to: "<to>"` |
| **rust-analyzer tools** (require RA running) | | |
| `illu ra_definition <pos>` | `mcp__illu__ra_definition` | `position: "<file:line:col>"` |
| `illu ra_hover <pos>` | `mcp__illu__ra_hover` | `position: "<file:line:col>"` |
| `illu ra_diagnostics` | `mcp__illu__ra_diagnostics` | `file: "<file>"` (optional) |
| `illu ra_call_hierarchy <pos>` | `mcp__illu__ra_call_hierarchy` | `position: "<file:line:col>", direction: "both"` |
| `illu ra_type_hierarchy <pos>` | `mcp__illu__ra_type_hierarchy` | `position: "<file:line:col>"` |
| `illu ra_rename <pos> <name>` | `mcp__illu__ra_rename` | `position: "<file:line:col>", new_name: "<name>"` |
| `illu ra_safe_rename <pos> <name>` | `mcp__illu__ra_safe_rename` | `position: "<file:line:col>", new_name: "<name>"` |
| `illu ra_code_actions <pos>` | `mcp__illu__ra_code_actions` | `position: "<file:line:col>", kind: "refactor"` |
| `illu ra_expand_macro <pos>` | `mcp__illu__ra_expand_macro` | `position: "<file:line:col>"` |
| `illu ra_ssr <pattern>` | `mcp__illu__ra_ssr` | `pattern: "foo($a) ==>> bar($a)"` |
| `illu ra_context <pos>` | `mcp__illu__ra_context` | `position: "<file:line:col>"` |
| `illu ra_syntax_tree <file>` | `mcp__illu__ra_syntax_tree` | `file: "<file>"` |
| `illu ra_related_tests <pos>` | `mcp__illu__ra_related_tests` | `position: "<file:line:col>"` |

### Workflow rules

1. **Locate before you read**: `illu query` or `illu context` to find the right file:line, then Read only what you need
2. **Impact before you change**: always run `illu impact` before modifying any public symbol
3. **Chain tools**: `illu query` to find candidates → `illu context` for the one you need → `illu impact` before changing it
4. **Save tokens**: use `sections: ["source", "callers"]` on context/batch_context to fetch only what you need
5. **Production focus**: use `exclude_tests: true` on context/neighborhood/callpath to filter out test functions
<!-- illu:end -->
