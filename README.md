# illu-rs

Code intelligence MCP server for Rust projects. Indexes your codebase with tree-sitter, stores symbols and references in SQLite, and serves structured queries over the [Model Context Protocol](https://modelcontextprotocol.io/) (stdio transport).

Built for Claude — gives it instant access to symbol definitions, call graphs, impact analysis, and dependency docs without reading entire files.

## How it works

```
Rust source files
       │
       ▼
  tree-sitter ──► symbols, refs, trait impls
       │
       ▼
  SQLite + FTS5 ──► .illu/index.db
       │
       ▼
  MCP server (stdio) ──► Claude / any MCP client
```

On first run, illu-rs parses every `.rs` file in the repo, extracts symbols (functions, structs, enums, traits, impls, consts, statics, type aliases, macros), maps references between them, and stores everything in a local SQLite database with full-text search. Subsequent runs detect changes via content hashing and re-index only modified files.

## Installation

```bash
cargo install --path .
```

Or build from source:

```bash
cargo build --release
# Binary at target/release/illu-rs
```

## Quick start

```bash
# From your Rust project root:
illu-rs serve
```

This will:
1. Index the codebase (or refresh if already indexed)
2. Write `.mcp.json` for Claude Code auto-discovery
3. Append an illu section to `CLAUDE.md` with usage instructions
4. Start the MCP server on stdio

Claude will automatically discover and use the tools.

### Manual setup

Add to your project's `.mcp.json`:

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

## Tools

### `query` — Search the codebase

Find symbols, documentation, or files by name.

| Parameter | Type | Description |
|-----------|------|-------------|
| `query` | string | Search term |
| `scope` | string? | `symbols`, `docs`, `files`, or `all` (default) |
| `kind` | string? | Filter by symbol kind: `function`, `struct`, `enum`, `trait`, `impl`, `const`, `static`, `type_alias`, `macro` |

```bash
illu-rs query Config --scope symbols --kind struct
```

### `context` — Symbol details

Full context for a symbol: definition, signature, doc comments, struct fields, enum variants, trait impls, callees, and related dependency docs.

| Parameter | Type | Description |
|-----------|------|-------------|
| `symbol_name` | string | Name of the symbol |

```bash
illu-rs context Config
```

### `impact` — Change analysis

Find all transitive dependents of a symbol. Shows the dependency chain so you know *why* something is affected.

| Parameter | Type | Description |
|-----------|------|-------------|
| `symbol_name` | string | Name of the symbol to analyze |

```bash
illu-rs impact Config
```

Example output:
```
## Impact Analysis: Config

### Depth 1
- **parse_config** (src/lib.rs)

### Depth 2
- **run_server** (src/main.rs) — via parse_config
```

### `docs` — Dependency documentation

Fetch and search documentation for external dependencies (from docs.rs).

| Parameter | Type | Description |
|-----------|------|-------------|
| `dependency` | string | Crate name |
| `topic` | string? | Filter docs by topic |

```bash
illu-rs docs serde --topic serialize
```

### `overview` — Structural map

List public symbols under a path prefix, grouped by file, with summary statistics.

| Parameter | Type | Description |
|-----------|------|-------------|
| `path` | string | File path prefix (e.g. `src/server/`) |

```bash
illu-rs overview src/server/
```

### `tree` — Module tree

Show the file/module structure with public symbol counts per file.

| Parameter | Type | Description |
|-----------|------|-------------|
| `path` | string | File path prefix |

```bash
illu-rs tree src/
```

## Features

- **Incremental indexing** — content-hashed, only re-parses changed files
- **Workspace support** — multi-crate workspaces with inter-crate dependency tracking
- **Full-text search** — FTS5 with substring fallback, exact match priority
- **Call graph** — tracks which symbols reference which, with noisy names filtered out
- **Trait impl tracking** — knows which types implement which traits
- **Attribute/derive extraction** — `#[derive(Serialize)]`, `#[test]`, etc.
- **Impact analysis** — recursive CTE walks the reference graph up to depth 5
- **Dependency docs** — fetches docs.rs content at the pinned version from Cargo.lock
- **Claude skill file** — auto-generates `.claude/skills/illu-rs.md` listing dependencies and available tools

## Architecture

```
src/
├── main.rs              # CLI + MCP server startup
├── db.rs                # SQLite layer (schema, queries, FTS5)
├── indexer/
│   ├── mod.rs           # Orchestrator (index_repo, refresh_index)
│   ├── parser.rs        # Tree-sitter AST extraction
│   ├── store.rs         # DB write operations
│   ├── dependencies.rs  # Cargo.toml/Cargo.lock parsing
│   ├── workspace.rs     # Workspace detection + member resolution
│   └── docs.rs          # docs.rs fetching + HTML extraction
└── server/
    ├── mod.rs           # MCP server (rmcp, tool routing)
    └── tools/
        ├── query.rs     # Search symbols/docs/files
        ├── context.rs   # Symbol detail view
        ├── impact.rs    # Change impact analysis
        ├── docs.rs      # Dependency doc lookup
        ├── overview.rs  # Structural symbol listing
        └── tree.rs      # Module tree with counts
```

## Development

```bash
# Run tests (120 total: 101 unit + 19 integration)
cargo test

# Lint
cargo clippy --all-targets --all-features -- -D warnings

# Format
cargo fmt --all

# Run against a repo with debug logging
RUST_LOG=debug cargo run -- --repo /path/to/project serve
```

## License

MIT
