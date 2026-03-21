<p align="center">
  <img src="docs/assets/banner.svg" alt="illu — Code intelligence for AI agents" width="400"/>
</p>

<p align="center">
  <strong>Give your AI agent a semantic understanding of your Rust codebase.</strong>
</p>

<p align="center">
  <a href="#works-with"><img src="https://img.shields.io/badge/Claude_Code-5A29E4?style=for-the-badge&logo=anthropic&logoColor=white" alt="Claude Code"/></a>
  <a href="#works-with"><img src="https://img.shields.io/badge/Gemini_CLI-4285F4?style=for-the-badge&logo=google&logoColor=white" alt="Gemini CLI"/></a>
  <a href="https://modelcontextprotocol.io"><img src="https://img.shields.io/badge/MCP-stdio-818cf8?style=for-the-badge" alt="MCP"/></a>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="MIT License"/></a>
  <img src="https://img.shields.io/badge/rust-2024_edition-dea584?style=flat-square&logo=rust" alt="Rust 2024"/>
  <img src="https://img.shields.io/badge/tools-31-blue?style=flat-square" alt="31 tools"/>
  <img src="https://img.shields.io/badge/tests-528_passing-brightgreen?style=flat-square" alt="528 tests"/>
</p>

---

## Get Started

Install and set up in your Rust project:

```bash
# Install (works on macOS and Linux)
git clone https://github.com/GeorgiosDelkos/illu-rs.git
cargo install --path illu-rs

# Set up in your Rust project
cd your-project
illu-rs init
```

That's it. Open **Claude Code** or **Gemini CLI** in the repo — illu is already running.

`init` indexes your codebase, writes the MCP config for both clients, and adds usage instructions to `CLAUDE.md` and `GEMINI.md`. Every time the server starts, it detects changed files and re-indexes only what's needed (sub-second).

> **Requirements:** Rust toolchain and a C compiler (Xcode CLI tools on macOS, `build-essential` on Linux). All C dependencies (SQLite, tree-sitter) are compiled from source — no system libraries needed.

<details>
<summary>Manual setup (without <code>init</code>)</summary>

Add to `.mcp.json` (Claude Code) or `.gemini/settings.json` (Gemini CLI):

```json
{
  "mcpServers": {
    "illu": {
      "command": "/path/to/illu-rs",
      "args": ["--repo", "/path/to/your/project", "serve"],
      "env": { "RUST_LOG": "warn" }
    }
  }
}
```

</details>

## What Your AI Gets

illu gives your AI agent **31 tools** through the [Model Context Protocol](https://modelcontextprotocol.io/), organized into five categories:

### Search and Navigate

#### Find symbols instantly — `query`

Instead of grepping, the AI searches an indexed database with full-text search, exact-match priority, and relevance ranking by reference count.

```
query: "Config"                                         → symbols + docs matching Config
query: "Config", scope: "symbols", kind: "struct"       → just the struct
query: "Color", scope: "symbols", kind: "enum_variant"  → enum variants of Color
query: "*", attribute: "test"                            → all #[test] functions
query: "*", signature: "-> Result<String"                → functions returning Result<String>
query: "parse", scope: "bodies"                          → search inside function bodies
query: "todo", scope: "doc_comments"                     → search doc comment content
```

#### Understand a symbol completely — `context`

One call returns everything: signature, doc comments, source body, struct fields, trait impls, callers, callees, related symbols, and dependency docs. No need to read the whole file.

```
symbol_name: "Database"                                  → full definition + callers + callees
symbol_name: "Database::new"                             → only the new() method on Database
symbol_name: "Config", file: "src/db.rs"                 → Config in a specific file
symbol_name: "parse", sections: ["source", "callers"]    → only source and callers (saves tokens)
symbol_name: "handle_query", exclude_tests: true         → production callers only
symbol_name: "parse", callers_path: "src/"               → callers filtered to a path prefix
```

#### Get multiple symbols at once — `batch_context`

Fetch context for several symbols in one call. Supports the same `sections` filter as `context`.

```
symbols: ["Database", "handle_query", "parse_rust_source"]
symbols: ["Config", "Server"], sections: ["source", "callees"]
```

#### Look up symbols by file and line — `symbols_at`

Find which symbols exist at a given line in a file.

```
file: "src/db.rs", line: 114   → Database::open (function, lines 114-133)
```

#### See project structure — `overview` and `tree`

- **`overview`** — public symbols under a path, grouped by file, with signatures, doc snippets, and intra-file call relationships
- **`tree`** — file/module hierarchy with symbol counts per file

```
path: "src/server/"                        → public API in the server module
path: "src/db.rs", include_private: true   → everything in db.rs, including private fns
```

### Analyze Impact

#### Know what breaks before changing it — `impact`

Before modifying a symbol, the AI sees every function, struct, and crate that depends on it — with the dependency chain explained.

```
symbol_name: "Config"              → full transitive impact (default depth 5)
symbol_name: "Config", depth: 1    → direct callers only (flat list)
```

```markdown
## Impact Analysis: Config

### Affected Crates
- **core** (defined here)
- **api**
- **cli**

### Depth 1
- **parse_config** (src/lib.rs)

### Depth 2
- **run_server** (src/main.rs) — via parse_config

### Related Tests
- **test_parse_config** (tests/config.rs:42)
- **test_config_defaults** (tests/config.rs:58)

Suggested: `cargo test test_parse_config test_config_defaults`
```

#### See what a change breaks — `diff_impact`

Pass a git ref and get batch impact analysis for every symbol touched by the diff.

```
git_ref: "HEAD~3..HEAD"   → impact of last 3 commits
git_ref: "main"           → impact of current branch vs main
(omit)                    → impact of unstaged changes
changes_only: true        → just the changed symbols, skip downstream analysis
```

#### Find which tests cover a symbol — `test_impact`

Combines impact analysis with test discovery. Returns test names, locations, and a suggested `cargo test` command.

```
symbol_name: "Database::migrate"   → 204 tests affected, suggests cargo test (full suite)
symbol_name: "parse_rust_source"   → tests in parser.rs + integration tests
```

#### Cross-crate impact — `crate_impact`

For workspaces, shows which crates are affected by changing a symbol.

```
symbol_name: "CoreConfig"   → affected crates: api, cli, web
```

### Explore the Call Graph

#### Trace call chains — `callpath`

Find paths through the call graph between two symbols.

```
from: "main", to: "parse_rust_source"            → shortest path (2 hops)
from: "main", to: "extract_refs", all_paths: true → up to 5 distinct paths via DFS
from: "main", to: "migrate", exclude_tests: true  → skip test functions in paths
```

#### Explore a symbol's neighborhood — `neighborhood`

Bidirectional BFS showing callers and callees within N hops.

```
symbol_name: "extract_all_symbol_refs", depth: 2, direction: "down", format: "tree"
```

```
extract_all_symbol_refs
├── store_symbol_refs_fast
│   ├── insert_symbol_ref
│   └── resolve
├── extract_refs
│   ├── collect_refs
│   ├── extract_import_map
│   └── parse_source
└── get_all_file_paths
    ├── push
    └── query
```

#### Unified references — `references`

All references to a symbol in one view: definition, call sites, type usage in signatures, and trait implementations.

```
symbol_name: "resolve_symbol"   → 13 call sites, 1 signature usage, 0 trait impls
```

#### Find where a type is used — `type_usage`

Searches function signatures and struct fields for a type name.

```
type_name: "Database"                    → every function taking or returning Database
type_name: "Database", compact: true     → grouped by file with counts
```

#### File-level dependencies — `file_graph`

Derives which files depend on which based on cross-file symbol references.

```
path: "src/server/"   → file dependency edges within server module
```

#### Export as Graphviz — `graph_export`

Export call graphs or file dependency graphs in DOT format.

```
symbol_name: "handle_impact"   → DOT graph of impact's call tree
path: "src/indexer/"           → DOT graph of file dependencies
```

### Discover and Audit

#### Find dead code — `unused`

Detect symbols with no incoming references. Excludes entry points (`main`, `#[test]`).

```
(no params)                                → all unreferenced public symbols
path: "src/server/", kind: "function"      → unused functions in server module
untested: true                             → symbols with no test coverage
```

#### Find truly dead code — `orphaned`

Symbols with no callers AND no test coverage — safe to remove.

```
(no params)           → symbols that are both unused and untested
path: "src/indexer/"  → scoped to a module
```

#### Analyze module boundaries — `boundary`

Classifies symbols as "Public API" (called from outside path) or "Internal Only" (safe to refactor).

```
path: "src/indexer/"   → 13 public API symbols, 26 internal-only
```

#### Find similar functions — `similar`

Discovers functions with similar signatures and call patterns.

```
symbol_name: "handle_impact"   → export_symbol_graph (score: 4, shared return type + callee)
```

#### Preview rename impact — `rename_plan`

All locations that reference a symbol: definition, call sites, type usage, struct fields, trait impls, doc comments.

```
symbol_name: "resolve_symbol"   → 13 call sites + 1 signature usage to update
```

#### Check documentation coverage — `doc_coverage`

Find symbols missing doc comments, grouped by file with coverage percentage.

```
(no params)                              → full project doc coverage
path: "src/server/", kind: "function"    → undocumented functions in server
```

#### Identify hotspots — `hotspots`

High-risk symbols: most-referenced (fragile), most-referencing (complex), and largest functions.

```
(no params)                → top hotspots across the codebase
path: "src/db.rs"          → hotspots in a specific file
```

#### Codebase statistics — `stats`

File/symbol counts, kind breakdown, test coverage ratio, most-referenced symbols, largest files.

```
(no params)         → full codebase dashboard
path: "src/server/" → stats scoped to server module
```

### Dependencies and Docs

#### Get accurate dependency docs — `docs`

Look up documentation for your exact dependency versions (from `Cargo.lock`), so the AI never hallucinates API signatures.

```
dependency: "serde"                    → full API summary
dependency: "tokio", topic: "runtime"  → filtered by keyword
```

<details>
<summary>How docs are fetched</summary>

Three sources, tried in order:

1. **`cargo +nightly doc`** — parses rustdoc JSON locally. Structured, version-accurate, works offline.
2. **docs.rs** — fetches the HTML page for the exact version, extracts text.
3. **GitHub README** — discovers repo URL via crates.io, fetches raw README.

Results are cached in the database. Subsequent lookups are instant.

</details>

#### Visualize workspace structure — `crate_graph`

For multi-crate workspaces, shows the dependency graph between crates — which crate depends on which, plus root and leaf crates.

```
(no params)   → crate list + dependency arrows + root/leaf identification
```

#### Find trait implementations — `implements`

Query which types implement a trait, or which traits a type implements.

```
trait_name: "Display"   → all types implementing Display
type_name: "Database"   → all traits Database implements
```

### Git Integration

#### Blame a symbol — `blame`

Runs `git blame` on a symbol's line range, summarizes author, date, and commit message.

```
symbol_name: "Database::open"   → who wrote each line, when, and why
```

#### Symbol history — `history`

Git commit history for a specific symbol, with optional function-level diffs.

```
symbol_name: "handle_query"                  → commits that touched this function
symbol_name: "handle_query", show_diff: true → with code diffs per commit
```

### Index Management

#### Check index health — `freshness`

See whether the index is current or stale. Shows the indexed commit vs HEAD and lists any files that changed since last indexing.

#### Diagnose index quality — `health`

Reports ref confidence distribution, signature quality, noise sources, and coverage metrics.

## Works With

<table>
<tr>
<td width="50%" align="center">

### <img src="https://img.shields.io/badge/-5A29E4?style=flat-square&logo=anthropic&logoColor=white" height="20" align="center"/> &nbsp;Claude Code

Auto-configured via `.mcp.json` and `CLAUDE.md`

31 tools: `mcp__illu__query`, `mcp__illu__context`, `mcp__illu__impact`, etc.

</td>
<td width="50%" align="center">

### <img src="https://img.shields.io/badge/-4285F4?style=flat-square&logo=google&logoColor=white" height="20" align="center"/> &nbsp;Gemini CLI

Auto-configured via `.gemini/settings.json` and `GEMINI.md`

31 tools: `@illu query`, `@illu context`, `@illu impact`, etc.

</td>
</tr>
</table>

Any MCP client with stdio transport support works — illu speaks standard MCP.

## Features

| Feature | What it does |
|---------|-------------|
| **Zero-config setup** | `illu-rs init` configures everything for both Claude and Gemini |
| **Incremental indexing** | Content-hashed — only re-parses files that changed, cleans stale refs |
| **Workspace support** | Multi-crate workspaces with cross-crate reference resolution |
| **Full-text search** | FTS5 prefix matching + trigram-indexed substring search |
| **Qualified refs** | Import-map-aware resolution — `use crate::foo::Bar` resolves to the right file |
| **Method-level refs** | `self.method()` resolves to the correct impl type, not a global name match |
| **Confidence scoring** | Refs are tagged `high` or `low` confidence; call graphs use high-confidence only |
| **Qualified symbol lookup** | `Database::new` syntax disambiguates methods across types; optional `file` filter |
| **Sections filter** | Request only `source`, `callers`, `callees`, `tested_by`, `traits`, `related`, `docs` — saves tokens |
| **Exclude tests filter** | `exclude_tests: true` on context, neighborhood, callpath — focus on production code |
| **Callers + callees** | `context` shows both what a symbol calls and who calls it, with line numbers |
| **Production-first callers** | Non-test callers sorted before test callers with visual separator |
| **Trait impl tracking** | Maps which types implement which traits |
| **Enum variant indexing** | Each variant is a searchable symbol — `Color::Red` via qualified lookup |
| **Impact analysis** | Recursive CTE walks the reference graph with configurable depth (default 5) |
| **Symbol-to-test mapping** | `impact`, `diff_impact`, and `test_impact` discover tests that exercise changed symbols |
| **Diff-based impact** | `diff_impact` maps git changes to symbols, shows downstream effects + test suggestions |
| **Call path tracing** | Shortest path (BFS) or all paths (DFS with backtracking) between any two symbols |
| **Neighborhood exploration** | Bidirectional BFS with tree or list output format |
| **Batch context** | Fetch context for multiple symbols in one call with optional sections filter |
| **Dead code detection** | `unused` finds public symbols with zero incoming references |
| **Orphaned detection** | `orphaned` finds symbols with no callers AND no test coverage |
| **Module boundary analysis** | `boundary` classifies symbols as public API or internal-only |
| **Rename planning** | `rename_plan` previews all locations to update before renaming |
| **Similar symbol discovery** | `similar` finds functions with matching signatures and call patterns |
| **Hotspot identification** | Most-referenced, most-referencing, and largest functions |
| **File-level dependency graph** | Derived from cross-file refs, exportable as Graphviz DOT |
| **Git blame and history** | Per-symbol blame and commit history with optional function-level diffs |
| **Doc coverage auditing** | Find undocumented symbols with coverage percentages |
| **Relevance-ranked results** | Query results sorted by incoming reference count — most important first |
| **Index freshness** | `freshness` compares indexed commit to HEAD, lists changed files |
| **Crate dependency graph** | `crate_graph` shows workspace inter-crate dependencies with root/leaf identification |
| **Constructor tracking** | `new`, `from`, `default`, `clone` calls are tracked as refs with impl_type disambiguation |
| **Version-pinned docs** | Two-tier: crate summary + per-module detail from rustdoc JSON |
| **Full body on demand** | `full_body: true` reads untruncated source from disk |

## Statusline Extension

illu writes real-time status to `.illu/status`. See what it's doing in your terminal:

```
▸ opus · my-project › main  ▰▰▰▱▱▱▱▱▱▱ 28%  ◆ illu
▸ opus · my-project › main  ▰▰▰▱▱▱▱▱▱▱ 28%  ◆ illu: indexing ▸ refs [12/40]
```

| Color | Meaning |
|-------|---------|
| Green `◆ illu` | Ready — index is current |
| Yellow `◆ illu: indexing ...` | Parsing source files |
| Cyan `◆ illu: fetching docs ...` | Downloading dependency docs |

```bash
cp extensions/statusline/combined-statusline.sh ~/.claude/statusline.sh
chmod +x ~/.claude/statusline.sh
```

```json
{ "statusLine": { "command": "~/.claude/statusline.sh" } }
```

See [`extensions/statusline/`](extensions/statusline/) for standalone and add-to-existing options.

## How It Works

```
            ┌──────────────────────────────────────┐
            │          Your Rust Project            │
            │  src/*.rs  Cargo.toml  Cargo.lock     │
            └─────────────────┬────────────────────┘
                              │
                     ┌────────▼────────┐
                     │   tree-sitter    │  parse every .rs file
                     └────────┬────────┘
                              │
          symbols, refs, trait impls, deps, docs
                              │
                     ┌────────▼────────┐
                     │  SQLite + FTS5   │  .illu/index.db
                     └────────┬────────┘
                              │
                     ┌────────▼────────┐
                     │   MCP server     │  stdio transport
                     └────────┬────────┘
                              │
          ┌───────────────────┼───────────────────┐
          │                   │                    │
    Claude Code         Gemini CLI          Any MCP client
```

<details>
<summary>Architecture</summary>

```
src/
├── main.rs              # CLI, init, MCP server startup
├── lib.rs               # Shared utilities
├── status.rs            # Real-time status file (.illu/status)
├── db.rs                # SQLite (schema, queries, FTS5 + trigram)
├── indexer/
│   ├── mod.rs           # Orchestrator (index, refresh, skill file)
│   ├── parser.rs        # Tree-sitter (symbols, refs, visibility)
│   ├── store.rs         # DB writes
│   ├── dependencies.rs  # Cargo.toml / Cargo.lock parsing
│   ├── workspace.rs     # Workspace detection + member resolution
│   ├── cargo_doc.rs     # Nightly rustdoc JSON parsing
│   └── docs.rs          # Doc fetching (cargo doc → docs.rs → GitHub)
└── server/
    ├── mod.rs           # MCP server (rmcp, tool routing)
    └── tools/           # 31 tool handlers
        ├── query.rs         # Symbol/doc/file/body search
        ├── context.rs       # Full symbol context with callers/callees
        ├── batch_context.rs # Multi-symbol context
        ├── impact.rs        # Transitive dependency analysis
        ├── diff_impact.rs   # Git-diff-based batch impact
        ├── test_impact.rs   # Symbol-to-test mapping
        ├── crate_impact.rs  # Cross-crate impact for workspaces
        ├── callpath.rs      # Call chain tracing (BFS/DFS)
        ├── neighborhood.rs  # Bidirectional call graph exploration
        ├── references.rs    # Unified reference view
        ├── type_usage.rs    # Type usage in signatures/fields
        ├── file_graph.rs    # File-level dependency graph
        ├── graph_export.rs  # DOT/Graphviz export
        ├── unused.rs        # Dead code detection
        ├── orphaned.rs      # No callers + no tests
        ├── boundary.rs      # Module API boundary analysis
        ├── similar.rs       # Similar function discovery
        ├── rename_plan.rs   # Rename impact preview
        ├── doc_coverage.rs  # Documentation coverage audit
        ├── hotspots.rs      # Complexity and coupling hotspots
        ├── stats.rs         # Codebase statistics dashboard
        ├── symbols_at.rs    # File:line symbol lookup
        ├── implements.rs    # Trait/type relationships
        ├── docs.rs          # Dependency documentation
        ├── overview.rs      # Module symbol listing
        ├── tree.rs          # File/module hierarchy
        ├── crate_graph.rs   # Workspace crate dependencies
        ├── freshness.rs     # Index staleness check
        ├── health.rs        # Index quality diagnosis
        ├── blame.rs         # Git blame per symbol
        └── history.rs       # Git history per symbol
```

</details>

<details>
<summary>Development</summary>

```bash
cargo test                                                    # 528 tests
cargo clippy --all-targets --all-features -- -D warnings      # strict lints
cargo fmt --all -- --check                                    # formatting
RUST_LOG=debug cargo run -- --repo /path/to/project serve     # debug mode
```

| Test Suite | Count | What it guards |
|------------|-------|----------------|
| Unit | 339 | Parser, DB, indexer, tool handlers |
| Data integrity | 68 | Line numbers, refs, cross-crate resolution, stale cleanup |
| Data quality | 61 | End-to-end tool output format and content |
| Integration | 24 | Full pipeline: index, query, verify |
| Self-index | 19 | illu indexes itself — validates real-world accuracy |
| Error paths | 6 | Edge cases: empty files, missing symbols, Unicode |

</details>

## License

MIT
