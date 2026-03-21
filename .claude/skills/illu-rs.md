# illu-rs Code Intelligence

This project is indexed by illu-rs. Use the following MCP tools to explore the codebase and its dependencies.

## Tools

- **query** — Search symbols, docs, or files. Pass `scope` (symbols/docs/files/all).
- **context** — Get full context for a symbol: doc comments, definition, source body, struct fields, trait implementations, and callees.
- **impact** — Analyze the impact of changing a symbol by finding all transitive dependents.
- **diff_impact** — Analyze impact of git changes. Shows modified symbols and their downstream dependents.
- **docs** — Get documentation for a dependency, optionally filtered by topic.
- **overview** — Get a structural overview of all public symbols under a file path prefix.

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
