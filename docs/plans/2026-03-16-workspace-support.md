# Workspace Support Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Support Cargo workspaces — index all member crates into one DB, resolve workspace-inherited deps, track inter-crate dependencies, and show crate-level impact in the impact tool.

**Architecture:** Detect workspace via `[workspace]` in root `Cargo.toml`. Create a `crates` table and `crate_deps` table. Walk each member's `src/` directory. Thread `crate_id` through file storage. Enhance impact tool with crate-level summary. Single-crate repos auto-detected, no behavioral change.

**Tech Stack:** Rust, rusqlite, toml, walkdir, tree-sitter (unchanged)

**Spec:** `docs/specs/2026-03-16-workspace-support-design.md`

---

### Task 1: Schema — Add `crates` and `crate_deps` Tables

**Files:**
- Modify: `src/db.rs` (migrate, clear_index, new methods)

- [ ] **Step 1: Write failing tests for crate storage**

Add to `src/db.rs` tests:

```rust
#[test]
fn test_insert_and_get_crate() {
    let db = Database::open_in_memory().unwrap();
    let id = db.insert_crate("hcfs-server", "hcfs-server", false).unwrap();
    assert!(id > 0);
    let c = db.get_crate_by_name("hcfs-server").unwrap().unwrap();
    assert_eq!(c.name, "hcfs-server");
    assert_eq!(c.path, "hcfs-server");
}

#[test]
fn test_insert_crate_dep() {
    let db = Database::open_in_memory().unwrap();
    let shared = db.insert_crate("shared", "shared", false).unwrap();
    let server = db.insert_crate("server", "server", false).unwrap();
    db.insert_crate_dep(server, shared).unwrap();
    let deps = db.get_crate_dependents(shared).unwrap();
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0].name, "server");
}

#[test]
fn test_insert_file_with_crate() {
    let db = Database::open_in_memory().unwrap();
    let crate_id = db.insert_crate("mylib", "mylib", false).unwrap();
    let file_id = db.insert_file_with_crate("mylib/src/lib.rs", "hash", crate_id).unwrap();
    assert!(file_id > 0);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -- db::tests::test_insert_and_get_crate db::tests::test_insert_crate_dep db::tests::test_insert_file_with_crate`
Expected: FAIL (methods don't exist)

- [ ] **Step 3: Add schema migration for new tables**

In `src/db.rs`, add to `migrate()` after the existing `CREATE TABLE` statements:

```rust
CREATE TABLE IF NOT EXISTS crates (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    path TEXT NOT NULL,
    is_workspace_root INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS crate_deps (
    source_crate_id INTEGER NOT NULL REFERENCES crates(id),
    target_crate_id INTEGER NOT NULL REFERENCES crates(id),
    PRIMARY KEY (source_crate_id, target_crate_id)
);
```

Add `crate_id INTEGER REFERENCES crates(id)` column to the `files` table.

Update `clear_index()` to also delete from `crate_deps` and `crates` (before `files`).

- [ ] **Step 4: Add `StoredCrate` struct and DB methods**

```rust
#[derive(Debug)]
pub struct StoredCrate {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub is_workspace_root: bool,
}

pub fn insert_crate(&self, name: &str, path: &str, is_workspace_root: bool) -> SqlResult<i64>
pub fn get_crate_by_name(&self, name: &str) -> SqlResult<Option<StoredCrate>>
pub fn insert_crate_dep(&self, source_crate_id: i64, target_crate_id: i64) -> SqlResult<()>
pub fn get_crate_dependents(&self, crate_id: i64) -> SqlResult<Vec<StoredCrate>>
pub fn insert_file_with_crate(&self, path: &str, hash: &str, crate_id: i64) -> SqlResult<i64>
pub fn get_crate_for_file(&self, file_path: &str) -> SqlResult<Option<StoredCrate>>
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -- db::tests`
Expected: PASS

- [ ] **Step 6: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: clean

- [ ] **Step 7: Commit**

```bash
git add src/db.rs
git commit -m "Add crates and crate_deps schema for workspace support"
```

---

### Task 2: Workspace Detection & Member Discovery

**Files:**
- Create: `src/indexer/workspace.rs`
- Modify: `src/indexer/mod.rs` (add `pub mod workspace;`)

- [ ] **Step 1: Write failing tests**

Create `src/indexer/workspace.rs` with tests:

```rust
#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn test_detect_workspace() {
        let toml = r#"
[workspace]
members = ["crate-a", "crate-b"]
"#;
        let info = parse_workspace_toml(toml).unwrap();
        assert!(info.is_workspace);
        assert_eq!(info.members, vec!["crate-a", "crate-b"]);
    }

    #[test]
    fn test_detect_single_crate() {
        let toml = r#"
[package]
name = "my-crate"
version = "0.1.0"
"#;
        let info = parse_workspace_toml(toml).unwrap();
        assert!(!info.is_workspace);
        assert!(info.members.is_empty());
    }

    #[test]
    fn test_workspace_deps() {
        let toml = r#"
[workspace]
members = ["app"]

[workspace.dependencies]
serde = { version = "1.0", features = ["derive"] }
tokio = "1"
"#;
        let info = parse_workspace_toml(toml).unwrap();
        assert_eq!(info.workspace_deps.len(), 2);
        let serde = info.workspace_deps.iter().find(|d| d.name == "serde").unwrap();
        assert_eq!(serde.version_req, "1.0");
        assert_eq!(serde.features, vec!["derive"]);
    }

    #[test]
    fn test_resolve_workspace_dep() {
        let ws_deps = vec![
            super::DirectDep {
                name: "serde".into(),
                version_req: "1.0".into(),
                features: vec!["derive".into()],
            },
        ];
        let member_toml = r#"
[package]
name = "my-app"
version = "0.1.0"

[dependencies]
serde = { workspace = true }
custom = "0.5"
"#;
        let resolved = resolve_member_deps(member_toml, &ws_deps).unwrap();
        let serde = resolved.iter().find(|d| d.name == "serde").unwrap();
        assert_eq!(serde.version_req, "1.0");
        let custom = resolved.iter().find(|d| d.name == "custom").unwrap();
        assert_eq!(custom.version_req, "0.5");
    }

    #[test]
    fn test_detect_inter_crate_deps() {
        let member_toml = r#"
[package]
name = "hcfs-server"
version = "0.1.0"

[dependencies]
hcfs-shared = { path = "../hcfs-shared" }
serde = { workspace = true }
"#;
        let path_deps = extract_path_deps(member_toml).unwrap();
        assert_eq!(path_deps.len(), 1);
        assert_eq!(path_deps[0].name, "hcfs-shared");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -- indexer::workspace::tests`
Expected: FAIL

- [ ] **Step 3: Implement workspace parsing**

In `src/indexer/workspace.rs`:

```rust
use crate::indexer::dependencies::DirectDep;
use serde::Deserialize;
use std::collections::HashMap;

pub struct WorkspaceInfo {
    pub is_workspace: bool,
    pub members: Vec<String>,
    pub workspace_deps: Vec<DirectDep>,
}

pub struct PathDep {
    pub name: String,
    pub path: String,
}

pub fn parse_workspace_toml(content: &str) -> Result<WorkspaceInfo, toml::de::Error>
pub fn resolve_member_deps(member_toml: &str, ws_deps: &[DirectDep]) -> Result<Vec<DirectDep>, toml::de::Error>
pub fn extract_path_deps(member_toml: &str) -> Result<Vec<PathDep>, toml::de::Error>
```

`parse_workspace_toml`: Deserialize looking for `[workspace]` section. If present, extract `members` array and `[workspace.dependencies]` (same logic as `parse_cargo_toml` but from the workspace section).

`resolve_member_deps`: Parse a member's `Cargo.toml`. For deps with `workspace = true`, look up in `ws_deps`. For deps with explicit versions, keep as-is. Skip `path = "..."` deps (those are inter-crate, not external).

`extract_path_deps`: Parse a member's `Cargo.toml`, return deps that have a `path` key.

- [ ] **Step 4: Add `pub mod workspace;` to `src/indexer/mod.rs`**

- [ ] **Step 5: Run tests**

Run: `cargo test -- indexer::workspace::tests`
Expected: PASS

- [ ] **Step 6: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: clean

- [ ] **Step 7: Commit**

```bash
git add src/indexer/workspace.rs src/indexer/mod.rs
git commit -m "Add workspace detection and member dependency resolution"
```

---

### Task 3: Refactor Indexer to Support Workspaces

**Files:**
- Modify: `src/indexer/mod.rs` (refactor `index_repo`)

This is the core change. `index_repo` detects workspace vs single-crate and dispatches accordingly.

- [ ] **Step 1: Write failing test for workspace indexing**

Add to `src/indexer/mod.rs` tests:

```rust
#[test]
fn test_index_workspace() {
    let dir = tempfile::TempDir::new().unwrap();

    // Create workspace root
    std::fs::write(
        dir.path().join("Cargo.toml"),
        r#"
[workspace]
members = ["shared", "app"]

[workspace.dependencies]
serde = "1.0"
"#,
    )
    .unwrap();

    std::fs::write(
        dir.path().join("Cargo.lock"),
        r#"
[[package]]
name = "serde"
version = "1.0.210"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "shared"
version = "0.1.0"

[[package]]
name = "app"
version = "0.1.0"
dependencies = ["shared", "serde"]
"#,
    )
    .unwrap();

    // Create shared crate
    let shared_dir = dir.path().join("shared");
    std::fs::create_dir_all(shared_dir.join("src")).unwrap();
    std::fs::write(
        shared_dir.join("Cargo.toml"),
        r#"
[package]
name = "shared"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();
    std::fs::write(
        shared_dir.join("src").join("lib.rs"),
        "pub struct SharedType { pub value: i32 }\n",
    )
    .unwrap();

    // Create app crate that depends on shared
    let app_dir = dir.path().join("app");
    std::fs::create_dir_all(app_dir.join("src")).unwrap();
    std::fs::write(
        app_dir.join("Cargo.toml"),
        r#"
[package]
name = "app"
version = "0.1.0"
edition = "2021"

[dependencies]
shared = { path = "../shared" }
serde = { workspace = true }
"#,
    )
    .unwrap();
    std::fs::write(
        app_dir.join("src").join("main.rs"),
        r#"
pub fn use_shared() -> SharedType {
    SharedType { value: 42 }
}
"#,
    )
    .unwrap();

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
        skip_doc_fetch: true,
    };
    index_repo(&db, &config).unwrap();

    // Both crates' symbols indexed
    let shared_syms = db.search_symbols("SharedType").unwrap();
    assert!(!shared_syms.is_empty(), "SharedType should be indexed");

    let app_syms = db.search_symbols("use_shared").unwrap();
    assert!(!app_syms.is_empty(), "use_shared should be indexed");

    // Inter-crate dependency tracked
    let shared_crate = db.get_crate_by_name("shared").unwrap().unwrap();
    let dependents = db.get_crate_dependents(shared_crate.id).unwrap();
    assert_eq!(dependents.len(), 1);
    assert_eq!(dependents[0].name, "app");

    // Workspace dep resolved
    let serde_dep = db.get_dependency_by_name("serde").unwrap();
    assert!(serde_dep.is_some());

    // Cross-crate symbol ref exists
    let refs_result = db.search_symbols("SharedType").unwrap();
    assert!(!refs_result.is_empty());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -- indexer::tests::test_index_workspace`
Expected: FAIL

- [ ] **Step 3: Refactor `index_repo`**

Extract the current single-crate source walking into a helper `index_crate_sources(db, repo_root, crate_src_dir, crate_id)`. Then restructure `index_repo`:

```rust
pub fn index_repo(db: &Database, config: &IndexConfig) -> Result<(), Box<dyn std::error::Error>> {
    db.clear_index()?;

    let cargo_toml = std::fs::read_to_string(config.repo_path.join("Cargo.toml"))?;
    let ws_info = workspace::parse_workspace_toml(&cargo_toml)?;

    if ws_info.is_workspace {
        index_workspace(db, config, &ws_info)?;
    } else {
        index_single_crate(db, config)?;
    }

    // Phases shared by both paths:
    // - Extract symbol refs (across all indexed files)
    // - Generate skill file
    // - Update metadata
    extract_all_symbol_refs(db, config)?;
    generate_skill_file(db, config)?;
    update_metadata(db, config)?;

    Ok(())
}
```

`index_workspace`:
1. Parse workspace-root `Cargo.lock` for locked versions
2. For each member in `ws_info.members`:
   a. Read member `Cargo.toml`
   b. Resolve deps (workspace-inherited + explicit)
   c. Store external deps via `store_dependencies`
   d. Insert crate into `crates` table
   e. Extract path deps, match to other members, store in `crate_deps`
   f. Walk member's `src/` dir, store files with `crate_id`, parse and store symbols

`index_single_crate`:
1. Same as current logic, but insert one row in `crates` and use `insert_file_with_crate`

Extract helpers: `extract_all_symbol_refs(db, config)`, `generate_skill_file(db, config)`, `update_metadata(db, config)` from the current inline code.

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: all PASS (both old single-crate tests and new workspace test)

- [ ] **Step 5: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: clean

- [ ] **Step 6: Commit**

```bash
git add src/indexer/mod.rs
git commit -m "Refactor indexer to support workspace and single-crate modes"
```

---

### Task 4: Impact Tool — Crate-Level Summary

**Files:**
- Modify: `src/server/tools/impact.rs`
- Modify: `src/db.rs` (add `get_crate_for_file`, `get_transitive_crate_dependents`)

- [ ] **Step 1: Write failing test**

Add to `src/server/tools/impact.rs` tests:

```rust
#[test]
fn test_impact_shows_affected_crates() {
    let db = Database::open_in_memory().unwrap();

    // Set up two crates
    let shared_id = db.insert_crate("shared", "shared", false).unwrap();
    let app_id = db.insert_crate("app", "app", false).unwrap();
    db.insert_crate_dep(app_id, shared_id).unwrap();

    // Add symbols in different crates
    let shared_file = db.insert_file_with_crate("shared/src/lib.rs", "h1", shared_id).unwrap();
    let app_file = db.insert_file_with_crate("app/src/main.rs", "h2", app_id).unwrap();

    store_symbols(&db, shared_file, &[Symbol {
        name: "SharedType".into(),
        kind: SymbolKind::Struct,
        visibility: Visibility::Public,
        file_path: "shared/src/lib.rs".into(),
        line_start: 1, line_end: 3,
        signature: "pub struct SharedType".into(),
    }]).unwrap();

    store_symbols(&db, app_file, &[Symbol {
        name: "use_it".into(),
        kind: SymbolKind::Function,
        visibility: Visibility::Public,
        file_path: "app/src/main.rs".into(),
        line_start: 1, line_end: 5,
        signature: "pub fn use_it()".into(),
    }]).unwrap();

    // Create cross-crate ref
    let shared_sym_id = db.get_symbol_id("SharedType", "shared/src/lib.rs").unwrap().unwrap();
    let app_sym_id = db.get_symbol_id("use_it", "app/src/main.rs").unwrap().unwrap();
    db.insert_symbol_ref(app_sym_id, shared_sym_id, "type_ref").unwrap();

    let result = handle_impact(&db, "SharedType").unwrap();
    assert!(result.contains("Affected Crates"), "should have crate summary");
    assert!(result.contains("shared"), "should mention shared crate");
    assert!(result.contains("app"), "should mention app crate");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -- server::tools::impact::tests::test_impact_shows_affected_crates`
Expected: FAIL

- [ ] **Step 3: Add DB helper methods**

In `src/db.rs`:

```rust
pub fn get_transitive_crate_dependents(&self, crate_id: i64) -> SqlResult<Vec<StoredCrate>> {
    // Recursive CTE on crate_deps
    let mut stmt = self.conn.prepare(
        "WITH RECURSIVE deps(id, name, path, depth) AS (
            SELECT id, name, path, 0 FROM crates WHERE id = ?1
          UNION
            SELECT c.id, c.name, c.path, deps.depth + 1
            FROM deps
            JOIN crate_deps cd ON cd.target_crate_id = deps.id
            JOIN crates c ON c.id = cd.source_crate_id
            WHERE deps.depth < 10
        )
        SELECT DISTINCT id, name, path FROM deps WHERE id != ?1"
    )?;
    // collect into Vec<StoredCrate>
}
```

- [ ] **Step 4: Enhance `handle_impact` in `src/server/tools/impact.rs`**

Before the symbol-level recursive CTE, add a crate-level section:

1. Look up which crate the symbol's file belongs to (via `get_crate_for_file`)
2. Get transitive crate dependents via `get_transitive_crate_dependents`
3. Format as `### Affected Crates` section with the defining crate marked as "(defined here)"

Only emit this section if `crates` table has more than one row (skip for single-crate repos).

- [ ] **Step 5: Run tests**

Run: `cargo test`
Expected: all PASS

- [ ] **Step 6: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: clean

- [ ] **Step 7: Commit**

```bash
git add src/db.rs src/server/tools/impact.rs
git commit -m "Add crate-level impact summary for workspace projects"
```

---

### Task 5: Integration Test — Workspace End-to-End

**Files:**
- Modify: `tests/integration.rs` (add workspace test)

- [ ] **Step 1: Write workspace integration test**

```rust
#[test]
fn test_workspace_end_to_end() {
    let dir = tempfile::TempDir::new().unwrap();

    // Build a mini workspace: shared + app
    // shared defines SharedConfig struct
    // app uses SharedConfig in a function
    // Verify: query finds symbols from both crates,
    //         impact shows cross-crate dependents,
    //         crate_deps are populated

    // ... (full temp dir setup similar to Task 3 test)

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
        skip_doc_fetch: true,
    };
    index_repo(&db, &config).unwrap();

    // Query tool finds symbols across crates
    let result = query::handle_query(&db, "SharedConfig", Some("symbols")).unwrap();
    assert!(result.contains("SharedConfig"));

    // Impact shows crate-level summary
    let result = impact::handle_impact(&db, "SharedConfig").unwrap();
    assert!(result.contains("Affected Crates"));
    assert!(result.contains("app"));

    // Context tool works across crates
    let result = context::handle_context(&db, "SharedConfig").unwrap();
    assert!(result.contains("shared/src/lib.rs"));

    // Skill file lists workspace crates
    let skill_path = dir.path().join(".claude").join("skills").join("illu-rs.md");
    assert!(skill_path.exists());
}
```

- [ ] **Step 2: Run integration tests**

Run: `cargo test --test integration`
Expected: PASS

- [ ] **Step 3: Final checks**

Run: `cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test`
Expected: all clean, all tests pass

- [ ] **Step 4: Commit**

```bash
git add tests/integration.rs
git commit -m "Add workspace end-to-end integration test"
```

---

### Task 6: Live Validation Against hcfs

**Files:** None (manual testing)

- [ ] **Step 1: Build release binary**

Run: `cargo build --release`

- [ ] **Step 2: Run against hcfs workspace root**

```bash
rm -rf /root/hcfs/.illu
RUST_LOG=info ./target/release/illu-rs /root/hcfs < /dev/null 2>&1 | head -10
```

Verify: All 4 member crates indexed, symbol count > 0, ref count > 0.

- [ ] **Step 3: Check indexed data**

```bash
sqlite3 /root/hcfs/.illu/index.db "SELECT name, path FROM crates;"
sqlite3 /root/hcfs/.illu/index.db "SELECT COUNT(*) FROM symbols;"
sqlite3 /root/hcfs/.illu/index.db "SELECT COUNT(*) FROM symbol_refs;"
sqlite3 /root/hcfs/.illu/index.db "SELECT c1.name, c2.name FROM crate_deps cd JOIN crates c1 ON c1.id = cd.source_crate_id JOIN crates c2 ON c2.id = cd.target_crate_id;"
```

- [ ] **Step 4: Test impact tool with cross-crate symbol**

Send MCP requests to test that a symbol from `hcfs-shared` shows dependents in `hcfs-server` and `hcfs-client`.

- [ ] **Step 5: Commit any fixes found during validation**
