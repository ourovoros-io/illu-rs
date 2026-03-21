# illu-rs Code Intelligence

This project is indexed by illu-rs. Use the following MCP tools to explore the codebase and its dependencies.

## Tools (31 available)

### Search & Navigate

- **query** — Search symbols, docs, or files. Filters: kind, attribute, signature, path.
- **context** — Full symbol context: source, callers, callees, trait impls. Supports `Type::method`, `sections` filter, `exclude_tests`.
- **batch_context** — Context for multiple symbols in one call.
- **symbols_at** — Find symbols at a file:line location.
- **overview** — Public symbols under a path, grouped by file.
- **tree** — File/module hierarchy.

### Impact Analysis

- **impact** — Transitive dependents of a symbol (configurable depth).
- **diff_impact** — Batch impact for all symbols in a git diff.
- **test_impact** — Which tests break when changing a symbol.
- **crate_impact** — Which workspace crates are affected.

### Call Graph

- **callpath** — Shortest or all paths between two symbols.
- **neighborhood** — Callers/callees within N hops (list or tree format).
- **references** — Unified view: call sites, type usage, trait impls.
- **type_usage** — Where a type appears in signatures and struct fields.
- **file_graph** — File-level dependency graph.
- **graph_export** — DOT/Graphviz export of call or file graphs.

### Discovery & Audit

- **unused** — Symbols with no incoming references.
- **orphaned** — Symbols with no callers AND no test coverage.
- **boundary** — Public API vs internal-only classification for a module.
- **similar** — Functions with matching signatures and call patterns.
- **rename_plan** — All locations to update before renaming a symbol.
- **doc_coverage** — Undocumented symbols with coverage percentage.
- **hotspots** — Most-referenced, most-complex, and largest functions.
- **stats** — File/symbol counts, test coverage, top references.

### Dependencies & Git

- **docs** — Version-pinned dependency documentation, filterable by topic.
- **implements** — Trait/type implementation relationships.
- **crate_graph** — Workspace inter-crate dependency graph.
- **blame** — Git blame on a symbol's line range.
- **history** — Git commit history for a symbol, with optional diffs.
- **freshness** — Index staleness check.
- **health** — Index quality diagnosis.

## Direct Dependencies

- clap
- reqwest
- rmcp
- rusqlite
- serde
- serde_json
- tokio
- toml
- tracing
- tracing-subscriber
- tree-sitter
- tree-sitter-rust
- walkdir
