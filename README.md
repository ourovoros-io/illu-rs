<p align="center">
  <img src="docs/assets/banner.svg" alt="illu — Code intelligence for AI agents" width="400"/>
</p>

<p align="center">
  <strong>Give your AI agent a semantic understanding of your Rust codebase.</strong>
</p>

<p align="center">
  <a href="#works-with"><img src="https://img.shields.io/badge/Claude_Code-5A29E4?style=for-the-badge&logo=anthropic&logoColor=white" alt="Claude Code"/></a>
  <a href="#works-with"><img src="https://img.shields.io/badge/Claude_Desktop-5A29E4?style=for-the-badge&logo=anthropic&logoColor=white" alt="Claude Desktop"/></a>
  <a href="#works-with"><img src="https://img.shields.io/badge/Gemini_CLI-4285F4?style=for-the-badge&logo=google&logoColor=white" alt="Gemini CLI"/></a>
  <a href="#works-with"><img src="https://img.shields.io/badge/Codex-000000?style=for-the-badge&logo=openai&logoColor=white" alt="Codex"/></a>
  <a href="#works-with"><img src="https://img.shields.io/badge/Cursor-000000?style=for-the-badge&logoColor=white" alt="Cursor"/></a>
  <a href="#works-with"><img src="https://img.shields.io/badge/VS_Code-007ACC?style=for-the-badge&logo=visualstudiocode&logoColor=white" alt="VS Code + Copilot"/></a>
  <a href="#works-with"><img src="https://img.shields.io/badge/Antigravity-4285F4?style=for-the-badge&logo=google&logoColor=white" alt="Antigravity"/></a>
  <a href="https://modelcontextprotocol.io"><img src="https://img.shields.io/badge/MCP-stdio-818cf8?style=for-the-badge" alt="MCP"/></a>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="MIT License"/></a>
  <img src="https://img.shields.io/badge/rust-2024_edition-dea584?style=flat-square&logo=rust" alt="Rust 2024"/>
  <img src="https://img.shields.io/badge/tools-49-blue?style=flat-square" alt="49 tools"/>
  <img src="https://img.shields.io/badge/agents-8_supported-blueviolet?style=flat-square" alt="8 agents supported"/>
  <img src="https://img.shields.io/badge/tests-841_passing-brightgreen?style=flat-square" alt="841 tests"/>
</p>

---

## Get Started

Install illu and set it up globally:

```bash
# Install (works on macOS and Linux)
git clone https://github.com/GeorgiosDelkos/illu-rs.git
cargo install --path illu-rs

# Global setup — detects your agents, prompts you to pick
illu-rs install
```

That's it. Open any supported agent in a Rust / TypeScript / Python project — illu auto-detects the repo, indexes it, and starts serving tools. Works with **git worktrees** too — each worktree gets its own isolated index.

`install` runs an interactive multi-select prompt listing the agents it detected on your system (boxes pre-checked for installed ones, unchecked for others you can opt into). For each selected agent it writes MCP config to the right location — `~/.claude/settings.json`, `~/Library/Application Support/Claude/claude_desktop_config.json`, `~/.codex/config.toml`, `~/.cursor/mcp.json`, etc. — plus a [statusline](#statusline) for Claude Code and a global gitignore for `.illu/`.

**Non-interactive and scripted flows:**

```bash
illu-rs install --yes                             # no prompt, accept detected agents
illu-rs install --agent claude-code --agent cursor # configure exactly these
illu-rs install --all                             # every agent, no prompt
illu-rs install --dry-run                          # print what would be written, touch nothing
```

If `illu-rs install` runs without a TTY (CI, piped, etc.) it behaves as `--yes`. If no agents are detected and no explicit flags are passed, it exits non-zero with a list of supported agent IDs.

> **Requirements:** Rust toolchain and a C compiler (Xcode CLI tools on macOS, `build-essential` on Linux). All C dependencies (SQLite, tree-sitter) are compiled from source — no system libraries needed. For `ra_*` tools, install rust-analyzer: `rustup component add rust-analyzer` (optional — core tools work without it).

<details>
<summary>Per-repo setup (alternative to global install)</summary>

For repo-specific configuration, use `init` instead:

```bash
cd your-project
illu-rs init
```

Same interactive prompt as `install`, but scoped to agents with per-repo config support (Claude Code, Gemini CLI, Cursor, VS Code + Copilot). Writes `.mcp.json`, `.cursor/mcp.json`, `.vscode/mcp.json`, `CLAUDE.md`, etc. as appropriate. The same flags work: `--agent`, `--all`, `--yes`, `--dry-run`.

</details>

<details>
<summary>Manual MCP config</summary>

Add to your MCP settings file:

```json
{
  "mcpServers": {
    "illu": {
      "command": "/path/to/illu-rs",
      "args": ["serve"],
      "env": { "RUST_LOG": "warn" }
    }
  }
}
```

Without `--repo`, illu auto-detects the repo from CWD via `git rev-parse --show-toplevel`.

</details>

## What Your AI Gets

illu gives your AI agent **49 tools** through the [Model Context Protocol](https://modelcontextprotocol.io/), organized into seven categories — including cross-repo intelligence and optional **rust-analyzer integration** for compiler-accurate operations:

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
query: "PRAGMA", scope: "strings"                        → search inside string literals only
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

- **`overview`** — public symbols under a path, grouped by file, with signatures, doc snippets, intra-file callees, and external callers
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

#### Export graphs — `graph_export`

Export call graphs or file dependency graphs in multiple formats.

```
symbol_name: "handle_impact"                       → DOT graph of impact's call tree
symbol_name: "handle_impact", format: "edges"      → compact A -> B lines (AI-friendly)
symbol_name: "handle_impact", format: "summary"    → node/edge counts, roots, leaves
path: "src/indexer/"                               → file dependency graph
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

### Compiler-Accurate Intelligence (rust-analyzer)

When rust-analyzer is installed, illu automatically spawns it in the background and exposes **13 additional tools** prefixed with `ra_`. These provide compiler-accurate results — resolving through macros, trait impls, and generics — complementing the fast tree-sitter-based tools.

> **Optional:** If rust-analyzer isn't installed or fails to start, all 36 core tools work normally. Use `--no-ra` to skip RA entirely.

#### Go-to-definition — `ra_definition`

Compiler-accurate definition lookup. Resolves through macros, generic impls, and re-exports that tree-sitter can't follow.

```
position: "src/db.rs:42:10"   → exact definition, even through macro-generated code
```

#### Type info and docs — `ra_hover`

Full type information and documentation for any position.

```
position: "src/server/mod.rs:100:15"   → type signature, doc comments, trait bounds
```

#### Compilation diagnostics — `ra_diagnostics`

Real compilation errors and warnings from the Rust compiler, not just syntax issues.

```
file: "src/db.rs"   → errors and warnings in this file
(omit file)         → all diagnostics across the workspace
```

#### Call hierarchy — `ra_call_hierarchy`

Compiler-accurate callers and callees, including calls through trait objects and dynamic dispatch.

```
position: "src/db.rs:42:10", direction: "in"    → who calls this
position: "src/db.rs:42:10", direction: "out"   → what this calls
position: "src/db.rs:42:10", direction: "both"  → both (default)
```

#### Type hierarchy — `ra_type_hierarchy`

Supertypes (traits implemented) and subtypes, including blanket impls and generics.

```
position: "src/db.rs:10:10"   → traits this type implements + types that extend it
```

#### Rename preview — `ra_rename`

Preview the impact of renaming a symbol — files affected and reference counts. Does not apply changes.

```
position: "src/db.rs:42:10", new_name: "open_connection"
```

#### Safe rename — `ra_safe_rename`

Applies a rename across the workspace, then verifies no new compilation errors were introduced.

```
position: "src/db.rs:42:10", new_name: "open_connection"
```

#### Code actions — `ra_code_actions`

Quick fixes and refactoring suggestions from rust-analyzer.

```
position: "src/db.rs:42:10"                    → all available actions
position: "src/db.rs:42:10", kind: "refactor"  → only refactoring actions
```

#### Macro expansion — `ra_expand_macro`

See the generated Rust code from a macro invocation.

```
position: "src/server/mod.rs:422:1"   → expanded code from #[tool_router]
```

#### Structural search and replace — `ra_ssr`

Pattern-based search and replace using rust-analyzer's SSR engine. Understands Rust syntax, not just text.

```
pattern: "foo($a, $b) ==>> bar($b, $a)"             → swap arguments
pattern: "Vec::new() ==>> Vec::with_capacity(16)"    → replace pattern
```

#### Full symbol context — `ra_context`

Combines definition, hover, references, callers, callees, implementations, and related tests in one call.

```
position: "src/db.rs:42:10"   → everything about this symbol
```

#### Related tests — `ra_related_tests`

Compiler-accurate test discovery — finds tests that exercise a symbol through any call path.

```
position: "src/db.rs:42:10"   → tests that cover this function
```

#### Syntax tree — `ra_syntax_tree`

Debug view of the parsed syntax tree. Useful for understanding macro expansion and parse structure.

```
file: "src/db.rs"   → full syntax tree
```

### Multi-Repo Intelligence

#### See all your repos — `repos`

Dashboard of all registered repos with status and symbol counts. Repos auto-register when illu starts in them.

```
(no params)   → table of all repos: name, path, status (active/indexed/missing), symbol count
```

#### Search across repos — `cross_query`

Find symbols in other registered repos. Same parameters as `query`, results grouped by repo.

```
query: "Database"   → finds Database structs/impls across all your repos
```

#### Cross-repo impact — `cross_impact`

"If I change this symbol, what breaks in other repos?" Name-based reference search across all registered repos.

```
symbol_name: "SharedConfig"   → references in other repos that use this type
```

#### Inter-repo dependencies — `cross_deps`

Shows how repos relate: path dependencies (direct source links) and shared crate dependencies.

```
(no params)   → path deps between repos + shared crates table
```

#### Cross-repo call chains — `cross_callpath`

Find symbols that bridge between repos — callees in the current repo that also exist in another.

```
from: "process_request", to: "handle_event", target_repo: "event-service"
```

## Works With

`illu-rs install` / `illu-rs init` ship with first-class support for eight MCP-capable agents. Pick any combination at setup time — detect-and-confirm, or explicit via `--agent <id>`.

| Agent | Type | Scope | Config location written |
|---|---|---|---|
| Claude Code | CLI | per-repo + global | `.mcp.json`, `.claude/`, `~/.claude/settings.json` |
| Claude Desktop | Desktop | global | `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS) |
| Gemini CLI | CLI | per-repo + global | `.gemini/settings.json`, `~/.gemini/settings.json` |
| Codex CLI | CLI | global | `~/.codex/config.toml` |
| Codex Desktop | Desktop | global | `~/.codex/config.toml` |
| Cursor | IDE | per-repo + global | `.cursor/mcp.json`, `~/.cursor/mcp.json` |
| VS Code + Copilot | IDE | per-repo | `.vscode/mcp.json` |
| Antigravity | IDE | global | `~/.antigravity/mcp.json` |

All 49 tools are available to every configured agent. Claude-family agents see them as `mcp__illu__query`, Gemini CLI as `@illu query`, Codex / Cursor / VS Code / Antigravity per their respective MCP conventions. Any other MCP client with stdio transport support works too — illu speaks standard MCP and can be wired in manually via the [manual MCP config](#get-started) block above.

## Features

| Feature | What it does |
|---------|-------------|
| **Multi-agent setup** | `illu-rs install` / `illu-rs init` support 8 agents (Claude Code/Desktop, Gemini CLI, Codex CLI/Desktop, Cursor, VS Code + Copilot, Antigravity) — detect + prompt, no blanket writes |
| **Scripted setup** | `--agent <id>`, `--all`, `--yes`, `--dry-run` flags for CI and dotfiles; no-TTY auto-accepts detected agents |
| **Read-modify-write configs** | Merges into existing MCP config files, preserves unrelated server entries, errors cleanly on malformed input |
| **Self-heal on serve** | `illu-rs serve` detects the calling agent via env vars and rewrites only that agent's config — no cross-contamination |
| **Worktree support** | Each git worktree gets its own isolated index, auto-detected from CWD |
| **Multi-repo registry** | Repos auto-register in `~/.illu/registry.toml`; worktrees dedup by shared git dir |
| **Cross-repo search** | `cross_query` searches symbols across all registered repos |
| **Cross-repo impact** | `cross_impact` finds references to a symbol in other repos |
| **Cross-repo dependencies** | `cross_deps` shows path deps and shared crates between repos |
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
| **File-level dependency graph** | Derived from cross-file refs, exportable as DOT, edge list, or summary |
| **Git blame and history** | Per-symbol blame and commit history with optional function-level diffs |
| **Doc coverage auditing** | Find undocumented symbols with coverage percentages |
| **Relevance-ranked results** | Query results sorted by incoming reference count — most important first |
| **Index freshness** | `freshness` compares indexed commit to HEAD, lists changed files |
| **Crate dependency graph** | `crate_graph` shows workspace inter-crate dependencies with root/leaf identification |
| **Constructor tracking** | `new`, `from`, `default`, `clone` calls are tracked as refs with impl_type disambiguation |
| **Version-pinned docs** | Two-tier: crate summary + per-module detail from rustdoc JSON |
| **Full body on demand** | `full_body: true` reads untruncated source from disk |
| **rust-analyzer integration** | Optional LSP backend for compiler-accurate definitions, hover, diagnostics, rename, and more |
| **Safe rename** | `ra_safe_rename` applies rename across workspace and verifies no new compilation errors |
| **Macro expansion** | `ra_expand_macro` shows generated code from any macro invocation |
| **Structural search & replace** | `ra_ssr` uses RA's syntax-aware engine for pattern-based refactoring |
| **Type hierarchy** | `ra_type_hierarchy` shows supertypes and subtypes including blanket impls |
| **Compiler diagnostics** | `ra_diagnostics` shows real compilation errors, not just parse errors |
| **Background RA startup** | rust-analyzer spawns in background; core tools work immediately while RA indexes |

## Statusline

`illu install` includes a Claude Code statusline that shows model, branch, context usage, and live illu status:

```
▸ opus · my-project › main  ▰▰▰▱▱▱▱▱▱▱ 28% · 4m12s  ◆ illu
▸ opus · my-project › main  ▰▰▰▱▱▱▱▱▱▱ 28% · 4m12s  ◆ illu: indexing ▸ refs [12/40]
```

| Color | Meaning |
|-------|---------|
| Green `◆ illu` | Ready — index is current |
| Yellow `◆ illu: indexing ...` | Parsing source files |
| Cyan `◆ illu: fetching docs ...` | Downloading dependency docs |

The statusline is installed automatically to `~/.illu/statusline.sh` and configured in `~/.claude/settings.json`. If you already have a custom statusline, `illu install` skips the config and prints instructions for manual setup.

<details>
<summary>Manual statusline setup</summary>

If you already have a statusline and want to switch:

```json
{ "statusLine": { "type": "command", "command": "~/.illu/statusline.sh" } }
```

Or copy the script to your preferred location:

```bash
cp ~/.illu/statusline.sh ~/.claude/statusline.sh
```

The script requires `jq` and `git` on PATH.

</details>

## How It Works

```
            ┌──────────────────────────────────────┐
            │          Your Rust Project            │
            │  src/*.rs  Cargo.toml  Cargo.lock     │
            └──────────┬───────────────┬───────────┘
                       │               │
              ┌────────▼────────┐  ┌───▼──────────────┐
              │   tree-sitter    │  │  rust-analyzer    │  optional
              │  (fast, offline) │  │  (compiler-exact) │
              └────────┬────────┘  └───┬──────────────┘
                       │               │
          symbols, refs, deps      definitions, types,
          trait impls, docs        diagnostics, rename
                       │               │
              ┌────────▼────────┐      │
              │  SQLite + FTS5   │     │
              │  .illu/index.db  │     │
              └────────┬────────┘      │
                       │               │
              ┌────────▼───────────────▼──┐
              │       MCP server           │  stdio transport
              │  36 core + 13 ra_* tools   │
              └────────────┬──────────────┘
                           │
       ┌───────────────────┼───────────────────┐
       │                   │                   │
  Claude Code           Cursor            Gemini CLI
  Claude Desktop        VS Code+Copilot   Codex CLI/Desktop
                                          Antigravity
```

**Two engines:** tree-sitter provides fast offline indexing (36 tools). rust-analyzer provides compiler-accurate intelligence (13 `ra_*` tools) when available. Both run simultaneously.

**Multi-repo:** Each repo gets its own index. A global registry at `~/.illu/registry.toml` tracks all repos. Cross-repo tools open other indexes read-only on demand.

<details>
<summary>Architecture</summary>

```
src/
├── main.rs              # CLI, init, MCP server startup (+ RA lifecycle)
├── lib.rs               # Shared utilities
├── status.rs            # Real-time status file (.illu/status)
├── git.rs               # Git operations (worktree detection, toplevel)
├── registry.rs          # Multi-repo registry (~/.illu/registry.toml)
├── db.rs                # SQLite (schema, queries, FTS5 + trigram)
├── agents/              # Per-agent config registry and orchestration
│   ├── mod.rs           # AGENTS static, configure_repo/global, self_heal_on_serve
│   ├── detect.rs        # DetectionContext trait + RealContext (env/PATH/fs)
│   ├── formats.rs       # MCP config writers (JSON + TOML, read-modify-write)
│   ├── paths.rs         # Platform-aware GlobalPath resolver
│   ├── selection.rs     # Pure flags + detection -> Vec<&Agent>
│   ├── prompt.rs        # dialoguer-based multi-select prompt
│   ├── allow_list.rs    # Claude-family settings.local.json allow-list
│   ├── instruction_md.rs# CLAUDE.md / GEMINI.md section injection
│   └── agent_files.rs   # .claude/agents/* / .gemini/agents/* generation
├── indexer/
│   ├── mod.rs           # Orchestrator (index, refresh, skill file)
│   ├── parser.rs        # Tree-sitter (symbols, refs, visibility)
│   ├── store.rs         # DB writes
│   ├── dependencies.rs  # Cargo.toml / Cargo.lock parsing
│   ├── workspace.rs     # Workspace detection + member resolution
│   ├── cargo_doc.rs     # Nightly rustdoc JSON parsing
│   └── docs.rs          # Doc fetching (cargo doc → docs.rs → GitHub)
├── ra/                  # rust-analyzer LSP client (optional at runtime)
│   ├── client.rs        # RaClient: spawn, initialize, shutdown
│   ├── transport.rs     # async-lsp notification router, progress tracking
│   ├── document.rs      # File open/sync tracker for LSP session
│   ├── retry.rs         # Exponential backoff on CONTENT_MODIFIED
│   ├── lsp.rs           # Typed LSP wrappers (definition, hover, rename, etc.)
│   ├── extensions.rs    # rust-analyzer custom requests (macro expand, SSR, etc.)
│   ├── ops.rs           # Composed operations (symbol_context, safe_rename)
│   ├── types.rs         # PositionSpec, RichLocation, SymbolContext, etc.
│   └── error.rs         # RaError enum
└── server/
    ├── mod.rs           # MCP server (rmcp, tool routing, 36 core + 13 RA tools)
    └── tools/           # 36 core tool handlers (RA tools inline in mod.rs)
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
        ├── history.rs       # Git history per symbol
        ├── repos.rs         # Registered repos dashboard
        ├── cross_query.rs   # Cross-repo symbol search
        ├── cross_impact.rs  # Cross-repo impact analysis
        ├── cross_deps.rs    # Inter-repo dependency graph
        └── cross_callpath.rs # Cross-repo call chain tracing
```

</details>

<details>
<summary>Development</summary>

```bash
cargo test                                                    # 841 tests
cargo clippy --all-targets --all-features -- -D warnings      # strict lints
cargo fmt --all -- --check                                    # formatting
RUST_LOG=debug cargo run -- --repo /path/to/project serve     # debug mode
RUST_LOG=debug cargo run -- --no-ra serve                     # without rust-analyzer
```

| Test Suite | Count | What it guards |
|------------|-------|----------------|
| Unit | 528 | Parser, DB, indexer, tool handlers, registry, agent setup |
| Agents end-to-end | 8 | `configure_repo` / `configure_global` write correct files per agent |
| Data integrity | 68 | Line numbers, refs, cross-crate resolution, stale cleanup |
| Data quality | 61 | End-to-end tool output format and content |
| Integration | 27 | Full pipeline: index, query, verify + cross-repo |
| Self-index | 24 | illu indexes itself — validates real-world accuracy |
| TypeScript / Python | 29 | Non-Rust language parsers |
| Error paths + misc | 96 | Edge cases, incremental correctness, cross-repo, etc. |

</details>

## License

MIT
