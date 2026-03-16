# illu-rs Design Document

## What illu-rs Is

illu-rs is a Rust binary that acts as an MCP server over stdio. It indexes a Rust repository — code structure, symbol relationships, and version-pinned dependency documentation — and exposes 4 tools that Claude can query during development.

The core value proposition: heavy analysis and documentation gathering happens once at startup, and Claude gets instant, focused answers without burning context window on research.

What it is not:

- Not a UI or dashboard
- Not a web service
- Not a general-purpose code search engine
- Not a replacement for `cargo doc`

## Lifecycle

1. Claude Code starts `illu-rs` as an MCP server (stdio)
2. illu-rs detects the repo from cwd (or `--repo` flag)
3. Parses `Cargo.toml` + `Cargo.lock` for dependency graph with exact versions
4. Tree-sitter parses all `.rs` files for symbols, imports, call relationships
5. Fetches docs for direct dependencies (docs.rs + GitHub README at pinned version)
6. Stores everything in a per-repo SQLite database (`.illu/index.db`)
7. MCP server starts responding to tool calls
8. Transitive dep docs are fetched lazily on first query

## Indexing Pipeline

### Phase 1 — Dependency Resolution

- Parse `Cargo.toml` for direct dependencies and their version specs
- Parse `Cargo.lock` for the exact resolved versions of everything
- Classify each dependency as direct or transitive
- Extract registry metadata (repository URL, description, features used)

### Phase 2 — Code Parsing (Tree-sitter)

- Walk `src/` and find all `.rs` files
- Parse each file into an AST using `tree-sitter-rust`
- Extract: functions, structs, enums, traits, impls, `use` statements, `mod` declarations
- For each symbol: name, visibility, file path, line range, signature
- Resolve cross-file relationships: which symbols import/use which others

### Phase 3 — Documentation Fetching (direct deps only)

- For each direct dependency at its locked version:
  - Fetch API docs from `docs.rs/crate/{name}/{version}`
  - Resolve the `repository` field from crates.io registry metadata
  - Fetch the README from the matching git tag on GitHub
- Parse and store as structured text in SQLite with FTS5 indexing
- Transitive deps are left unfetched until queried

### Phase 4 — Indexing into SQLite

- Store all symbols, relationships, and docs in `.illu/index.db`
- Build FTS5 indexes over: symbol names, doc content, file paths
- Store a metadata row with the repo's current commit hash for freshness checks

### Incremental Updates

On subsequent startups, compare current commit hash to stored one. If changed, re-parse only modified files (via `git diff`), re-fetch docs only if `Cargo.lock` changed for that dependency.

## MCP Tool Surface

### `query` — "Find me things related to X"

- Full-text search across symbols, file paths, and documentation
- Accepts optional filters: `scope` (one of `symbols`, `docs`, `files`, `all`)
- Returns ranked results with file location, symbol kind, and a snippet of context

### `context` — "Tell me everything about X"

- Input: a symbol name or file path
- Returns: definition, signature, visibility, file + line range, what it imports, what imports it, relevant doc snippets from dependencies it uses
- 360-degree view of a symbol without Claude needing to read multiple files

### `impact` — "What breaks if I change X?"

- Input: a symbol name or file path
- Returns: all direct and transitive dependents within the repo
- Grouped by depth (direct users, their users, etc.)

### `docs` — "How do I use dependency Y?"

- Input: dependency name, optional symbol/topic filter
- Returns: API docs + README content for the pinned version
- If it's a transitive dep not yet fetched, fetches and caches on the spot

## Storage Schema

Single SQLite database at `.illu/index.db`.

### Tables

**`metadata`** — Freshness tracking: `repo_path`, `commit_hash`, `last_indexed_at`.

**`files`** — Source files: `id`, `path`, `hash` (content hash for incremental diffing).

**`symbols`** — Code elements: `id`, `file_id`, `name`, `kind` (function, struct, enum, trait, impl, mod), `visibility` (pub, pub(crate), private), `line_start`, `line_end`, `signature`.

**`symbol_refs`** — Relationships: `from_symbol_id`, `to_symbol_id`, `ref_kind` (calls, imports, implements, field_access). Traversed with recursive CTEs for impact analysis.

**`dependencies`** — From Cargo.toml/Cargo.lock: `id`, `name`, `version`, `is_direct`, `repository_url`, `features_used`.

**`docs`** — Fetched documentation: `id`, `dependency_id`, `source` (docs_rs, github_readme), `content`, `fetched_at`.

**`docs_fts`** — FTS5 virtual table over `docs.content`.

**`symbols_fts`** — FTS5 virtual table over `symbols.name` + `symbols.signature`.

No ORM — raw `rusqlite` with prepared statements.

## Dependencies

| Crate | Purpose |
|-------|---------|
| `tree-sitter` + `tree-sitter-rust` | AST parsing of Rust source files |
| `rusqlite` (bundled + fts5 features) | SQLite storage, compiled into the binary |
| `mcp-server` (or hand-rolled) | MCP protocol handling over stdio |
| `serde` + `serde_json` | JSON serialization for MCP messages and registry metadata |
| `reqwest` (rustls-tls) | HTTP client for fetching docs.rs and GitHub content |
| `tokio` (rt, macros, io-std) | Async runtime |
| `toml` | Parsing Cargo.toml |
| `tracing` + `tracing-subscriber` | Structured logging to stderr |

If no mature `mcp-server` crate exists, implement the stdio MCP protocol directly — it's JSON-RPC over stdin/stdout.

## Binary Interface

```
illu-rs                    # index cwd, start MCP server
illu-rs --repo /path/to   # index specified path, start MCP server
```

No subcommands, no config file. Stdout is reserved for MCP (JSON-RPC). All logging goes to stderr via `tracing`.

## Error Handling

- **Startup errors** (no Cargo.toml, SQLite failures): log to stderr, exit non-zero. Don't start a broken server.
- **Indexing errors** (file parse failure, doc fetch failure): log warning, skip item, continue. One missing doc shouldn't block the index.
- **Tool call errors** (bad input, symbol not found): return MCP error responses with clear messages.
- **Network errors during lazy fetch**: return partial result with error note. Don't crash the server.

Principle: the server always starts and always responds. Missing data is reported honestly, never silently omitted.

## Project Structure

```
illu-rs/
├── Cargo.toml
├── src/
│   ├── main.rs              # Entry point: parse args, run indexer, start server
│   ├── indexer/
│   │   ├── mod.rs            # Orchestrates the 4-phase pipeline
│   │   ├── dependencies.rs   # Phase 1: Cargo.toml/lock parsing
│   │   ├── parser.rs         # Phase 2: Tree-sitter AST extraction
│   │   ├── docs.rs           # Phase 3: docs.rs + GitHub fetching
│   │   └── store.rs          # Phase 4: SQLite schema + writes
│   ├── server/
│   │   ├── mod.rs            # MCP server setup + stdio transport
│   │   └── tools/
│   │       ├── mod.rs        # Tool registration
│   │       ├── query.rs      # Full-text search tool
│   │       ├── context.rs    # Symbol 360-degree view tool
│   │       ├── impact.rs     # Blast radius tool
│   │       └── docs.rs       # Dependency docs tool
│   └── db.rs                 # SQLite connection, migrations, queries
```

## Claude Integration

### MCP Configuration

Added to Claude Code's MCP config (project-level `.mcp.json`):

```json
{
  "mcpServers": {
    "illu": {
      "command": "illu-rs",
      "args": []
    }
  }
}
```

Claude Code starts the binary and communicates over stdio. The server auto-detects the repo from the inherited working directory.

### Claude Skill

A skill file auto-generated during indexing, installed to `.claude/skills/`, that teaches Claude when to use illu tools:

- Before reading unfamiliar code: use `context` to understand a symbol's full picture
- Before modifying code: use `impact` to check blast radius
- When needing a dependency's API: use `docs` instead of web searching
- When exploring the codebase broadly: use `query` to find relevant symbols and docs

The skill lists direct dependencies by name so Claude knows what's available. Example: "This repo uses `axiom-rs`, `tokio`, `serde`. Use the `docs` tool for API questions about these instead of searching the web."

## Scope

- Rust only at launch (extensible architecture via Tree-sitter)
- Single repo per server instance
- No human-facing interface
- No config file (YAGNI)
