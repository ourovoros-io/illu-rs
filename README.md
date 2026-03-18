<p align="center">
  <img src="docs/assets/banner.svg" alt="illu — Code intelligence for AI agents" width="700"/>
</p>

<p align="center">
  <strong>Give your AI agent a semantic understanding of your Rust codebase.</strong>
</p>

<p align="center">
  <a href="#supported-clients"><img src="https://img.shields.io/badge/Claude_Code-5A29E4?style=for-the-badge&logo=anthropic&logoColor=white" alt="Claude Code"/></a>
  <a href="#supported-clients"><img src="https://img.shields.io/badge/Gemini_CLI-4285F4?style=for-the-badge&logo=google&logoColor=white" alt="Gemini CLI"/></a>
  <a href="https://modelcontextprotocol.io"><img src="https://img.shields.io/badge/MCP-stdio-818cf8?style=for-the-badge" alt="MCP"/></a>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="MIT License"/></a>
  <img src="https://img.shields.io/badge/rust-2024_edition-dea584?style=flat-square&logo=rust" alt="Rust 2024"/>
  <img src="https://img.shields.io/badge/tests-243_passing-brightgreen?style=flat-square" alt="243 tests"/>
</p>

---

**illu-rs** (from *illumination*) is an MCP server that indexes Rust codebases and serves structured code intelligence to AI agents. Instead of reading entire files or grepping blindly, your AI gets instant access to symbol definitions, call graphs, impact analysis, and dependency docs.

## Why illu?

When AI agents work on large Rust projects, they waste context window and turns on:
- Reading 1000-line files to find one struct definition
- Grepping for function names without understanding call graphs
- Guessing at API signatures for external crates
- Missing downstream breakage when modifying public symbols

**illu solves all of this.** It parses your codebase with tree-sitter, stores everything in SQLite with full-text search, and exposes 6 MCP tools that give your AI agent surgical precision.

## Quick Start

```bash
# Install
cargo install --path .

# From your Rust project root:
illu-rs init .
```

That's it. illu writes the MCP config for your AI client, indexes the codebase, and is ready to serve queries. Next time you open Claude Code or Gemini CLI in that repo, illu starts automatically.

## Supported Clients

<table>
<tr>
<td width="50%" align="center">

### <img src="https://img.shields.io/badge/-5A29E4?style=flat-square&logo=anthropic&logoColor=white" height="20"/> Claude Code

Auto-configured via `.mcp.json` and `CLAUDE.md`.

Tools appear as `mcp__illu__query`, etc.

</td>
<td width="50%" align="center">

### <img src="https://img.shields.io/badge/-4285F4?style=flat-square&logo=google&logoColor=white" height="20"/> Gemini CLI

Auto-configured via `.gemini/settings.json` and `GEMINI.md`.

Tools appear as `mcp_illu_query` or via `@illu`.

</td>
</tr>
</table>

Any MCP client that supports stdio transport will work — illu speaks standard MCP.

## Tools

### `query` — Search the codebase

Find symbols, documentation, or files by name. Supports FTS5 prefix matching with trigram substring fallback.

```
query: "Config"              → all symbols/docs matching "Config"
query: "Config", scope: "symbols", kind: "struct"  → only struct definitions
```

### `context` — Full symbol details

Everything your AI needs about a symbol: signature, doc comments, source body, struct fields, trait impls, callees, and related dependency docs.

```
symbol_name: "Database"      → definition, fields, trait impls, who calls it
symbol_name: "parse_config", full_body: true  → includes untruncated source
```

### `impact` — Change analysis

Before modifying a symbol, see everything that depends on it. Walks the reference graph up to depth 5 with "via" chains showing *why* something is affected.

```
symbol_name: "Config"        → all transitive dependents across the workspace
```

```
## Impact Analysis: Config

### Affected Crates
- **core** (defined here)
- **api**
- **cli**

### Depth 1
- **parse_config** (src/lib.rs)

### Depth 2
- **run_server** (src/main.rs) — via parse_config
```

### `docs` — Dependency documentation

Fetch docs for external crates at the exact version from your `Cargo.lock`. Uses `cargo +nightly doc` JSON output when available, falls back to docs.rs.

```
dependency: "serde"                    → all docs for serde
dependency: "tokio", topic: "runtime"  → filtered by topic
```

### `overview` — Structural map

Public symbols under a path prefix, grouped by file, with doc comment snippets and summary statistics.

```
path: "src/server/"   → all public API in the server module
```

### `tree` — Module tree

File/module structure with public symbol counts per file. Helps the AI understand project layout before diving in.

```
path: "src/"          → full module tree with symbol counts
```

## How It Works

```
                    ┌─────────────────────────────────────────────┐
                    │              Your Rust Project              │
                    │  src/*.rs  Cargo.toml  Cargo.lock           │
                    └──────────────────┬──────────────────────────┘
                                       │
                              ┌────────▼────────┐
                              │   tree-sitter    │
                              │  AST extraction  │
                              └────────┬─────────┘
                                       │
                    symbols, refs, trait impls, deps, docs
                                       │
                              ┌────────▼────────┐
                              │  SQLite + FTS5   │
                              │  .illu/index.db  │
                              └────────┬─────────┘
                                       │
                              ┌────────▼────────┐
                              │   MCP Server     │
                              │   (stdio)        │
                              └────────┬─────────┘
                                       │
                    ┌──────────────────┼──────────────────┐
                    │                  │                   │
              Claude Code        Gemini CLI         Any MCP Client
```

**Indexing pipeline:**
1. Parse every `.rs` file with tree-sitter (extracts symbols, signatures, bodies, doc comments, attributes)
2. Resolve dependencies from `Cargo.toml` + `Cargo.lock`
3. Extract cross-references between symbols (with local scope filtering to avoid false positives)
4. Detect trait implementations and link them to types
5. Fetch dependency docs via `cargo +nightly doc` (JSON) or docs.rs (HTML)
6. Store everything in SQLite with FTS5 + trigram indexes

**Incremental updates:** Content-hashed files — only re-parses what changed. Sub-second refresh on typical edits.

## Features

| Feature | Description |
|---------|-------------|
| **Incremental indexing** | Content-hashed — only re-parses changed files |
| **Workspace support** | Multi-crate workspaces with inter-crate dependency tracking |
| **Full-text search** | FTS5 prefix matching + trigram substring search |
| **Call graph** | Tracks symbol references with scope-aware local variable filtering |
| **Trait impl tracking** | Maps which types implement which traits |
| **Impact analysis** | Recursive CTE walks references up to depth 5 |
| **Dependency docs** | `cargo doc` JSON (nightly) with docs.rs/GitHub fallback |
| **Attribute extraction** | `#[derive(Serialize)]`, `#[test]`, custom attributes |
| **Full body access** | `full_body: true` reads source from disk for truncated functions |
| **Dual client support** | Auto-configures both Claude Code and Gemini CLI |

## Architecture

```
src/
├── main.rs              # CLI, init, MCP server startup
├── lib.rs               # Shared utilities
├── db.rs                # SQLite layer (schema, queries, FTS5 + trigram)
├── indexer/
│   ├── mod.rs           # Orchestrator (index, refresh, skill file)
│   ├── parser.rs        # Tree-sitter extraction (symbols, refs, visibility)
│   ├── store.rs         # DB write operations
│   ├── dependencies.rs  # Cargo.toml/Cargo.lock parsing
│   ├── workspace.rs     # Workspace detection + member resolution
│   ├── cargo_doc.rs     # Nightly rustdoc JSON parsing
│   └── docs.rs          # Doc fetching pipeline (cargo doc → network)
└── server/
    ├── mod.rs           # MCP server (rmcp, tool routing)
    └── tools/           # query, context, impact, docs, overview, tree
```

## Development

```bash
# All tests (243 total)
cargo test

# Lint (strict — pedantic clippy, deny unwrap/panic/todo)
cargo clippy --all-targets --all-features -- -D warnings

# Format
cargo fmt --all -- --check

# Run with debug logging
RUST_LOG=debug cargo run -- --repo /path/to/project serve
```

### Test structure

| Suite | Count | Purpose |
|-------|-------|---------|
| Unit tests | 144 | Parser, DB, indexer, tool handlers |
| Data integrity | 38 | Guards against data poisoning (line numbers, signatures, refs) |
| Data quality | 42 | End-to-end tool output format and content |
| Integration | 19 | Full pipeline: index, query, verify |

## License

MIT
