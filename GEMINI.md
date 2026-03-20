# GEMINI.md

<!-- illu:start -->
## Code Intelligence (illu)

This repo is indexed by illu. **Use illu tools as your first step** — before reading files, before grep, before guessing at code structure.

### When to use illu

- **Starting any task**: `@illu query` the relevant symbols to understand what exists
- **Before modifying a function/struct/trait**: `@illu impact` to see what depends on it
- **Debugging or tracing issues**: `@illu context` to get the full definition and references
- **Using an external crate**: `@illu docs` to check how it's used in this project
- **Before reading files**: query first — illu tells you exactly where things are
- **Finding call paths**: `@illu callpath` to trace how one symbol reaches another
- **Dead code detection**: `@illu unused` to find unreferenced symbols
- **Index health**: `@illu freshness` to check if the index is current

### Commands

| User types | MCP tool | Params |
|------------|----------|--------|
| `@illu query <term>` | `mcp_illu_query` | `query: "<term>"` |
| `@illu query <term> --scope <s>` | `mcp_illu_query` | `query: "<term>", scope: "<s>"` |
| `@illu context <symbol>` | `mcp_illu_context` | `symbol_name: "<symbol>"` |
| `@illu context Type::method` | `mcp_illu_context` | `symbol_name: "Type::method"` |
| `@illu context <symbol> --file <f>` | `mcp_illu_context` | `symbol_name: "<symbol>", file: "<f>"` |
| `@illu impact <symbol>` | `mcp_illu_impact` | `symbol_name: "<symbol>"` |
| `@illu impact <symbol> --depth 1` | `mcp_illu_impact` | `symbol_name: "<symbol>", depth: 1` |
| `@illu docs <dep>` | `mcp_illu_docs` | `dependency: "<dep>"` |
| `@illu docs <dep> --topic <t>` | `mcp_illu_docs` | `dependency: "<dep>", topic: "<t>"` |
| `@illu callpath <from> <to>` | `mcp_illu_callpath` | `from: "<from>", to: "<to>"` |
| `@illu batch_context <sym1> <sym2>` | `mcp_illu_batch_context` | `symbols: ["<sym1>", "<sym2>"]` |
| `@illu unused` | `mcp_illu_unused` | |
| `@illu unused --path src/server/` | `mcp_illu_unused` | `path: "src/server/"` |
| `@illu freshness` | `mcp_illu_freshness` | |
| `@illu crate_graph` | `mcp_illu_crate_graph` | |

### Workflow rules

1. **Locate before you read**: `@illu query` or `@illu context` to find the right file:line, then Read only what you need
2. **Impact before you change**: always run `@illu impact` before modifying any public symbol
3. **Chain tools**: `@illu query` to find candidates → `@illu context` for the one you need → `@illu impact` before changing it
<!-- illu:end -->
