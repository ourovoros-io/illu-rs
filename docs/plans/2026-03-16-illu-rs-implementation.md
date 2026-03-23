# illu-rs Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a Rust MCP server that indexes a Rust repo (code + version-pinned dependency docs) and serves code intelligence to Claude over stdio.

**Architecture:** Single binary, two phases — indexer (runs at startup) and server (runs forever after). SQLite with FTS5 is the storage layer. Tree-sitter parses Rust code. rmcp handles MCP protocol. Docs fetched from docs.rs and GitHub at pinned versions.

**Tech Stack:** Rust (2024 edition), rmcp 0.16, tree-sitter 0.26, rusqlite 0.39 (bundled), reqwest 0.13, tokio, serde, toml, tracing.

---

### Task 1: Project Setup — Cargo.toml and Lints

**Files:**
- Modify: `Cargo.toml`
- Create: `src/main.rs` (replace stub)

**Step 1: Set up Cargo.toml with all dependencies and clippy lints**

```toml
[package]
name = "illu-rs"
version = "0.1.0"
edition = "2024"

[dependencies]
rmcp = { version = "0.16", features = ["server", "transport-io"] }
tree-sitter = "0.26"
tree-sitter-rust = "0.23"
rusqlite = { version = "0.39", features = ["bundled"] }
reqwest = { version = "0.13", features = ["rustls", "json"] }
tokio = { version = "1", features = ["rt", "macros", "io-std"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[lints.clippy]
pedantic = { level = "warn", priority = -1 }
unwrap_used = "deny"
expect_used = "warn"
panic = "deny"
panic_in_result_fn = "deny"
unimplemented = "deny"
allow_attributes = "deny"
dbg_macro = "deny"
todo = "deny"
print_stdout = "deny"
print_stderr = "deny"
await_holding_lock = "deny"
large_futures = "deny"
exit = "deny"
mem_forget = "deny"
module_name_repetitions = "allow"
similar_names = "allow"
```

**Step 2: Replace main.rs with a basic entry point**

```rust
fn main() {
    eprintln!("illu-rs starting...");
}
```

**Step 3: Verify it compiles**

Run: `cargo build`
Expected: compiles successfully, all deps download

**Step 4: Run clippy**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings

**Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/main.rs
git commit -m "Set up dependencies and clippy lints"
```

---

### Task 2: SQLite Database Layer

**Files:**
- Create: `src/db.rs`
- Modify: `src/main.rs` (add `mod db;`)

**Step 1: Write tests for DB creation and schema migration**

At the bottom of `src/db.rs`, write tests that:
- Create an in-memory DB, run migrations, verify all tables exist
- Verify FTS5 virtual tables exist
- Insert and query a metadata row

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_creates_schema() {
        let db = Database::open_in_memory().unwrap();
        // Verify tables exist by querying sqlite_master
        let tables: Vec<String> = db.conn.prepare(
            "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"
        ).unwrap()
        .query_map([], |row| row.get(0)).unwrap()
        .filter_map(Result::ok)
        .collect();

        assert!(tables.contains(&"metadata".to_string()));
        assert!(tables.contains(&"files".to_string()));
        assert!(tables.contains(&"symbols".to_string()));
        assert!(tables.contains(&"symbol_refs".to_string()));
        assert!(tables.contains(&"dependencies".to_string()));
        assert!(tables.contains(&"docs".to_string()));
    }

    #[test]
    fn test_metadata_roundtrip() {
        let db = Database::open_in_memory().unwrap();
        db.set_metadata("/tmp/repo", "abc123").unwrap();
        let hash = db.get_commit_hash("/tmp/repo").unwrap();
        assert_eq!(hash, Some("abc123".to_string()));
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -- db::tests`
Expected: FAIL — `Database` not defined

**Step 3: Implement `Database` struct with schema creation**

Implement in `src/db.rs`:
- `Database` struct wrapping `rusqlite::Connection`
- `open(path)` — opens or creates `.illu/index.db`
- `open_in_memory()` — for tests
- `migrate()` — creates all tables and FTS5 virtual tables
- `set_metadata()` / `get_commit_hash()` — metadata read/write

Schema tables: `metadata`, `files`, `symbols`, `symbol_refs`, `dependencies`, `docs`, `docs_fts` (FTS5), `symbols_fts` (FTS5).

**Step 4: Run tests to verify they pass**

Run: `cargo test -- db::tests`
Expected: PASS

**Step 5: Run clippy**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings

**Step 6: Commit**

```bash
git add src/db.rs src/main.rs
git commit -m "Add SQLite database layer with schema and FTS5"
```

---

### Task 3: Dependency Parsing (Phase 1)

**Files:**
- Create: `src/indexer/mod.rs`
- Create: `src/indexer/dependencies.rs`
- Modify: `src/main.rs` (add `mod indexer;`)

**Step 1: Write tests for Cargo.toml and Cargo.lock parsing**

In `src/indexer/dependencies.rs`, write tests that:
- Parse a minimal `Cargo.toml` string and extract direct dependencies with version specs
- Parse a minimal `Cargo.lock` string and extract exact resolved versions
- Classify dependencies as direct vs transitive
- Extract repository URL from crate metadata

Use inline TOML strings as test fixtures, not real files.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cargo_toml_direct_deps() {
        let toml_content = r#"
[package]
name = "test-project"
version = "0.1.0"

[dependencies]
serde = { version = "1.0", features = ["derive"] }
tokio = "1"
"#;
        let deps = parse_cargo_toml(toml_content).unwrap();
        assert_eq!(deps.len(), 2);
        assert!(deps.iter().any(|d| d.name == "serde"));
        assert!(deps.iter().any(|d| d.name == "tokio"));
    }

    #[test]
    fn test_parse_cargo_lock_versions() {
        let lock_content = r#"
[[package]]
name = "serde"
version = "1.0.210"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "serde_derive"
version = "1.0.210"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#;
        let locked = parse_cargo_lock(lock_content).unwrap();
        assert_eq!(locked.len(), 2);
        assert_eq!(locked[0].version, "1.0.210");
    }

    #[test]
    fn test_classify_direct_vs_transitive() {
        let direct = vec![
            DirectDep { name: "serde".into(), version_req: "1.0".into(), features: vec!["derive".into()] },
        ];
        let locked = vec![
            LockedDep { name: "serde".into(), version: "1.0.210".into(), source: None },
            LockedDep { name: "serde_derive".into(), version: "1.0.210".into(), source: None },
        ];
        let resolved = resolve_dependencies(&direct, &locked);
        assert!(resolved.iter().find(|d| d.name == "serde").unwrap().is_direct);
        assert!(!resolved.iter().find(|d| d.name == "serde_derive").unwrap().is_direct);
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -- indexer::dependencies::tests`
Expected: FAIL

**Step 3: Implement dependency parsing**

Implement in `src/indexer/dependencies.rs`:
- `DirectDep` struct: `name`, `version_req`, `features`
- `LockedDep` struct: `name`, `version`, `source`
- `ResolvedDep` struct: `name`, `version`, `is_direct`, `repository_url`, `features`
- `parse_cargo_toml(content: &str) -> Result<Vec<DirectDep>>` — parse `[dependencies]` table
- `parse_cargo_lock(content: &str) -> Result<Vec<LockedDep>>` — parse `[[package]]` entries
- `resolve_dependencies(direct, locked) -> Vec<ResolvedDep>` — classify and merge

**Step 4: Run tests**

Run: `cargo test -- indexer::dependencies::tests`
Expected: PASS

**Step 5: Commit**

```bash
git add src/indexer/
git commit -m "Add Cargo.toml and Cargo.lock dependency parsing"
```

---

### Task 4: Store Dependencies in SQLite

**Files:**
- Create: `src/indexer/store.rs`
- Modify: `src/db.rs` (add insert/query methods for dependencies)
- Modify: `src/indexer/mod.rs` (add `pub mod store;`)

**Step 1: Write tests for dependency storage**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::indexer::dependencies::ResolvedDep;

    #[test]
    fn test_store_and_retrieve_dependencies() {
        let db = Database::open_in_memory().unwrap();
        let deps = vec![
            ResolvedDep {
                name: "serde".into(),
                version: "1.0.210".into(),
                is_direct: true,
                repository_url: Some("https://github.com/serde-rs/serde".into()),
                features: vec!["derive".into()],
            },
        ];
        store_dependencies(&db, &deps).unwrap();
        let stored = db.get_direct_dependencies().unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].name, "serde");
        assert_eq!(stored[0].version, "1.0.210");
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -- indexer::store::tests`
Expected: FAIL

**Step 3: Implement storage functions**

- `store_dependencies(db, deps)` — batch insert into `dependencies` table
- Add `get_direct_dependencies()` and `get_dependency_by_name()` to `Database`

**Step 4: Run tests**

Run: `cargo test -- indexer::store::tests`
Expected: PASS

**Step 5: Commit**

```bash
git add src/indexer/store.rs src/db.rs src/indexer/mod.rs
git commit -m "Add dependency storage in SQLite"
```

---

### Task 5: Tree-sitter Code Parsing (Phase 2)

**Files:**
- Create: `src/indexer/parser.rs`
- Modify: `src/indexer/mod.rs` (add `pub mod parser;`)

**Step 1: Write tests for Rust source parsing**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_function() {
        let source = r#"
pub fn hello(name: &str) -> String {
    format!("Hello, {name}")
}
"#;
        let symbols = parse_rust_source(source, "src/lib.rs").unwrap();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "hello");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
        assert_eq!(symbols[0].visibility, Visibility::Public);
    }

    #[test]
    fn test_extract_struct_and_impl() {
        let source = r#"
pub struct Config {
    pub port: u16,
}

impl Config {
    pub fn new(port: u16) -> Self {
        Self { port }
    }
}
"#;
        let symbols = parse_rust_source(source, "src/config.rs").unwrap();
        let struct_sym = symbols.iter().find(|s| s.name == "Config").unwrap();
        assert_eq!(struct_sym.kind, SymbolKind::Struct);
        let method = symbols.iter().find(|s| s.name == "new").unwrap();
        assert_eq!(method.kind, SymbolKind::Function);
    }

    #[test]
    fn test_extract_use_statements() {
        let source = r#"
use std::collections::HashMap;
use serde::{Serialize, Deserialize};
"#;
        let symbols = parse_rust_source(source, "src/lib.rs").unwrap();
        let uses: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Use).collect();
        assert_eq!(uses.len(), 2);
    }

    #[test]
    fn test_extract_enum_and_trait() {
        let source = r#"
pub enum Color { Red, Green, Blue }
pub trait Drawable { fn draw(&self); }
"#;
        let symbols = parse_rust_source(source, "src/lib.rs").unwrap();
        assert!(symbols.iter().any(|s| s.name == "Color" && s.kind == SymbolKind::Enum));
        assert!(symbols.iter().any(|s| s.name == "Drawable" && s.kind == SymbolKind::Trait));
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -- indexer::parser::tests`
Expected: FAIL

**Step 3: Implement the parser**

Implement in `src/indexer/parser.rs`:
- `SymbolKind` enum: `Function`, `Struct`, `Enum`, `Trait`, `Impl`, `Use`, `Mod`
- `Visibility` enum: `Public`, `PublicCrate`, `Private`
- `Symbol` struct: `name`, `kind`, `visibility`, `file_path`, `line_start`, `line_end`, `signature`
- `parse_rust_source(source: &str, file_path: &str) -> Result<Vec<Symbol>>`

Use tree-sitter queries to extract `function_item`, `struct_item`, `enum_item`, `trait_item`, `impl_item`, `use_declaration`, `mod_item`. Check for `visibility_modifier` child nodes to determine pub/private.

**Step 4: Run tests**

Run: `cargo test -- indexer::parser::tests`
Expected: PASS

**Step 5: Commit**

```bash
git add src/indexer/parser.rs src/indexer/mod.rs
git commit -m "Add Tree-sitter Rust source parsing"
```

---

### Task 6: Store Symbols in SQLite

**Files:**
- Modify: `src/indexer/store.rs` (add symbol storage)
- Modify: `src/db.rs` (add symbol insert/query methods)

**Step 1: Write tests for symbol storage and FTS5 search**

```rust
#[test]
fn test_store_and_search_symbols() {
    let db = Database::open_in_memory().unwrap();
    let file_id = db.insert_file("src/lib.rs", "abc123").unwrap();
    let symbols = vec![
        Symbol {
            name: "parse_config".into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            file_path: "src/lib.rs".into(),
            line_start: 10,
            line_end: 25,
            signature: "pub fn parse_config(path: &Path) -> Result<Config>".into(),
        },
    ];
    store_symbols(&db, file_id, &symbols).unwrap();
    let results = db.search_symbols("parse").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "parse_config");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -- test_store_and_search_symbols`
Expected: FAIL

**Step 3: Implement symbol storage**

- `store_symbols(db, file_id, symbols)` — batch insert into `symbols` table + `symbols_fts`
- `db.insert_file(path, hash) -> Result<i64>` — insert into `files`, return id
- `db.search_symbols(query) -> Result<Vec<Symbol>>` — FTS5 search over symbol names and signatures
- `store_symbol_refs(db, refs)` — insert into `symbol_refs`

**Step 4: Run tests**

Run: `cargo test`
Expected: all PASS

**Step 5: Commit**

```bash
git add src/indexer/store.rs src/db.rs
git commit -m "Add symbol storage and FTS5 search"
```

---

### Task 7: Documentation Fetching (Phase 3)

**Files:**
- Create: `src/indexer/docs.rs`
- Modify: `src/indexer/mod.rs` (add `pub mod docs;`)

**Step 1: Write tests for doc fetching**

These tests need to be integration tests that hit real network. Mark them `#[ignore]` for CI, run manually.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_docs_rs_url() {
        let url = docs_rs_url("serde", "1.0.210");
        assert_eq!(url, "https://docs.rs/serde/1.0.210/serde/");
    }

    #[test]
    fn test_build_github_readme_url() {
        let url = github_readme_url(
            "https://github.com/serde-rs/serde",
            "v1.0.210"
        );
        assert_eq!(
            url,
            "https://raw.githubusercontent.com/serde-rs/serde/v1.0.210/README.md"
        );
    }

    #[test]
    fn test_parse_github_repo_url() {
        let (owner, repo) = parse_github_url(
            "https://github.com/serde-rs/serde"
        ).unwrap();
        assert_eq!(owner, "serde-rs");
        assert_eq!(repo, "serde");
    }

    #[tokio::test]
    #[ignore] // hits network
    async fn test_fetch_docs_rs_content() {
        let content = fetch_docs_rs("serde", "1.0.210").await.unwrap();
        assert!(!content.is_empty());
        assert!(content.contains("serde") || content.contains("Serde"));
    }

    #[tokio::test]
    #[ignore] // hits network
    async fn test_fetch_github_readme() {
        let content = fetch_github_readme(
            "https://github.com/serde-rs/serde",
            "1.0.210"
        ).await.unwrap();
        assert!(!content.is_empty());
    }
}
```

**Step 2: Run unit tests to verify they fail**

Run: `cargo test -- indexer::docs::tests -- --skip ignored`
Expected: FAIL (non-network tests)

**Step 3: Implement doc fetching**

Implement in `src/indexer/docs.rs`:
- `docs_rs_url(name, version) -> String`
- `github_readme_url(repo_url, version) -> String`
- `parse_github_url(url) -> Result<(String, String)>` — extract owner/repo
- `fetch_docs_rs(name, version) -> Result<String>` — HTTP GET, extract text content from HTML
- `fetch_github_readme(repo_url, version) -> Result<String>` — try tag patterns `v{version}`, `{version}`, fall back to default branch
- `fetch_crate_metadata(name, version) -> Result<CrateMetadata>` — hit crates.io API for repository URL
- `DocContent` struct: `source` (DocsRs/GitHubReadme), `content`, `dependency_name`, `version`

Use `reqwest` with a shared client. Set a user-agent header (crates.io requires it). Handle 404s gracefully — return `Ok(None)` not an error.

**Step 4: Run unit tests**

Run: `cargo test -- indexer::docs::tests`
Expected: unit tests PASS (network tests skipped unless `--ignored`)

**Step 5: Commit**

```bash
git add src/indexer/docs.rs src/indexer/mod.rs
git commit -m "Add version-pinned documentation fetching"
```

---

### Task 8: Store Docs in SQLite

**Files:**
- Modify: `src/indexer/store.rs` (add doc storage)
- Modify: `src/db.rs` (add doc insert/query methods)

**Step 1: Write tests**

```rust
#[test]
fn test_store_and_search_docs() {
    let db = Database::open_in_memory().unwrap();
    let dep_id = db.insert_dependency("serde", "1.0.210", true, None).unwrap();
    store_doc(&db, dep_id, DocSource::DocsRs, "Serde is a serialization framework").unwrap();
    let results = db.search_docs("serialization").unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].content.contains("serialization"));
}
```

**Step 2: Run test, verify fail, implement, verify pass**

Add to `db.rs`:
- `insert_dependency(name, version, is_direct, repo_url) -> Result<i64>`
- `store_doc(db, dep_id, source, content)`
- `search_docs(query) -> Result<Vec<DocResult>>` — FTS5 search
- `get_docs_for_dependency(name) -> Result<Vec<DocResult>>`

**Step 3: Commit**

```bash
git add src/indexer/store.rs src/db.rs
git commit -m "Add documentation storage with FTS5 search"
```

---

### Task 9: Indexer Orchestrator

**Files:**
- Modify: `src/indexer/mod.rs` (implement pipeline orchestration)

**Step 1: Write test for the full pipeline**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;
    use std::fs;

    #[test]
    fn test_index_pipeline_offline() {
        let dir = TempDir::new().unwrap();
        let src_dir = dir.path().join("src");
        fs::create_dir_all(&src_dir).unwrap();

        // Write a minimal Cargo.toml (no deps to avoid network)
        fs::write(dir.path().join("Cargo.toml"), r#"
[package]
name = "test"
version = "0.1.0"
edition = "2021"
"#).unwrap();

        fs::write(dir.path().join("Cargo.lock"), r#"
[[package]]
name = "test"
version = "0.1.0"
"#).unwrap();

        fs::write(src_dir.join("main.rs"), r#"
pub fn hello() -> &'static str { "hello" }
"#).unwrap();

        let db = Database::open_in_memory().unwrap();
        let config = IndexConfig {
            repo_path: dir.path().to_path_buf(),
            skip_doc_fetch: true,
        };
        index_repo(&db, &config).unwrap();

        let symbols = db.search_symbols("hello").unwrap();
        assert_eq!(symbols.len(), 1);
    }
}
```

Note: add `tempfile` as a dev-dependency in Cargo.toml.

**Step 2: Run test, verify fail**

Run: `cargo test -- indexer::tests`
Expected: FAIL

**Step 3: Implement orchestrator**

```rust
pub struct IndexConfig {
    pub repo_path: PathBuf,
    pub skip_doc_fetch: bool,
}

pub fn index_repo(db: &Database, config: &IndexConfig) -> Result<()> {
    // Phase 1: Parse dependencies
    let cargo_toml = fs::read_to_string(config.repo_path.join("Cargo.toml"))?;
    let direct = dependencies::parse_cargo_toml(&cargo_toml)?;
    let locked = if let Ok(lock) = fs::read_to_string(config.repo_path.join("Cargo.lock")) {
        dependencies::parse_cargo_lock(&lock)?
    } else {
        vec![]
    };
    let resolved = dependencies::resolve_dependencies(&direct, &locked);
    store::store_dependencies(db, &resolved)?;

    // Phase 2: Parse source files
    for entry in walkdir (src/) finding .rs files {
        let source = fs::read_to_string(&path)?;
        let symbols = parser::parse_rust_source(&source, &relative_path)?;
        let file_id = db.insert_file(&relative_path, &content_hash)?;
        store::store_symbols(db, file_id, &symbols)?;
    }

    // Phase 3: Fetch docs (async, only direct deps)
    if !config.skip_doc_fetch {
        // run async doc fetching for direct deps
    }

    // Phase 4: Update metadata
    let commit_hash = get_current_commit_hash(&config.repo_path)?;
    db.set_metadata(&config.repo_path.display().to_string(), &commit_hash)?;

    Ok(())
}
```

Use `walkdir` crate (add as dependency) or `std::fs::read_dir` recursively.

**Step 4: Run tests**

Run: `cargo test -- indexer::tests`
Expected: PASS

**Step 5: Commit**

```bash
git add src/indexer/mod.rs Cargo.toml
git commit -m "Add indexer pipeline orchestrator"
```

---

### Task 10: MCP Server — Query Tool

**Files:**
- Create: `src/server/mod.rs`
- Create: `src/server/tools/mod.rs`
- Create: `src/server/tools/query.rs`
- Modify: `src/main.rs` (add `mod server;`)

**Step 1: Write tests for query tool logic**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[test]
    fn test_query_symbols() {
        let db = Database::open_in_memory().unwrap();
        // Insert test data
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(&db, file_id, &[Symbol {
            name: "parse_config".into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            file_path: "src/lib.rs".into(),
            line_start: 1, line_end: 10,
            signature: "pub fn parse_config() -> Config".into(),
        }]).unwrap();

        let result = handle_query(&db, "parse", Some("symbols")).unwrap();
        assert!(!result.is_empty());
        assert!(result.contains("parse_config"));
    }

    #[test]
    fn test_query_docs() {
        let db = Database::open_in_memory().unwrap();
        let dep_id = db.insert_dependency("serde", "1.0", true, None).unwrap();
        store_doc(&db, dep_id, DocSource::DocsRs, "Serde serialization framework").unwrap();

        let result = handle_query(&db, "serialization", Some("docs")).unwrap();
        assert!(result.contains("serialization"));
    }
}
```

**Step 2: Run test, verify fail**

**Step 3: Implement query tool**

- `handle_query(db, query, scope) -> Result<String>` — dispatch to FTS5 search based on scope
- Scope: `"symbols"`, `"docs"`, `"files"`, `"all"` (default)
- Format results as structured text with file paths, line numbers, snippets

**Step 4: Run tests, verify pass**

**Step 5: Commit**

```bash
git add src/server/
git commit -m "Add MCP query tool"
```

---

### Task 11: MCP Server — Context Tool

**Files:**
- Create: `src/server/tools/context.rs`

**Step 1: Write tests**

Test that given a symbol name, context returns: definition, signature, file location, what it imports, what references it, and relevant dependency doc snippets.

**Step 2: Implement**

- `handle_context(db, symbol_name) -> Result<String>`
- Look up symbol in DB, find all `symbol_refs` where it's the source or target
- If it uses types from a dependency, include relevant doc snippet
- Format as structured text

**Step 3: Run tests, commit**

```bash
git add src/server/tools/context.rs
git commit -m "Add MCP context tool"
```

---

### Task 12: MCP Server — Impact Tool

**Files:**
- Create: `src/server/tools/impact.rs`

**Step 1: Write tests**

Test that changing a symbol returns all transitive dependents grouped by depth.

**Step 2: Implement**

- `handle_impact(db, symbol_name) -> Result<String>`
- Recursive CTE query on `symbol_refs` to find all transitive dependents
- Group by depth level
- Format as structured text

**Step 3: Run tests, commit**

```bash
git add src/server/tools/impact.rs
git commit -m "Add MCP impact tool"
```

---

### Task 13: MCP Server — Docs Tool

**Files:**
- Create: `src/server/tools/docs.rs`

**Step 1: Write tests**

Test that querying docs for a dependency returns its stored documentation, filtered by optional topic.

**Step 2: Implement**

- `handle_docs(db, dep_name, topic) -> Result<String>`
- Look up dependency, return stored docs
- If topic provided, filter with FTS5 snippet matching
- If dependency not found in DB and it's a transitive dep, trigger lazy fetch

**Step 3: Run tests, commit**

```bash
git add src/server/tools/docs.rs
git commit -m "Add MCP docs tool"
```

---

### Task 14: MCP Server Wiring with rmcp

**Files:**
- Modify: `src/server/mod.rs` (wire up rmcp server)
- Modify: `src/server/tools/mod.rs` (register all tools)
- Modify: `src/main.rs` (full startup flow)

**Step 1: Implement the MCP server handler**

Wire up `rmcp` with stdio transport. Register 4 tools: `query`, `context`, `impact`, `docs`. Each tool's handler dispatches to the corresponding `handle_*` function with a shared `Database` reference.

Reference rmcp pattern:
```rust
use rmcp::ServerHandler;
// Implement ServerHandler trait or use #[tool] macros
// Serve over rmcp::transport::stdio()
```

**Step 2: Wire up main.rs**

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Init tracing to stderr
    // 2. Parse --repo flag or use cwd
    // 3. Open/create .illu/index.db
    // 4. Check freshness, run indexer if needed
    // 5. Start MCP server over stdio
}
```

**Step 3: Manual test — run the binary and send a JSON-RPC initialize request on stdin**

**Step 4: Commit**

```bash
git add src/server/ src/main.rs
git commit -m "Wire up MCP server with rmcp over stdio"
```

---

### Task 15: Claude Skill Generation

**Files:**
- Modify: `src/indexer/mod.rs` (add skill generation after indexing)

**Step 1: Write test**

```rust
#[test]
fn test_generate_skill_content() {
    let deps = vec!["serde".to_string(), "tokio".to_string()];
    let skill = generate_claude_skill(&deps);
    assert!(skill.contains("serde"));
    assert!(skill.contains("tokio"));
    assert!(skill.contains("docs"));
    assert!(skill.contains("context"));
}
```

**Step 2: Implement**

- `generate_claude_skill(direct_dep_names: &[String]) -> String`
- Generates a `.md` skill file listing the 4 tools and when to use them
- Lists direct dependencies by name
- Write to `.claude/skills/illu-rs.md` in the repo

**Step 3: Run tests, commit**

```bash
git add src/indexer/mod.rs
git commit -m "Add Claude skill auto-generation"
```

---

### Task 16: End-to-End Integration Test

**Files:**
- Create: `tests/integration.rs`

**Step 1: Write integration test**

Create a temp directory with a minimal Rust project, run the full indexer, verify all 4 tool handlers return sensible results. No network (skip doc fetch).

**Step 2: Run and verify**

Run: `cargo test --test integration`
Expected: PASS

**Step 3: Final clippy + fmt**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings`
Expected: clean

**Step 4: Commit**

```bash
git add tests/ src/
git commit -m "Add end-to-end integration test"
```
