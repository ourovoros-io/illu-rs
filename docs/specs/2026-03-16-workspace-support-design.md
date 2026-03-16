# Workspace Support Design

## Problem

illu-rs currently indexes a single crate by looking at `{repo}/src/`. Rust workspaces (multiple crates in one repo) are common, and the tool fails to:

1. Find source files in member crates (e.g., `hcfs-server/src/`)
2. Resolve `{ workspace = true }` dependency declarations
3. Understand inter-crate dependencies (e.g., `hcfs-server` depends on `hcfs-shared`)
4. Track cross-crate symbol references for impact analysis

## Design

### Workspace Detection & Member Discovery

When `index_repo` receives a path, it checks if `Cargo.toml` contains a `[workspace]` section.

- **Workspace root**: Parse `workspace.members` (supporting globs like `crates/*`). For each member, locate its `Cargo.toml` and `src/` directory.
- **Single crate** (no `[workspace]`): Behave exactly as today — one crate, one `src/` directory.

Each member is stored as a row in a new `crates` table.

### Schema Changes

```sql
CREATE TABLE crates (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    path TEXT NOT NULL,
    is_workspace_root INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE crate_deps (
    source_crate_id INTEGER NOT NULL REFERENCES crates(id),
    target_crate_id INTEGER NOT NULL REFERENCES crates(id),
    PRIMARY KEY (source_crate_id, target_crate_id)
);

ALTER TABLE files ADD COLUMN crate_id INTEGER REFERENCES crates(id);
```

For single-crate mode, one row in `crates`, all files linked to it. No behavioral change.

### Workspace Dependency Resolution

**Inter-crate dependencies**: Parse each member's `Cargo.toml` for `path = "..."` style dependencies. Match path targets to workspace members. Store in `crate_deps` table.

**Workspace-inherited external deps**: When a member has `dep = { workspace = true }`, look up the version and features from the root `[workspace.dependencies]`. Merge into the existing `dependencies` table with the resolved version.

**Cargo.lock**: Use the single workspace-root `Cargo.lock`. Same as today.

### Cross-Crate Indexing & Symbol References

**File indexing**: Walk `src/` for every workspace member. File paths are stored relative to workspace root (e.g., `hcfs-server/src/main.rs`). Each file is linked to its crate via `files.crate_id`.

**Symbol references**: After indexing all members' symbols, run the ref extraction pass across all files. Since all symbols are in one unified DB, a function in `hcfs-server` that uses `SharedType` from `hcfs-shared` naturally matches against the global known_symbols set.

No changes to the parser or ref extraction logic — they already work on arbitrary source files.

### Impact Tool Enhancement

The impact tool adds a crate-level summary before symbol-level detail:

```
## Impact Analysis: SharedType

### Affected Crates
- hcfs-shared (defined here)
- hcfs-server (direct dependency)
- hcfs-client (direct dependency)
- hcfs-client-cli (transitive via hcfs-client)

### Depth 1
- **validate_shared** (hcfs-server/src/lib.rs)
- **build_request** (hcfs-client/src/client.rs)
```

The crate summary is derived by:
1. Find which crate the symbol lives in (via `files.crate_id`)
2. Walk `crate_deps` transitively to find all dependent crates
3. Distinguish actual impact (has symbol-level refs) from potential impact (crate dependency exists but no direct symbol refs found)

### Unified Database

One DB at the workspace root (`.illu/index.db`) containing all members. This is required for cross-crate queries and impact analysis to work.

### What Doesn't Change

- Parser (`parse_rust_source`, `extract_refs`) — works on any source file
- Store functions (`store_symbols`, `store_dependencies`) — just called more times
- MCP server and tool handlers — query the same DB
- Query, context, and docs tools — work against the unified index
- Single-crate repos — detected automatically, same behavior as today
