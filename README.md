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
  <img src="https://img.shields.io/badge/tests-355_passing-brightgreen?style=flat-square" alt="355 tests"/>
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

illu gives your AI agent 12 tools through the [Model Context Protocol](https://modelcontextprotocol.io/):

### Find symbols instantly — `query`

Instead of grepping, the AI searches an indexed database with full-text search and exact-match priority.

```
query: "Config"                                         → symbols + docs matching Config
query: "Config", scope: "symbols", kind: "struct"       → just the struct
query: "Color", scope: "symbols", kind: "enum_variant"  → enum variants of Color
query: "", attribute: "test"                             → all #[test] functions
query: "", signature: "&Database"                        → functions taking &Database
```

### Understand a symbol completely — `context`

One call returns everything: signature, doc comments, source body, struct fields, trait impls, callers, callees, and related dependency docs. No need to read the whole file.

Supports `Type::method` syntax for disambiguation and file-scoped lookups:

```
symbol_name: "Database"                      → full definition + who it calls + who calls it
symbol_name: "Database::new"                 → only the new() method on Database
symbol_name: "Config", file: "src/db.rs"     → Config in a specific file
symbol_name: "parse_config", full_body: true → untruncated source for large functions
```

### Know what breaks before changing it — `impact`

Before modifying a symbol, the AI sees every function, struct, and crate that depends on it — with the dependency chain explained. Automatically discovers related tests and suggests which to run.

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

### Get accurate dependency docs — `docs`

The AI looks up documentation for your exact dependency versions (from `Cargo.lock`), so it never hallucinates API signatures.

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

### See what a change breaks — `diff_impact`

Instead of querying one symbol at a time, the AI passes a git ref and gets batch impact analysis for every symbol touched by the diff.

```
git_ref: "HEAD~3..HEAD"   → impact of last 3 commits
git_ref: "main"           → impact of current branch vs main
(omit)                    → impact of unstaged changes
```

```markdown
## Changed Symbols

### src/db.rs
- **search_symbols** (function, line 450-480)

### Downstream Impact

#### search_symbols
- handle_query (src/server/tools/query.rs) — depth 1
- handle_context (src/server/tools/context.rs) — depth 1

### Related Tests
- **test_search_symbols** (src/db.rs:1450)

Suggested: `cargo test test_search_symbols`
```

### Get multiple symbols at once — `batch_context`

When starting a task that touches several symbols, fetch all their context in one call instead of making separate requests.

```
symbols: ["Database", "handle_query", "parse_rust_source"]  → context for all three
symbols: ["Config", "Server"], full_body: true               → with full source bodies
```

### Trace call chains — `callpath`

Find the shortest path through the call graph between two symbols. Useful for understanding how a change in one function reaches another.

```
from: "main", to: "parse_rust_source"   → main → open_or_index → ... → parse_rust_source
from: "handle_query", to: "escape_like" → shows the call chain with file locations
```

### Find dead code — `unused`

Detect potentially unused symbols — public functions, structs, or traits with no incoming references. Excludes entry points (`main`, `#[test]`).

```
(no params)                                → all unreferenced public symbols
path: "src/server/", kind: "function"      → unused functions in server module
include_private: true                      → include private symbols too
```

### Check index health — `freshness`

See whether the index is current or stale. Shows the indexed commit vs HEAD and lists any files that changed since last indexing.

```
(no params)   → indexed commit, current HEAD, changed file list
```

### Visualize workspace structure — `crate_graph`

For multi-crate workspaces, shows the dependency graph between crates — which crate depends on which, plus root and leaf crates.

```
(no params)   → crate list + dependency arrows + root/leaf identification
```

### See project structure — `overview` and `tree`

The AI can explore the codebase layout without reading files:

- **`overview`** — public symbols under a path, grouped by file, with signatures and doc snippets. Set `include_private: true` to see internal helpers and test functions.
- **`tree`** — file/module hierarchy with symbol counts per file

```
path: "src/server/"                        → public API in the server module
path: "src/db.rs", include_private: true   → everything in db.rs, including private fns
```

## Works With

<table>
<tr>
<td width="50%" align="center">

### <img src="https://img.shields.io/badge/-5A29E4?style=flat-square&logo=anthropic&logoColor=white" height="20" align="center"/> &nbsp;Claude Code

Auto-configured via `.mcp.json` and `CLAUDE.md`

12 tools: `mcp__illu__query`, `mcp__illu__context`, `mcp__illu__callpath`, etc.

</td>
<td width="50%" align="center">

### <img src="https://img.shields.io/badge/-4285F4?style=flat-square&logo=google&logoColor=white" height="20" align="center"/> &nbsp;Gemini CLI

Auto-configured via `.gemini/settings.json` and `GEMINI.md`

12 tools: `@illu query`, `@illu context`, `@illu callpath`, etc.

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
| **Qualified symbol lookup** | `Database::new` syntax disambiguates methods across types; optional `file` filter |
| **Callers + callees** | `context` shows both what a symbol calls and who calls it |
| **Trait impl tracking** | Maps which types implement which traits |
| **Enum variant indexing** | Each variant is a searchable symbol — `Color::Red` via qualified lookup |
| **Impact analysis** | Recursive CTE walks the reference graph with configurable depth (default 5) |
| **Symbol-to-test mapping** | `impact` and `diff_impact` discover `#[test]` functions that exercise changed symbols |
| **Diff-based impact** | `diff_impact` maps git changes to symbols, shows downstream effects + test suggestions |
| **Call path tracing** | `callpath` finds the shortest call chain between any two symbols via BFS |
| **Batch context** | `batch_context` fetches context for multiple symbols in one call |
| **Dead code detection** | `unused` finds public symbols with zero incoming references |
| **Index freshness** | `freshness` compares indexed commit to HEAD, lists changed files |
| **Crate dependency graph** | `crate_graph` shows workspace inter-crate dependencies with root/leaf identification |
| **Attribute search** | `query` with `attribute` parameter finds symbols by derive/attribute (e.g. `#[test]`) |
| **Signature search** | `query` with `signature` parameter finds symbols by type signature pattern |
| **Constructor tracking** | `new`, `from`, `default`, `clone` calls are tracked as refs with impl_type disambiguation |
| **Private symbol access** | `overview` with `include_private: true` shows internal helpers and test functions |
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
    └── tools/           # 12 tools: query, context, batch_context, impact, diff_impact,
                         #   callpath, unused, freshness, docs, overview, tree, crate_graph
```

</details>

<details>
<summary>Development</summary>

```bash
cargo test                                                    # 355 tests
cargo clippy --all-targets --all-features -- -D warnings      # strict lints
cargo fmt --all -- --check                                    # formatting
RUST_LOG=debug cargo run -- --repo /path/to/project serve     # debug mode
```

| Test Suite | Count | What it guards |
|------------|-------|----------------|
| Unit | 182 | Parser, DB, indexer, tool handlers |
| Data integrity | 68 | Line numbers, refs, cross-crate resolution, stale cleanup |
| Data quality | 61 | End-to-end tool output format and content |
| Integration | 19 | Full pipeline: index, query, verify |
| Self-index | 19 | illu indexes itself — validates real-world accuracy |
| Error paths | 6 | Edge cases: empty files, missing symbols, Unicode |

</details>

## License

MIT
