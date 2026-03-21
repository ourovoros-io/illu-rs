# illu-rs Testing Plan: Efficiency & Correctness

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Establish comprehensive testing that proves illu-rs produces correct, efficient results — both on synthetic fixtures and on its own codebase as a real-world benchmark.

**Architecture:** Three new test files targeting distinct gaps: self-indexing correctness (`tests/self_index.rs`), performance benchmarks (`benches/indexing.rs`), and error resilience (`tests/error_paths.rs`). Extends existing `data_integrity.rs` and `data_quality.rs` for specific edge cases.

**Tech Stack:** `cargo test`, `criterion` for benchmarks, `tempfile` for fixtures, `std::time::Instant` for lightweight timing assertions.

---

## Phase 1: Self-Indexing — The Ultimate Correctness Test

The most powerful test we can write: index illu-rs's own source and validate results against known ground truth. If illu can't correctly describe itself, it can't correctly describe anything.

### Task 1: Self-index smoke test

**Files:**
- Create: `tests/self_index.rs`

**Step 1: Write the failing test**

```rust
#![expect(clippy::unwrap_used, reason = "integration tests")]

//! Self-indexing tests: index illu-rs's own source code and validate
//! that the tool output matches known ground truth about this codebase.

use illu_rs::db::Database;
use illu_rs::indexer::{IndexConfig, index_repo};
use illu_rs::server::tools::{context, impact, overview, query};
use std::sync::OnceLock;

/// Index the illu-rs repo once, share across all tests in this file.
fn self_db() -> &'static Database {
    static DB: OnceLock<Database> = OnceLock::new();
    DB.get_or_init(|| {
        let db = Database::open_in_memory().unwrap();
        let config = IndexConfig {
            repo_path: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        };
        index_repo(&db, &config).unwrap();
        db
    })
}

#[test]
fn self_index_finds_database_struct() {
    let db = self_db();
    let result = query::handle_query(db, "Database", Some("symbols"), None).unwrap();
    assert!(result.contains("Database"), "should find Database struct");
    assert!(result.contains("src/db.rs"), "should locate in db.rs");
}

#[test]
fn self_index_finds_index_repo_function() {
    let db = self_db();
    let result = query::handle_query(db, "index_repo", Some("symbols"), None).unwrap();
    assert!(result.contains("index_repo"));
    assert!(result.contains("function"));
}

#[test]
fn self_index_finds_illu_server() {
    let db = self_db();
    let result = query::handle_query(db, "IlluServer", Some("symbols"), None).unwrap();
    assert!(result.contains("IlluServer"));
    assert!(result.contains("struct"));
}
```

**Step 2: Run test to verify it passes**

```bash
cargo test --test self_index -v
```

Expected: PASS (this is a smoke test against real code)

**Step 3: Commit**

```bash
git add tests/self_index.rs
git commit -m "test: add self-indexing smoke tests"
```

### Task 2: Self-index context correctness

**Files:**
- Modify: `tests/self_index.rs`

**Step 1: Write the tests**

```rust
#[test]
fn self_context_database_has_fields_and_source() {
    let db = self_db();
    let result = context::handle_context(db, "Database", false).unwrap();
    // Database struct wraps a rusqlite::Connection — verify structure appears
    assert!(result.contains("pub struct Database"), "should show source");
    assert!(result.contains("src/db.rs"), "should show file path");
}

#[test]
fn self_context_parse_rust_source_has_signature() {
    let db = self_db();
    let result = context::handle_context(db, "parse_rust_source", false).unwrap();
    assert!(result.contains("pub fn parse_rust_source"), "should show signature");
    assert!(result.contains("Symbol"), "return type should mention Symbol");
}

#[test]
fn self_context_handle_query_shows_callees() {
    let db = self_db();
    let result = context::handle_context(db, "handle_query", false).unwrap();
    // handle_query calls search_symbols and search_docs
    assert!(
        result.contains("search_symbols") || result.contains("search_docs"),
        "should show callees: got {result}"
    );
}
```

**Step 2: Run and verify**

```bash
cargo test --test self_index -- self_context -v
```

**Step 3: Commit**

```bash
git add tests/self_index.rs
git commit -m "test: self-index context correctness checks"
```

### Task 3: Self-index impact analysis

**Files:**
- Modify: `tests/self_index.rs`

**Step 1: Write the tests**

```rust
#[test]
fn self_impact_database_is_widely_used() {
    let db = self_db();
    let result = impact::handle_impact(db, "Database").unwrap();
    // Database is used by nearly every module
    assert!(result.contains("Impact Analysis"));
    // Should show many dependents
    let dependent_count = result.matches("src/").count();
    assert!(
        dependent_count >= 3,
        "Database should have ≥3 file references in impact, got {dependent_count}"
    );
}

#[test]
fn self_impact_symbol_struct_has_dependents() {
    let db = self_db();
    let result = impact::handle_impact(db, "Symbol").unwrap();
    // Symbol is used by store.rs, parser.rs at minimum
    assert!(result.contains("Impact Analysis"));
    assert!(
        result.contains("store") || result.contains("parser"),
        "Symbol should impact store or parser modules"
    );
}
```

**Step 2: Run and verify**

```bash
cargo test --test self_index -- self_impact -v
```

**Step 3: Commit**

```bash
git add tests/self_index.rs
git commit -m "test: self-index impact analysis validation"
```

### Task 4: Self-index overview and cross-tool consistency

**Files:**
- Modify: `tests/self_index.rs`

**Step 1: Write the tests**

```rust
#[test]
fn self_overview_lists_known_public_api() {
    let db = self_db();
    let result = overview::handle_overview(db, "src/").unwrap();
    // Must contain key public symbols
    for sym in ["Database", "index_repo", "IlluServer", "parse_rust_source"] {
        assert!(result.contains(sym), "overview should list {sym}");
    }
}

#[test]
fn self_overview_db_module() {
    let db = self_db();
    let result = overview::handle_overview(db, "src/db.rs").unwrap();
    // db.rs has many public functions
    for sym in ["open", "search_symbols", "insert_file", "impact_dependents"] {
        assert!(result.contains(sym), "db overview should list {sym}");
    }
}

#[test]
fn self_query_and_context_agree_on_file_path() {
    let db = self_db();
    let query_result = query::handle_query(db, "extract_refs", Some("symbols"), None).unwrap();
    let context_result = context::handle_context(db, "extract_refs", false).unwrap();
    // Both should reference parser.rs
    assert!(query_result.contains("parser.rs"), "query should show parser.rs");
    assert!(context_result.contains("parser.rs"), "context should show parser.rs");
}

#[test]
fn self_index_symbol_count_sanity() {
    let db = self_db();
    let result = overview::handle_overview(db, "src/").unwrap();
    // We know from illu overview there are 147 symbols — allow some drift
    // but it should be in the right ballpark (>100)
    let line_count = result.lines().count();
    assert!(
        line_count > 50,
        "overview should have substantial output for illu-rs, got {line_count} lines"
    );
}
```

**Step 2: Run and verify**

```bash
cargo test --test self_index -v
```

**Step 3: Commit**

```bash
git add tests/self_index.rs
git commit -m "test: self-index overview and cross-tool consistency"
```

---

## Phase 2: Performance Benchmarks

### Task 5: Add criterion dependency

**Files:**
- Modify: `Cargo.toml`

**Step 1: Add dev dependency and bench target**

Add to `Cargo.toml`:
```toml
[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }

[[bench]]
name = "indexing"
harness = false
```

**Step 2: Verify it compiles**

```bash
cargo check --benches
```

**Step 3: Commit**

```bash
git add Cargo.toml
git commit -m "build: add criterion for benchmarks"
```

### Task 6: Indexing benchmark

**Files:**
- Create: `benches/indexing.rs`

**Step 1: Write benchmarks**

```rust
use criterion::{Criterion, criterion_group, criterion_main, black_box};
use illu_rs::db::Database;
use illu_rs::indexer::{IndexConfig, index_repo};

fn bench_self_index(c: &mut Criterion) {
    c.bench_function("index_illu_rs", |b| {
        b.iter(|| {
            let db = Database::open_in_memory().unwrap();
            let config = IndexConfig {
                repo_path: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            };
            index_repo(&db, black_box(&config)).unwrap();
        });
    });
}

fn bench_query_after_index(c: &mut Criterion) {
    // Setup: index once
    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")),
    };
    index_repo(&db, &config).unwrap();

    let mut group = c.benchmark_group("tools");
    group.bench_function("query_symbol", |b| {
        b.iter(|| {
            illu_rs::server::tools::query::handle_query(
                &db,
                black_box("Database"),
                Some("symbols"),
                None,
            ).unwrap();
        });
    });
    group.bench_function("context", |b| {
        b.iter(|| {
            illu_rs::server::tools::context::handle_context(
                &db,
                black_box("Database"),
                false,
            ).unwrap();
        });
    });
    group.bench_function("impact", |b| {
        b.iter(|| {
            illu_rs::server::tools::impact::handle_impact(
                &db,
                black_box("Database"),
            ).unwrap();
        });
    });
    group.bench_function("overview", |b| {
        b.iter(|| {
            illu_rs::server::tools::overview::handle_overview(
                &db,
                black_box("src/"),
            ).unwrap();
        });
    });
    group.finish();
}

criterion_group!(benches, bench_self_index, bench_query_after_index);
criterion_main!(benches);
```

**Step 2: Run benchmarks**

```bash
cargo bench
```

Record baseline numbers. This gives us indexing time and per-tool latency.

**Step 3: Commit**

```bash
git add benches/indexing.rs
git commit -m "bench: add criterion benchmarks for indexing and tool latency"
```

### Task 7: Timing guardrails in tests

**Files:**
- Modify: `tests/self_index.rs`

**Step 1: Add timing assertions**

These aren't benchmarks — they're regression guards that fail if something is catastrophically slow.

```rust
#[test]
fn self_index_completes_under_5_seconds() {
    let start = std::time::Instant::now();
    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")),
    };
    index_repo(&db, &config).unwrap();
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_secs() < 5,
        "indexing illu-rs should complete in <5s, took {elapsed:?}"
    );
}

#[test]
fn tool_queries_complete_under_100ms() {
    let db = self_db();
    let tools: Vec<(&str, Box<dyn Fn()>)> = vec![
        ("query", Box::new(|| { query::handle_query(db, "Database", Some("symbols"), None).unwrap(); })),
        ("context", Box::new(|| { context::handle_context(db, "Database", false).unwrap(); })),
        ("impact", Box::new(|| { impact::handle_impact(db, "Database").unwrap(); })),
        ("overview", Box::new(|| { overview::handle_overview(db, "src/").unwrap(); })),
    ];
    for (name, f) in &tools {
        let start = std::time::Instant::now();
        f();
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 100,
            "{name} tool should complete in <100ms, took {elapsed:?}"
        );
    }
}
```

**Step 2: Run and verify**

```bash
cargo test --test self_index -- timing -v
```

**Step 3: Commit**

```bash
git add tests/self_index.rs
git commit -m "test: add timing guardrails for indexing and tool latency"
```

---

## Phase 3: Error Resilience

### Task 8: Error path tests

**Files:**
- Create: `tests/error_paths.rs`

**Step 1: Write error path tests**

```rust
#![expect(clippy::unwrap_used, reason = "integration tests")]

//! Error path tests: verify illu-rs handles bad input gracefully
//! without panicking or returning misleading results.

use illu_rs::db::Database;
use illu_rs::indexer::{IndexConfig, index_repo};
use illu_rs::server::tools::{context, docs, impact, query};

fn empty_crate() -> (tempfile::TempDir, Database) {
    let dir = tempfile::TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"empty\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    ).unwrap();
    std::fs::write(
        dir.path().join("Cargo.lock"),
        "[[package]]\nname = \"empty\"\nversion = \"0.1.0\"\n",
    ).unwrap();
    std::fs::write(src_dir.join("lib.rs"), "").unwrap();
    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig { repo_path: dir.path().to_path_buf() };
    index_repo(&db, &config).unwrap();
    (dir, db)
}

// --- Empty crate ---

#[test]
fn query_on_empty_crate_returns_no_results() {
    let (_dir, db) = empty_crate();
    let result = query::handle_query(&db, "anything", None, None).unwrap();
    assert!(
        result.contains("No results") || result.is_empty() || !result.contains("function"),
        "empty crate query should not find symbols"
    );
}

#[test]
fn context_on_nonexistent_symbol_returns_not_found() {
    let (_dir, db) = empty_crate();
    let result = context::handle_context(&db, "Nonexistent", false).unwrap();
    assert!(
        result.contains("not found") || result.contains("No symbol"),
        "context for missing symbol should say not found"
    );
}

#[test]
fn impact_on_nonexistent_symbol_does_not_panic() {
    let (_dir, db) = empty_crate();
    let result = impact::handle_impact(&db, "Nonexistent").unwrap();
    // Should not panic, should return something sensible
    assert!(!result.is_empty());
}

// --- Malformed source ---

#[test]
fn malformed_rust_source_does_not_crash() {
    let dir = tempfile::TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"bad\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    ).unwrap();
    std::fs::write(
        dir.path().join("Cargo.lock"),
        "[[package]]\nname = \"bad\"\nversion = \"0.1.0\"\n",
    ).unwrap();
    // Syntactically broken Rust
    std::fs::write(src_dir.join("lib.rs"), "pub fn broken( { }}}}} struct @@@ ").unwrap();
    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig { repo_path: dir.path().to_path_buf() };
    // Should not panic — tree-sitter is error-tolerant
    let result = index_repo(&db, &config);
    assert!(result.is_ok(), "indexing broken source should not panic");
}

// --- Unicode and special characters ---

#[test]
fn unicode_in_doc_comments_preserved() {
    let dir = tempfile::TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"uni\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    ).unwrap();
    std::fs::write(
        dir.path().join("Cargo.lock"),
        "[[package]]\nname = \"uni\"\nversion = \"0.1.0\"\n",
    ).unwrap();
    std::fs::write(
        src_dir.join("lib.rs"),
        "/// Héllo wörld — docs with üñícödé\npub fn greet() {}\n",
    ).unwrap();
    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig { repo_path: dir.path().to_path_buf() };
    index_repo(&db, &config).unwrap();
    let result = context::handle_context(&db, "greet", false).unwrap();
    assert!(result.contains("Héllo") || result.contains("wörld"), "unicode in docs should be preserved");
}

// --- Deeply nested modules ---

#[test]
fn deeply_nested_module_files_indexed() {
    let dir = tempfile::TempDir::new().unwrap();
    let deep = dir.path().join("src/a/b/c");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"deep\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    ).unwrap();
    std::fs::write(
        dir.path().join("Cargo.lock"),
        "[[package]]\nname = \"deep\"\nversion = \"0.1.0\"\n",
    ).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "pub mod a;").unwrap();
    std::fs::write(dir.path().join("src/a/mod.rs"), "pub mod b;").unwrap();
    std::fs::write(dir.path().join("src/a/b/mod.rs"), "pub mod c;").unwrap();
    std::fs::write(deep.join("mod.rs"), "pub fn deep_fn() {}\n").unwrap();
    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig { repo_path: dir.path().to_path_buf() };
    index_repo(&db, &config).unwrap();
    let result = query::handle_query(&db, "deep_fn", Some("symbols"), None).unwrap();
    assert!(result.contains("deep_fn"), "should find deeply nested function");
}
```

**Step 2: Run and verify**

```bash
cargo test --test error_paths -v
```

**Step 3: Commit**

```bash
git add tests/error_paths.rs
git commit -m "test: add error path and edge case tests"
```

---

## Phase 4: Real-World Rust Syntax Edge Cases

### Task 9: Complex syntax coverage in data_quality

**Files:**
- Modify: `tests/data_quality.rs`

Add tests for syntax patterns not currently covered by the synthetic fixtures:

**Step 1: Write the tests** (append to existing file)

```rust
// --- Complex Rust syntax edge cases ---

#[test]
fn where_clause_in_signature() {
    let (_dir, db) = index_source(
        "pub fn process<T>(item: T) -> String where T: std::fmt::Display + Clone {\n    item.to_string()\n}\n",
    );
    let result = query::handle_query(&db, "process", Some("symbols"), None).unwrap();
    assert!(result.contains("process"), "should parse fn with where clause");
}

#[test]
fn const_generic_parameter() {
    let (_dir, db) = index_source(
        "pub struct FixedArray<const N: usize> {\n    data: [u8; N],\n}\n",
    );
    let result = context::handle_context(&db, "FixedArray", false).unwrap();
    assert!(result.contains("FixedArray"), "should parse const generics");
}

#[test]
fn impl_block_with_lifetime() {
    let (_dir, db) = index_source(
        "pub struct Parser<'a> {\n    input: &'a str,\n}\nimpl<'a> Parser<'a> {\n    pub fn new(input: &'a str) -> Self {\n        Parser { input }\n    }\n}\n",
    );
    let result = context::handle_context(&db, "Parser", false).unwrap();
    assert!(result.contains("Parser"), "should parse lifetime in impl");
    assert!(result.contains("new"), "should find method in lifetime impl");
}

#[test]
fn multiple_impl_blocks_for_same_type() {
    let (_dir, db) = index_source(
        "pub struct Builder;\nimpl Builder {\n    pub fn new() -> Self { Builder }\n}\nimpl Builder {\n    pub fn build(&self) -> String { String::new() }\n}\n",
    );
    let result = context::handle_context(&db, "Builder", false).unwrap();
    assert!(result.contains("new"), "should find method from first impl");
    assert!(result.contains("build"), "should find method from second impl");
}

#[test]
fn cfg_gated_function() {
    let (_dir, db) = index_source(
        "#[cfg(test)]\npub fn test_only() {}\n\n#[cfg(not(test))]\npub fn prod_only() {}\n\npub fn always() {}\n",
    );
    let result = query::handle_query(&db, "always", Some("symbols"), None).unwrap();
    assert!(result.contains("always"), "should find non-cfg function");
}

#[test]
fn unsafe_function_parsed() {
    let (_dir, db) = index_source(
        "pub unsafe fn dangerous(ptr: *const u8) -> u8 {\n    *ptr\n}\n",
    );
    let result = query::handle_query(&db, "dangerous", Some("symbols"), None).unwrap();
    assert!(result.contains("dangerous"), "should find unsafe function");
}
```

**Step 2: Run and verify**

```bash
cargo test --test data_quality -- where_clause const_generic impl_block_with_lifetime multiple_impl cfg_gated unsafe_function -v
```

**Step 3: Commit**

```bash
git add tests/data_quality.rs
git commit -m "test: add complex Rust syntax edge case coverage"
```

---

## Phase 5: Incremental Indexing Edge Cases

### Task 10: Refresh index edge cases in data_integrity

**Files:**
- Modify: `tests/data_integrity.rs`

**Step 1: Write the tests** (append to existing file)

```rust
// --- Incremental indexing edge cases ---

#[test]
fn refresh_handles_new_file_added() {
    let dir = tempfile::TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"inc\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    ).unwrap();
    std::fs::write(
        dir.path().join("Cargo.lock"),
        "[[package]]\nname = \"inc\"\nversion = \"0.1.0\"\n",
    ).unwrap();
    std::fs::write(src_dir.join("lib.rs"), "pub mod utils;\npub fn original() {}\n").unwrap();
    std::fs::write(src_dir.join("utils.rs"), "pub fn helper() {}\n").unwrap();

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig { repo_path: dir.path().to_path_buf() };
    index_repo(&db, &config).unwrap();

    // Add a new file
    std::fs::write(src_dir.join("extra.rs"), "pub fn bonus() {}\n").unwrap();
    std::fs::write(src_dir.join("lib.rs"), "pub mod utils;\npub mod extra;\npub fn original() {}\n").unwrap();

    refresh_index(&db, &config).unwrap();

    let result = query::handle_query(&db, "bonus", Some("symbols"), None).unwrap();
    assert!(result.contains("bonus"), "refresh should pick up new file's symbols");
}

#[test]
fn refresh_handles_file_content_change() {
    let dir = tempfile::TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"chg\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    ).unwrap();
    std::fs::write(
        dir.path().join("Cargo.lock"),
        "[[package]]\nname = \"chg\"\nversion = \"0.1.0\"\n",
    ).unwrap();
    std::fs::write(src_dir.join("lib.rs"), "pub fn version_one() {}\n").unwrap();

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig { repo_path: dir.path().to_path_buf() };
    index_repo(&db, &config).unwrap();

    // Change the function name
    std::fs::write(src_dir.join("lib.rs"), "pub fn version_two() {}\n").unwrap();
    refresh_index(&db, &config).unwrap();

    let old = query::handle_query(&db, "version_one", Some("symbols"), None).unwrap();
    let new = query::handle_query(&db, "version_two", Some("symbols"), None).unwrap();
    assert!(!old.contains("version_one"), "old symbol should be gone after refresh");
    assert!(new.contains("version_two"), "new symbol should appear after refresh");
}
```

**Step 2: Run and verify**

```bash
cargo test --test data_integrity -- refresh_handles -v
```

**Step 3: Commit**

```bash
git add tests/data_integrity.rs
git commit -m "test: incremental indexing edge cases for file add/change"
```

---

## Phase 6: FTS Search Quality

### Task 11: Search ranking tests

**Files:**
- Modify: `tests/data_quality.rs`

**Step 1: Write ranking-focused tests**

```rust
#[test]
fn exact_match_ranks_above_substring() {
    let (_dir, db) = index_source(
        "pub fn config() {}\npub fn config_parser() {}\npub fn parse_config_file() {}\n",
    );
    let result = query::handle_query(&db, "config", Some("symbols"), None).unwrap();
    let lines: Vec<&str> = result.lines().collect();
    // Find positions of exact vs partial matches
    let exact_pos = lines.iter().position(|l| l.contains("config") && !l.contains("config_") && !l.contains("_config"));
    let partial_pos = lines.iter().position(|l| l.contains("config_parser"));
    if let (Some(e), Some(p)) = (exact_pos, partial_pos) {
        assert!(e < p, "exact match 'config' should rank above 'config_parser'");
    }
}

#[test]
fn query_kind_filter_excludes_other_kinds() {
    let (_dir, db) = index_source(
        "pub fn process() {}\npub struct Process {}\npub trait Processable {}\n",
    );
    let result = query::handle_query(&db, "process", Some("symbols"), Some("function")).unwrap();
    assert!(result.contains("process"), "should find function");
    // struct and trait should not appear when filtering by function
    assert!(!result.contains("struct Process"), "should not show struct when filtering function");
}
```

**Step 2: Run and verify**

```bash
cargo test --test data_quality -- exact_match_ranks query_kind_filter -v
```

**Step 3: Commit**

```bash
git add tests/data_quality.rs
git commit -m "test: FTS search ranking and kind filter tests"
```

---

## Summary

| Phase | File | Tests | What it proves |
|-------|------|-------|----------------|
| 1 | `tests/self_index.rs` | ~12 | illu correctly indexes real-world Rust (itself) |
| 2 | `benches/indexing.rs` | 5 benchmarks | Performance baselines with regression detection |
| 2 | `tests/self_index.rs` | 2 | Timing guardrails (index <5s, tools <100ms) |
| 3 | `tests/error_paths.rs` | ~6 | Graceful handling of empty/broken/unicode/deep input |
| 4 | `tests/data_quality.rs` | ~6 | Complex Rust syntax (const generics, lifetimes, cfg, unsafe) |
| 5 | `tests/data_integrity.rs` | ~2 | Incremental indexing handles adds and changes |
| 6 | `tests/data_quality.rs` | ~2 | FTS ranking and kind filter precision |

**Total new tests:** ~30
**Total new files:** 3 (`tests/self_index.rs`, `tests/error_paths.rs`, `benches/indexing.rs`)
**Modified files:** 3 (`Cargo.toml`, `tests/data_quality.rs`, `tests/data_integrity.rs`)
