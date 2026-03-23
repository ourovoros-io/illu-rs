# Correctness Test Suite Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fill 11 correctness gaps in the test suite and fix 1 pre-existing test failure, ensuring illu-rs delivers accurate data.

**Architecture:** All tests go in `tests/data_integrity.rs` (integration) or existing unit test modules. One fix to `tests/data_integrity.rs` for the failing `docs_tool_shows_version_and_source` test.

**Tech Stack:** Rust, cargo test

---

### Task 1: Fix failing `docs_tool_shows_version_and_source` test

The test stores two docs (both with `module=""`), but the new two-tier routing (`get_doc_by_module` with LIMIT 1) only returns the first. Update the test to match two-tier behavior.

**Files:**
- Modify: `tests/data_integrity.rs` (~line 1030-1054)

**Fix:** Change the test to store one doc as summary (module="") and one as a module doc. Verify the summary is returned when no topic, and the module doc when topic matches. Or simpler: just verify version is shown and at least one source appears.

```rust
#[test]
fn docs_tool_shows_version_and_source() {
    let (_dir, db) = index_source("pub fn placeholder() {}\n");
    let dep_id = db
        .insert_dependency("serde", "1.0.210", true, None)
        .unwrap();
    db.store_doc(dep_id, "cargo_doc", "# serde 1.0.210\n\nStructured docs")
        .unwrap();

    let result = docs::handle_docs(&db, "serde", None).unwrap();
    assert!(
        result.contains("1.0.210"),
        "docs must include version: {result}"
    );
    assert!(
        result.contains("cargo_doc"),
        "must show source: {result}"
    );
}
```

Run: `cargo test --test data_integrity docs_tool_shows_version_and_source`
Expected: PASS

Commit: `fix: update docs test for two-tier routing behavior`

---

### Task 2: Add correctness tests for reference resolution

11 new tests covering the gaps identified in the audit.

**Files:**
- Modify: `tests/data_integrity.rs` (add tests)

All tests use the existing `index_source`, `index_multi_file`, `index_workspace` helpers already in the test files.

**Tests to add:**

```rust
// === GAP 1: Cross-crate ref resolution end-to-end ===
#[test]
fn workspace_cross_crate_ref_resolves_target_file() {
    // In a workspace, `use shared::SharedType` in app creates a ref
    // with target_file pointing to the shared crate's file.
    let (_dir, db) = index_workspace(
        r#"
[workspace]
members = ["shared", "app"]
"#,
        r#"
[[package]]
name = "shared"
version = "0.1.0"

[[package]]
name = "app"
version = "0.1.0"
"#,
        &[
            (
                "shared",
                "[package]\nname = \"shared\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
                "pub struct SharedType { pub value: i32 }",
            ),
            (
                "app",
                "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nshared = { path = \"../shared\" }\n",
                "use shared::SharedType;\npub fn use_it() -> SharedType { SharedType { value: 1 } }",
            ),
        ],
    );

    // use_it should have a ref to SharedType
    let deps = db.impact_dependents("SharedType").unwrap();
    assert!(
        deps.iter().any(|d| d.name == "use_it"),
        "use_it must depend on SharedType via cross-crate ref: {deps:?}"
    );
}

// === GAP 2: Same-name symbols across crates resolve correctly ===
#[test]
fn workspace_same_name_ref_resolves_to_correct_crate() {
    // Both core and api have `Error`. app uses core::Error.
    // The ref should NOT accidentally point to api::Error.
    let (_dir, db) = index_workspace(
        r#"
[workspace]
members = ["core_lib", "api", "app"]
"#,
        r#"
[[package]]
name = "core_lib"
version = "0.1.0"

[[package]]
name = "api"
version = "0.1.0"

[[package]]
name = "app"
version = "0.1.0"
"#,
        &[
            (
                "core_lib",
                "[package]\nname = \"core_lib\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
                "pub struct Error { pub message: String }",
            ),
            (
                "api",
                "[package]\nname = \"api\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
                "pub struct Error { pub code: i32 }",
            ),
            (
                "app",
                "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\ncore_lib = { path = \"../core_lib\" }\n",
                "use core_lib::Error;\npub fn handle() -> Error { Error { message: String::new() } }",
            ),
        ],
    );

    // handle should depend on Error
    let deps = db.impact_dependents("Error").unwrap();
    let handle_dep = deps.iter().find(|d| d.name == "handle");
    assert!(
        handle_dep.is_some(),
        "handle must depend on Error: {deps:?}"
    );
    // The ref should be to core_lib's Error (file path check)
    assert!(
        handle_dep.unwrap().file_path.contains("app/"),
        "handle is in app crate"
    );
}

// === GAP 3: self.method() resolves to correct impl end-to-end ===
#[test]
fn self_method_resolves_correct_impl_with_collision() {
    let (_dir, db) = index_source(
        r#"
pub struct TypeA;
pub struct TypeB;

impl TypeA {
    pub fn helper(&self) -> i32 { 1 }
    pub fn do_work(&self) -> i32 { self.helper() }
}

impl TypeB {
    pub fn helper(&self) -> i32 { 2 }
}
"#,
    );

    // do_work calls self.helper() — should resolve to TypeA::helper
    let callees = db.get_callees("do_work").unwrap();
    let helper_callee = callees.iter().find(|c| c.name == "helper");
    assert!(
        helper_callee.is_some(),
        "do_work must call helper: {callees:?}"
    );
}

// === GAP 4: Impact depth and via chain accuracy ===
#[test]
fn impact_depth_and_via_chain_accurate() {
    let (_dir, db) = index_source(
        r#"
pub fn root() {}
pub fn depth1() { root(); }
pub fn depth2() { depth1(); }
pub fn depth3() { depth2(); }
"#,
    );

    let result = impact::handle_impact(&db, "root").unwrap();
    // Depth 1: depth1 directly calls root
    assert!(result.contains("depth1"), "depth1 at depth 1: {result}");
    // Depth 2: depth2 calls depth1 which calls root
    assert!(result.contains("depth2"), "depth2 at depth 2: {result}");
    // Depth 3: depth3 calls depth2
    assert!(result.contains("depth3"), "depth3 at depth 3: {result}");
    // Via chain should show the path
    assert!(
        result.contains("via"),
        "must show via chain: {result}"
    );
}

// === GAP 5: Stale refs from unchanged file to deleted symbol ===
#[test]
fn refresh_cleans_refs_from_unchanged_caller_to_deleted_target() {
    let dir = tempfile::TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    // caller.rs calls target()
    std::fs::write(
        src_dir.join("caller.rs"),
        "pub fn caller() { target(); }\n",
    )
    .unwrap();
    // target.rs defines target()
    std::fs::write(
        src_dir.join("target.rs"),
        "pub fn target() {}\n",
    )
    .unwrap();
    std::fs::write(
        src_dir.join("lib.rs"),
        "mod caller;\nmod target;\n",
    )
    .unwrap();

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();

    // Verify ref exists
    let deps = db.impact_dependents("target").unwrap();
    assert!(
        deps.iter().any(|d| d.name == "caller"),
        "caller should depend on target initially"
    );

    // Delete target.rs (caller.rs unchanged)
    std::fs::remove_file(src_dir.join("target.rs")).unwrap();
    std::fs::write(src_dir.join("lib.rs"), "mod caller;\n").unwrap();
    refresh_index(&db, &config).unwrap();

    // target symbol gone, stale ref should be cleaned
    let syms = db.search_symbols("target").unwrap();
    assert!(syms.is_empty(), "target should be gone");
    // No dangling refs
    let deps = db.impact_dependents("target").unwrap();
    assert!(deps.is_empty(), "stale refs should be cleaned: {deps:?}");
}

// === GAP 6: Line numbers update after refresh ===
#[test]
fn refresh_updates_line_numbers() {
    let dir = tempfile::TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(src_dir.join("lib.rs"), "pub fn foo() {}\n").unwrap();

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();

    let syms = db.search_symbols("foo").unwrap();
    assert_eq!(syms[0].line_start, 1, "initially at line 1");

    // Add 5 blank lines before foo
    std::fs::write(
        src_dir.join("lib.rs"),
        "\n\n\n\n\npub fn foo() {}\n",
    )
    .unwrap();
    refresh_index(&db, &config).unwrap();

    let syms = db.search_symbols("foo").unwrap();
    assert_eq!(
        syms[0].line_start, 6,
        "after adding 5 blank lines, foo should be at line 6"
    );
}

// === GAP 7: full_body: true reads actual file ===
#[test]
fn context_full_body_returns_untruncated_source() {
    let dir = tempfile::TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    // Create a function with 120 lines (exceeds 100-line truncation)
    let mut body = String::from("pub fn big_fn() {\n");
    for i in 0..118 {
        body.push_str(&format!("    let _x{i} = {i};\n"));
    }
    body.push_str("}\n");
    std::fs::write(src_dir.join("lib.rs"), &body).unwrap();

    let db_path = dir.path().join(".illu").join("index.db");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    let db = Database::open(&db_path).unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();

    // Without full_body, should be truncated
    let result = context::handle_context(&db, "big_fn", false).unwrap();
    assert!(
        result.contains("truncated"),
        "should be truncated without full_body: {result}"
    );

    // With full_body, should show all lines
    let result = context::handle_context(&db, "big_fn", true).unwrap();
    assert!(
        !result.contains("truncated"),
        "should NOT be truncated with full_body: {result}"
    );
    assert!(
        result.contains("_x117"),
        "should contain last variable: {result}"
    );
}

// === GAP 8: Docs no-topic lists available modules ===
#[test]
fn docs_no_topic_lists_modules() {
    let (_dir, db) = index_source("pub fn placeholder() {}\n");
    let dep_id = db
        .insert_dependency("tokio", "1.35.0", true, None)
        .unwrap();
    db.store_doc_with_module(dep_id, "cargo_doc", "Tokio summary", "")
        .unwrap();
    db.store_doc_with_module(dep_id, "cargo_doc", "Sync primitives", "sync")
        .unwrap();
    db.store_doc_with_module(dep_id, "cargo_doc", "File I/O", "fs")
        .unwrap();

    let result = docs::handle_docs(&db, "tokio", None).unwrap();
    assert!(
        result.contains("sync"),
        "must list sync module: {result}"
    );
    assert!(
        result.contains("fs"),
        "must list fs module: {result}"
    );
}

// === GAP 9: Non-self field method calls still found ===
#[test]
fn non_self_method_call_creates_ref() {
    let (_dir, db) = index_source(
        r#"
pub struct Processor;

impl Processor {
    pub fn process(&self) -> i32 { 42 }
}

pub fn run(p: Processor) -> i32 {
    p.process()
}
"#,
    );

    // run calls p.process() — even though it's not self, the ref should exist
    let callees = db.get_callees("run").unwrap();
    assert!(
        callees.iter().any(|c| c.name == "process"),
        "run must have a ref to process: {callees:?}"
    );
}

// === GAP 10: Empty and comment-only files don't crash ===
#[test]
fn empty_file_indexed_without_error() {
    let (_dir, db) = index_multi_file(&[
        ("lib.rs", "pub mod empty;\npub fn real() {}\n"),
        ("empty.rs", ""),
    ]);
    // Should not crash, real function should be found
    let syms = db.search_symbols("real").unwrap();
    assert_eq!(syms.len(), 1);
}

#[test]
fn comment_only_file_indexed_without_error() {
    let (_dir, db) = index_multi_file(&[
        ("lib.rs", "pub mod comments;\npub fn real() {}\n"),
        ("comments.rs", "// This file has only comments\n// Nothing else\n"),
    ]);
    let syms = db.search_symbols("real").unwrap();
    assert_eq!(syms.len(), 1);
}

// === GAP 11: Same name different kinds in impact ===
#[test]
fn impact_handles_overloaded_names() {
    let (_dir, db) = index_source(
        r#"
pub struct Error {
    pub message: String,
}

pub fn make_error() -> Error {
    Error { message: String::new() }
}
"#,
    );

    // Impact on "Error" should show make_error as a dependent
    let result = impact::handle_impact(&db, "Error").unwrap();
    assert!(
        result.contains("make_error"),
        "make_error uses Error: {result}"
    );
}
```

### Required imports at top of `tests/data_integrity.rs`

Make sure these are imported (most should already be):
```rust
use illu_rs::db::Database;
use illu_rs::indexer::{IndexConfig, index_repo, refresh_index};
use illu_rs::server::tools::{context, docs, impact};
```

Run: `cargo test --test data_integrity`
Expected: All pass (including the fixed `docs_tool_shows_version_and_source`).

Run: `cargo test && cargo clippy --all-targets --all-features -- -D warnings`
Expected: All pass.

Commit: `test: add 12 correctness tests covering reference resolution, impact accuracy, and data integrity gaps`
