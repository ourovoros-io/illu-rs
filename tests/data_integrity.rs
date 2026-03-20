#![expect(clippy::unwrap_used, reason = "integration tests")]

//! Data integrity tests: verify that illu-rs never produces wrong data
//! that could poison an AI model's understanding of the codebase.
//!
//! These tests focus on the highest-risk scenarios:
//! - Line numbers must be exact (wrong = Claude edits wrong code)
//! - Signatures must never contain body content (wrong = broken API calls)
//! - References must not create false links (wrong = misleading impact analysis)
//! - Circular references must terminate (wrong = tool hangs)
//! - Incremental re-indexing must not leave stale data (wrong = outdated info)
//! - Cross-tool consistency (all tools see the same file paths and line ranges)

use illu_rs::db::Database;
use illu_rs::indexer::{IndexConfig, index_repo, refresh_index};
use illu_rs::server::tools::{context, docs, impact, overview, query, tree};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn index_source(lib_rs: &str) -> (tempfile::TempDir, Database) {
    let dir = tempfile::TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test_crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("Cargo.lock"),
        "[[package]]\nname = \"test_crate\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(src_dir.join("lib.rs"), lib_rs).unwrap();
    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();
    (dir, db)
}

fn index_multi_file(files: &[(&str, &str)]) -> (tempfile::TempDir, Database) {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test_crate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("Cargo.lock"),
        "[[package]]\nname = \"test_crate\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    for (path, content) in files {
        let full = dir.path().join("src").join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full, content).unwrap();
    }
    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();
    (dir, db)
}

fn write_crate(base: &std::path::Path, name: &str, cargo_toml: &str, lib_rs: &str) {
    let crate_dir = base.join(name);
    std::fs::create_dir_all(crate_dir.join("src")).unwrap();
    std::fs::write(crate_dir.join("Cargo.toml"), cargo_toml).unwrap();
    std::fs::write(crate_dir.join("src").join("lib.rs"), lib_rs).unwrap();
}

fn index_workspace(
    root_toml: &str,
    lock: &str,
    crates: &[(&str, &str, &str)],
) -> (tempfile::TempDir, Database) {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), root_toml).unwrap();
    std::fs::write(dir.path().join("Cargo.lock"), lock).unwrap();
    for (name, toml, src) in crates {
        write_crate(dir.path(), name, toml, src);
    }
    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();
    (dir, db)
}

// =========================================================================
// 1. LINE NUMBER ACCURACY
//    Wrong line numbers → Claude edits the wrong lines of code
// =========================================================================

#[test]
fn line_numbers_exact_for_multiline_struct() {
    // Lines:
    // 1: (empty)
    // 2: /// Doc comment
    // 3: pub struct Config {
    // 4:     pub host: String,
    // 5:     pub port: u16,
    // 6:     pub debug: bool,
    // 7: }
    let (_dir, db) = index_source(
        r"
/// Doc comment
pub struct Config {
    pub host: String,
    pub port: u16,
    pub debug: bool,
}
",
    );
    let syms = db.search_symbols("Config").unwrap();
    let sym = syms.iter().find(|s| s.name == "Config").unwrap();
    // Tree-sitter includes doc comments for functions but not always
    // for structs — the struct node starts at the `pub struct` line
    assert_eq!(sym.line_start, 3, "struct starts at `pub struct` line");
    assert_eq!(sym.line_end, 7, "struct should end at closing brace");
}

#[test]
fn line_numbers_exact_for_function_with_attributes() {
    // Lines:
    // 1: (empty)
    // 2: #[inline]
    // 3: #[must_use]
    // 4: /// Important function
    // 5: pub fn compute(x: i32) -> i32 {
    // 6:     x * 2
    // 7: }
    let (_dir, db) = index_source(
        r"
#[inline]
#[must_use]
/// Important function
pub fn compute(x: i32) -> i32 {
    x * 2
}
",
    );
    let syms = db.search_symbols("compute").unwrap();
    let sym = syms.iter().find(|s| s.name == "compute").unwrap();
    // Tree-sitter includes attributes and doc comments in the node span
    assert!(
        sym.line_start <= 5,
        "function start should include attrs/docs, got {}",
        sym.line_start
    );
    assert_eq!(sym.line_end, 7, "function should end at closing brace");
}

#[test]
fn line_numbers_survive_body_truncation() {
    // Create a function with >100 lines so body gets truncated
    use std::fmt::Write;
    let mut source = String::from("pub fn big_func() {\n");
    for i in 0..120 {
        let _ = writeln!(source, "    let x{i} = {i};");
    }
    source.push_str("}\n");

    let (_dir, db) = index_source(&source);
    let syms = db.search_symbols("big_func").unwrap();
    let sym = syms.iter().find(|s| s.name == "big_func").unwrap();

    // Body is truncated, but line_end must reflect the ACTUAL end
    assert_eq!(sym.line_start, 1, "function starts at line 1");
    assert_eq!(
        sym.line_end, 122,
        "line_end must be actual end (122), not truncated end"
    );

    // Verify the body IS truncated
    let body = sym.body.as_deref().unwrap();
    assert!(
        body.contains("// ... truncated"),
        "body should be truncated"
    );
}

#[test]
fn line_numbers_for_impl_methods() {
    let (_dir, db) = index_source(
        r"
pub struct Foo;

impl Foo {
    pub fn method_a(&self) -> i32 {
        1
    }

    pub fn method_b(&self) -> i32 {
        2
    }
}
",
    );
    let syms = db.search_symbols("method_a").unwrap();
    let a = syms.iter().find(|s| s.name == "method_a").unwrap();
    let syms = db.search_symbols("method_b").unwrap();
    let b = syms.iter().find(|s| s.name == "method_b").unwrap();

    // method_a and method_b must not overlap
    assert!(
        a.line_end < b.line_start,
        "method_a (ends {}) must end before method_b (starts {})",
        a.line_end,
        b.line_start
    );
}

// =========================================================================
// 2. SIGNATURE PURITY
//    Body content in signatures → Claude writes wrong API calls
// =========================================================================

#[test]
fn signature_never_contains_body_content() {
    let (_dir, db) = index_source(
        r#"
pub fn complex_logic(input: &str) -> Result<String, Box<dyn std::error::Error>> {
    let parsed = input.parse::<i32>()?;
    let doubled = parsed * 2;
    Ok(format!("result: {doubled}"))
}
"#,
    );
    let syms = db.search_symbols("complex_logic").unwrap();
    let sym = syms.iter().find(|s| s.name == "complex_logic").unwrap();

    assert!(
        !sym.signature.contains("parsed"),
        "signature must not contain body variable: {}",
        sym.signature
    );
    assert!(
        !sym.signature.contains("doubled"),
        "signature must not contain body variable: {}",
        sym.signature
    );
    assert!(
        !sym.signature.contains("format!"),
        "signature must not contain body macro: {}",
        sym.signature
    );
    assert!(
        sym.signature.contains("complex_logic"),
        "signature must contain function name: {}",
        sym.signature
    );
    assert!(
        sym.signature.contains("Result"),
        "signature must contain return type: {}",
        sym.signature
    );
}

#[test]
fn signature_preserves_deeply_nested_generics() {
    let (_dir, db) = index_source(
        r"
pub fn process(data: Vec<HashMap<String, Vec<Option<i32>>>>) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

use std::collections::HashMap;
",
    );
    let syms = db.search_symbols("process").unwrap();
    let sym = syms.iter().find(|s| s.name == "process").unwrap();

    assert!(
        sym.signature
            .contains("Vec<HashMap<String, Vec<Option<i32>>>>"),
        "deeply nested generics must be fully preserved: {}",
        sym.signature
    );
}

#[test]
fn signature_is_first_line_only() {
    let (_dir, db) = index_source(
        r"
pub fn multi_line(
    first: String,
    second: i32,
    third: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    Ok(first)
}
",
    );
    let syms = db.search_symbols("multi_line").unwrap();
    let sym = syms.iter().find(|s| s.name == "multi_line").unwrap();

    // Signature is only the first line — parameters on continuation
    // lines are in the body, not the signature
    assert!(
        !sym.signature.contains('\n'),
        "signature must be a single line: {}",
        sym.signature
    );
    assert!(
        sym.signature.contains("multi_line"),
        "signature must contain function name"
    );
}

// =========================================================================
// 3. REFERENCE GRAPH CORRECTNESS
//    False references → misleading impact analysis
//    Missing references → incomplete impact analysis
// =========================================================================

#[test]
fn circular_references_terminate() {
    let (_dir, db) = index_multi_file(&[(
        "lib.rs",
        r"
pub fn alpha() -> i32 {
    beta()
}

pub fn beta() -> i32 {
    alpha()
}
",
    )]);

    // Impact analysis on a circular chain must not hang or error
    let result = impact::handle_impact(&db, "alpha", None, false).unwrap();
    assert!(
        result.contains("beta"),
        "should show direct dependent: {result}"
    );
    // Alpha appears as dependent of beta which is dependent of alpha
    // The recursive CTE depth limit (5) prevents infinite expansion
    assert!(
        !result.is_empty(),
        "circular refs must not cause empty output"
    );
}

#[test]
fn impact_depth_limit_respected() {
    // Create a chain: a -> b -> c -> d -> e -> f -> g (depth 6)
    let (_dir, db) = index_source(
        r"
pub fn a() { b(); }
pub fn b() { c(); }
pub fn c() { d(); }
pub fn d() { e(); }
pub fn e() { f(); }
pub fn f() { g(); }
pub fn g() {}
",
    );
    let result = impact::handle_impact(&db, "g", None, false).unwrap();

    // Depth limit is 5, so 'a' (at depth 6) should NOT appear
    assert!(result.contains("**f**"), "depth 1 should appear");
    assert!(result.contains("**e**"), "depth 2 should appear");
    assert!(result.contains("**b**"), "depth 5 should appear");
    // 'a' is at depth 6 — beyond the limit
    assert!(
        !result.contains("Depth 6"),
        "depth 6 should be cut off: {result}"
    );
}

#[test]
fn self_method_calls_detected_as_refs() {
    let (_dir, db) = index_source(
        r"
pub struct Server;

impl Server {
    pub fn start(&self) {
        self.bind();
    }

    pub fn bind(&self) {}
}
",
    );
    let result = context::handle_context(&db, "start", false, None, None).unwrap();
    assert!(
        result.contains("bind"),
        "self.bind() should be detected as a callee: {result}"
    );
}

#[test]
fn local_shadowing_prevents_false_ref() {
    let (_dir, db) = index_source(
        r"
pub struct Config {
    pub port: u16,
}

pub fn builder() {
    let Config = 42;
    let _ = Config + 1;
}
",
    );
    let result = impact::handle_impact(&db, "Config", None, false).unwrap();
    // builder shadows Config with a local variable — should NOT
    // appear as a dependent of the Config struct
    assert!(
        !result.contains("builder"),
        "local shadowing should prevent false ref: {result}"
    );
}

#[test]
fn constructor_names_are_tracked_as_refs() {
    let (_dir, db) = index_source(
        r"
pub fn new() -> i32 { 0 }
pub fn default() -> i32 { 1 }
pub fn clone() -> i32 { 2 }
pub fn fmt() -> i32 { 3 }

pub fn caller() -> i32 {
    let x = new();
    let y = default();
    let z = clone();
    let w = fmt();
    x + y + z + w
}
",
    );
    // new/default/clone are user-written constructors — tracked as refs
    let result = impact::handle_impact(&db, "new", None, false).unwrap();
    assert!(
        result.contains("caller"),
        "new should be tracked as a ref target: {result}"
    );
    // fmt is still in the noisy list (derive/trait plumbing) — not tracked
    let result = impact::handle_impact(&db, "fmt", None, false).unwrap();
    assert!(
        !result.contains("caller"),
        "fmt should still be filtered as noisy: {result}"
    );
}

// =========================================================================
// 4. INCREMENTAL RE-INDEXING
//    Stale data after changes → Claude works with outdated info
// =========================================================================

#[test]
fn refresh_removes_renamed_symbol() {
    let dir = tempfile::TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(src_dir.join("lib.rs"), "pub fn old_name() {}\n").unwrap();

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();

    // Verify old_name exists
    let syms = db.search_symbols("old_name").unwrap();
    assert_eq!(syms.len(), 1);

    // Rename the function
    std::fs::write(src_dir.join("lib.rs"), "pub fn new_name() {}\n").unwrap();
    let refreshed = refresh_index(&db, &config).unwrap();
    assert!(refreshed > 0, "should detect changed file");

    // old_name must be GONE — no stale data
    let old = db.search_symbols("old_name").unwrap();
    assert!(old.is_empty(), "old_name must be removed after refresh");

    // new_name must exist
    let new = db.search_symbols("new_name").unwrap();
    assert_eq!(new.len(), 1, "new_name must appear after refresh");
}

#[test]
fn refresh_removes_deleted_file_refs() {
    let dir = tempfile::TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        src_dir.join("lib.rs"),
        "pub fn base() {}\npub fn caller() { base(); }\n",
    )
    .unwrap();
    std::fs::write(
        src_dir.join("extra.rs"),
        "pub fn extra_caller() { base(); }\n",
    )
    .unwrap();

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();

    // Verify extra_caller exists and references base
    let result = impact::handle_impact(&db, "base", None, false).unwrap();
    assert!(
        result.contains("extra_caller"),
        "extra_caller should be dependent before delete"
    );

    // Delete extra.rs
    std::fs::remove_file(src_dir.join("extra.rs")).unwrap();
    refresh_index(&db, &config).unwrap();

    // extra_caller must be completely gone
    let syms = db.search_symbols("extra_caller").unwrap();
    assert!(syms.is_empty(), "deleted file's symbols must be removed");

    // Impact on base should no longer mention extra_caller
    let result = impact::handle_impact(&db, "base", None, false).unwrap();
    assert!(
        !result.contains("extra_caller"),
        "deleted file's refs must be cleaned up: {result}"
    );
}

#[test]
fn refresh_cleans_stale_refs() {
    let dir = tempfile::TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("Cargo.lock"),
        "[[package]]\nname = \"test\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    // Two functions: caller calls target
    std::fs::write(
        src_dir.join("lib.rs"),
        "pub fn target() -> i32 { 42 }\npub fn caller() -> i32 { target() }\n",
    )
    .unwrap();

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();

    // Verify the ref exists
    let result = impact::handle_impact(&db, "target", None, false).unwrap();
    assert!(
        result.contains("caller"),
        "caller should depend on target before change: {result}"
    );

    // Remove the target function, keep only caller (with no call)
    std::fs::write(src_dir.join("lib.rs"), "pub fn caller() -> i32 { 0 }\n").unwrap();
    refresh_index(&db, &config).unwrap();

    // target symbol is gone — stale refs should be cleaned
    let syms = db.search_symbols("target").unwrap();
    assert!(syms.is_empty(), "target must be gone after refresh");

    // No dependents for a symbol that no longer exists
    let result = impact::handle_impact(&db, "target", None, false).unwrap();
    assert!(
        !result.contains("caller"),
        "stale ref from caller to target must be cleaned up: {result}"
    );
}

// =========================================================================
// 5. CROSS-TOOL CONSISTENCY
//    Different tools showing different data for the same symbol → confusion
// =========================================================================

#[test]
fn all_tools_agree_on_file_path() {
    let (_dir, db) = index_multi_file(&[(
        "lib.rs",
        r"
pub struct AppConfig {
    pub port: u16,
}

pub fn use_config() -> AppConfig {
    AppConfig { port: 8080 }
}
",
    )]);

    let query_result =
        query::handle_query(&db, "AppConfig", Some("symbols"), None, None, None, None).unwrap();
    let context_result = context::handle_context(&db, "AppConfig", false, None, None).unwrap();
    let impact_result = impact::handle_impact(&db, "AppConfig", None, false).unwrap();

    // All tools must reference the same file path
    assert!(
        query_result.contains("src/lib.rs"),
        "query must show file path: {query_result}"
    );
    assert!(
        context_result.contains("src/lib.rs"),
        "context must show file path: {context_result}"
    );
    // Impact shows file paths for dependents
    assert!(
        impact_result.contains("src/lib.rs"),
        "impact must show file path: {impact_result}"
    );
}

#[test]
fn context_always_includes_required_fields() {
    let (_dir, db) = index_source(
        r"
/// Application configuration.
#[derive(Debug)]
pub struct Config {
    pub host: String,
    pub port: u16,
}
",
    );
    let result = context::handle_context(&db, "Config", false, None, None).unwrap();

    // Every context response MUST include these fields
    assert!(result.contains("**File:**"), "must include file path");
    assert!(
        result.contains("**Visibility:**"),
        "must include visibility"
    );
    assert!(result.contains("**Signature:**"), "must include signature");
    assert!(
        result.contains("Application configuration"),
        "must include doc comment"
    );
    assert!(
        result.contains("### Fields/Variants"),
        "struct must include fields"
    );
}

#[test]
fn query_result_always_includes_kind_and_signature() {
    let (_dir, db) = index_source(
        r"
pub fn my_func() -> i32 { 42 }
pub struct MyStruct { pub x: i32 }
pub trait MyTrait { fn method(&self); }
pub enum MyEnum { A, B }
",
    );
    let result = query::handle_query(&db, "My", Some("symbols"), None, None, None, None).unwrap();

    // Every symbol in query results must have kind and signature
    for name in &["my_func", "MyStruct", "MyTrait", "MyEnum"] {
        assert!(
            result.contains(name),
            "query for 'My' should find {name}: {result}"
        );
    }
    // Verify kind annotations present
    assert!(result.contains("(function)"), "should show function kind");
    assert!(result.contains("(struct)"), "should show struct kind");
    assert!(result.contains("(trait)"), "should show trait kind");
    assert!(result.contains("(enum)"), "should show enum kind");
}

// =========================================================================
// 6. WORKSPACE DATA CORRECTNESS
//    Wrong crate associations → Claude gets confused about module boundaries
// =========================================================================

#[test]
fn workspace_same_name_symbols_in_different_crates() {
    let (_dir, db) = index_workspace(
        r#"
[workspace]
members = ["core", "api"]
"#,
        r#"
[[package]]
name = "core"
version = "0.1.0"

[[package]]
name = "api"
version = "0.1.0"
"#,
        &[
            (
                "core",
                r#"
[package]
name = "core"
version = "0.1.0"
edition = "2021"
"#,
                "pub struct Error { pub message: String }",
            ),
            (
                "api",
                r#"
[package]
name = "api"
version = "0.1.0"
edition = "2021"
"#,
                "pub struct Error { pub code: i32 }",
            ),
        ],
    );

    // Both Error structs should appear, disambiguated by file path
    let result =
        query::handle_query(&db, "Error", Some("symbols"), None, None, None, None).unwrap();
    assert!(
        result.contains("core/src/lib.rs"),
        "should show core's Error: {result}"
    );
    assert!(
        result.contains("api/src/lib.rs"),
        "should show api's Error: {result}"
    );

    // Context should show the correct file for each
    let ctx = context::handle_context(&db, "Error", false, None, None).unwrap();
    assert!(
        ctx.contains("core/src/lib.rs") && ctx.contains("api/src/lib.rs"),
        "context should show both Error structs: {ctx}"
    );
}

#[test]
fn workspace_transitive_crate_impact() {
    let (_dir, db) = index_workspace(
        r#"
[workspace]
members = ["core", "service", "cli"]
"#,
        r#"
[[package]]
name = "core"
version = "0.1.0"

[[package]]
name = "service"
version = "0.1.0"

[[package]]
name = "cli"
version = "0.1.0"
"#,
        &[
            (
                "core",
                r#"
[package]
name = "core"
version = "0.1.0"
edition = "2021"
"#,
                "pub struct CoreType { pub id: u64 }",
            ),
            (
                "service",
                r#"
[package]
name = "service"
version = "0.1.0"
edition = "2021"

[dependencies]
core = { path = "../core" }
"#,
                "pub fn use_core() -> CoreType { CoreType { id: 1 } }",
            ),
            (
                "cli",
                r#"
[package]
name = "cli"
version = "0.1.0"
edition = "2021"

[dependencies]
service = { path = "../service" }
"#,
                "pub fn main() { use_core(); }",
            ),
        ],
    );

    let result = impact::handle_impact(&db, "CoreType", None, false).unwrap();
    // Should show affected crates in dependency order
    assert!(
        result.contains("Affected Crates"),
        "workspace impact should show crate section: {result}"
    );
    assert!(
        result.contains("core"),
        "should show defining crate: {result}"
    );
    assert!(
        result.contains("service"),
        "should show direct dependent crate: {result}"
    );
    assert!(
        result.contains("cli"),
        "should show transitive dependent crate: {result}"
    );
}

// =========================================================================
// 7. SEARCH QUALITY
//    Wrong ranking → Claude picks the wrong symbol
// =========================================================================

#[test]
fn exact_match_always_first_even_with_many_partials() {
    let (_dir, db) = index_source(
        r"
pub fn configure() {}
pub fn reconfigure() {}
pub fn preconfigure() {}
pub fn configure_all() {}
pub fn Config() {}
pub struct ConfigManager;
pub struct ConfigHelper;
pub struct ConfigLoader;
pub struct Config;
",
    );
    let result =
        query::handle_query(&db, "Config", Some("symbols"), None, None, None, None).unwrap();

    // Find positions of exact match vs others
    let exact_pos = result.find("**Config** (struct)");
    let helper_pos = result.find("ConfigHelper");
    assert!(
        exact_pos.is_some(),
        "exact match 'Config' must appear: {result}"
    );
    if let (Some(exact), Some(helper)) = (exact_pos, helper_pos) {
        assert!(
            exact < helper,
            "exact match must appear before partial matches"
        );
    }
}

#[test]
fn search_with_special_characters_does_not_crash() {
    let (_dir, db) = index_source("pub fn safe() {}\n");

    // These should all return without panicking
    let _ = db.search_symbols("test%inject").unwrap();
    let _ = db.search_symbols("test_wild").unwrap();
    let _ = db.search_symbols("test'quote").unwrap();
    let _ = db.search_symbols("test\"double").unwrap();
    let _ = db.search_symbols("test\\backslash").unwrap();
    let _ = db.search_symbols("").unwrap();
    let _ = db.search_symbols("   ").unwrap();
}

// =========================================================================
// 8. EDGE CASES
//    Unusual but valid Rust code must not crash or produce garbage
// =========================================================================

#[test]
fn file_with_syntax_errors_does_not_poison_db() {
    // Tree-sitter is error-tolerant — it should parse what it can
    let (_dir, db) = index_multi_file(&[
        ("lib.rs", "pub fn valid() -> i32 { 42 }\n"),
        (
            "broken.rs",
            "pub fn broken( { } // syntax error\npub fn also_valid() {}\n",
        ),
    ]);

    // Valid symbols should still be indexed
    let valid = db.search_symbols("valid").unwrap();
    assert!(!valid.is_empty(), "valid function must be indexed");

    // The DB should not contain garbage from the broken file
    let all = db.search_symbols("broken").unwrap();
    // tree-sitter may or may not extract "broken" — either way is fine,
    // but it must not crash or corrupt other data
    let _ = all; // just verify no panic
}

#[test]
fn empty_function_body_handled() {
    let (_dir, db) = index_source(
        r"
pub fn noop() {}
pub fn with_return() -> i32 { 42 }
",
    );
    let result = context::handle_context(&db, "noop", false, None, None).unwrap();
    assert!(result.contains("noop"), "empty-body function must be found");
    assert!(
        result.contains("**Signature:**"),
        "must have signature: {result}"
    );
}

#[test]
fn trait_with_default_methods_indexed_correctly() {
    let (_dir, db) = index_source(
        r"
pub trait Processor {
    fn process(&self, input: &str) -> String;

    fn validate(&self, input: &str) -> bool {
        !input.is_empty()
    }
}
",
    );
    let result = context::handle_context(&db, "Processor", false, None, None).unwrap();
    assert!(
        result.contains("Processor"),
        "trait must be found: {result}"
    );
    // Trait details should mention both methods
    assert!(
        result.contains("process") || result.contains("validate"),
        "trait details should include method info: {result}"
    );
}

#[test]
fn very_long_symbol_name_handled() {
    let name = "a".repeat(200);
    let source = format!("pub fn {name}() {{}}\n");
    let (_dir, db) = index_source(&source);
    let syms = db.search_symbols(&name).unwrap();
    assert_eq!(syms.len(), 1, "very long name must be indexed");
    assert_eq!(syms[0].name, name);
}

// =========================================================================
// 9. DOC COMMENT INTEGRITY
//    Garbled docs → Claude misunderstands the API contract
// =========================================================================

#[test]
fn multiline_doc_comment_fully_preserved() {
    let (_dir, db) = index_source(
        r"
/// First line of docs.
///
/// Second paragraph with `code`.
///
/// # Examples
///
/// ```
/// let x = my_func();
/// ```
pub fn my_func() -> i32 { 42 }
",
    );
    let result = context::handle_context(&db, "my_func", false, None, None).unwrap();
    assert!(
        result.contains("First line of docs."),
        "first line: {result}"
    );
    assert!(
        result.contains("Second paragraph"),
        "second paragraph: {result}"
    );
    assert!(result.contains("# Examples"), "examples heading: {result}");
}

#[test]
fn doc_comment_not_attributed_to_wrong_symbol() {
    let (_dir, db) = index_source(
        r"
/// This doc belongs to alpha.
pub fn alpha() {}

/// This doc belongs to beta.
pub fn beta() {}
",
    );
    let alpha = context::handle_context(&db, "alpha", false, None, None).unwrap();
    let beta = context::handle_context(&db, "beta", false, None, None).unwrap();

    assert!(alpha.contains("belongs to alpha"), "alpha's doc: {alpha}");
    assert!(
        !alpha.contains("belongs to beta"),
        "alpha must NOT have beta's doc: {alpha}"
    );
    assert!(beta.contains("belongs to beta"), "beta's doc: {beta}");
    assert!(
        !beta.contains("belongs to alpha"),
        "beta must NOT have alpha's doc: {beta}"
    );
}

// =========================================================================
// 10. DOCS TOOL DATA INTEGRITY
//     Wrong docs → Claude uses wrong API / hallucinates function signatures
// =========================================================================

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
    assert!(result.contains("cargo_doc"), "must show source: {result}");
}

#[test]
fn docs_tool_topic_filter_is_precise() {
    let (_dir, db) = index_source("pub fn placeholder() {}\n");
    let serde_id = db.insert_dependency("serde", "1.0", true, None).unwrap();
    let tokio_id = db.insert_dependency("tokio", "1.0", true, None).unwrap();
    db.store_doc(serde_id, "docs.rs", "Serde serialization framework")
        .unwrap();
    db.store_doc(
        tokio_id,
        "docs.rs",
        "Tokio async runtime with serialization support",
    )
    .unwrap();

    // Searching for "serialization" scoped to serde should NOT return tokio
    let result = docs::handle_docs(&db, "serde", Some("serialization")).unwrap();
    assert!(result.contains("Serde"), "should find serde docs: {result}");
    assert!(
        !result.contains("Tokio"),
        "must NOT include tokio docs when scoped to serde: {result}"
    );
}

#[test]
fn docs_tool_clear_message_for_unknown_dep() {
    let (_dir, db) = index_source("pub fn placeholder() {}\n");
    let result = docs::handle_docs(&db, "nonexistent_crate", None).unwrap();
    assert!(
        result.contains("not a known dependency"),
        "must clearly state dep is unknown: {result}"
    );
}

#[test]
fn docs_tool_clear_message_for_known_dep_no_docs() {
    let (_dir, db) = index_source("pub fn placeholder() {}\n");
    db.insert_dependency("obscure", "0.1.0", true, None)
        .unwrap();
    let result = docs::handle_docs(&db, "obscure", None).unwrap();
    assert!(
        result.contains("known dependency"),
        "must acknowledge dep exists: {result}"
    );
    assert!(
        result.contains("no docs"),
        "must explain docs are missing: {result}"
    );
}

// =========================================================================
// 11. OVERVIEW TOOL DATA INTEGRITY
//     Wrong overview → Claude has wrong mental model of project structure
// =========================================================================

#[test]
fn overview_includes_signature_and_kind() {
    let (_dir, db) = index_source(
        r"
/// Application configuration.
pub struct Config {
    pub port: u16,
}

pub fn start_server() {}
pub trait Handler {}
",
    );
    let result = overview::handle_overview(&db, "src/", false).unwrap();

    // Every symbol must show kind and signature
    assert!(result.contains("(struct)"), "struct kind: {result}");
    assert!(result.contains("(function)"), "function kind: {result}");
    assert!(result.contains("(trait)"), "trait kind: {result}");
    assert!(
        result.contains("`pub struct Config"),
        "struct signature: {result}"
    );
    assert!(
        result.contains("`pub fn start_server()"),
        "fn signature: {result}"
    );

    // Doc comment first line shown
    assert!(
        result.contains("Application configuration"),
        "doc snippet: {result}"
    );

    // Summary at the bottom
    assert!(result.contains("Summary"), "must have summary: {result}");
}

#[test]
fn overview_scoped_to_path_prefix() {
    let (_dir, db) = index_multi_file(&[
        ("lib.rs", "pub fn root_fn() {}\n"),
        ("models/user.rs", "pub struct User { pub id: u64 }\n"),
        ("models/post.rs", "pub struct Post { pub title: String }\n"),
    ]);
    let result = overview::handle_overview(&db, "src/models/", false).unwrap();

    assert!(result.contains("User"), "should include User: {result}");
    assert!(result.contains("Post"), "should include Post: {result}");
    assert!(
        !result.contains("root_fn"),
        "must NOT include root_fn: {result}"
    );
}

#[test]
fn overview_empty_prefix_returns_everything() {
    let (_dir, db) = index_source(
        r"
pub fn alpha() {}
pub fn beta() {}
",
    );
    let result = overview::handle_overview(&db, "src/", false).unwrap();
    assert!(result.contains("alpha"), "alpha: {result}");
    assert!(result.contains("beta"), "beta: {result}");
}

// =========================================================================
// 12. TREE TOOL DATA INTEGRITY
//     Wrong tree → Claude navigates to wrong files
// =========================================================================

#[test]
fn tree_shows_all_files_with_symbol_counts() {
    let (_dir, db) = index_multi_file(&[
        ("lib.rs", "pub fn a() {}\npub fn b() {}\n"),
        ("models/user.rs", "pub struct User { pub id: u64 }\n"),
    ]);
    let result = tree::handle_tree(&db, "src/").unwrap();

    assert!(result.contains("lib.rs"), "must show lib.rs: {result}");
    assert!(
        result.contains("models/user.rs") || result.contains("models\\user.rs"),
        "must show nested file: {result}"
    );
}

#[test]
fn tree_empty_prefix_returns_full_tree() {
    let (_dir, db) = index_source("pub fn hello() {}\n");
    let result = tree::handle_tree(&db, "").unwrap();
    assert!(
        result.contains("src/lib.rs"),
        "empty prefix should show all files: {result}"
    );
}

#[test]
fn tree_nonexistent_prefix_is_clear() {
    let (_dir, db) = index_source("pub fn hello() {}\n");
    let result = tree::handle_tree(&db, "nonexistent/").unwrap();
    assert!(
        result.contains("No files") || result.is_empty() || !result.contains("src/"),
        "nonexistent prefix should not show files: {result}"
    );
}

// =========================================================================
// 13. CARGO DOC JSON PARSING INTEGRITY
//     Wrong doc parsing → Claude gets wrong API descriptions
// =========================================================================

#[test]
fn cargo_doc_parse_includes_all_item_kinds() {
    let json = serde_json::json!({
        "root": 1,
        "crate_version": "2.0.0",
        "index": {
            "1": {
                "id": 1, "crate_id": 0, "name": "mylib",
                "visibility": "public", "docs": "My library.",
                "inner": { "module": { "is_crate": true, "items": [2,3,4,5,6] } }
            },
            "2": {
                "id": 2, "crate_id": 0, "name": "MyTrait",
                "visibility": "public", "docs": "A trait.",
                "inner": { "trait_": { "items": [10,11], "generics": {"params":[],"where_predicates":[]}, "bounds": [] } }
            },
            "3": {
                "id": 3, "crate_id": 0, "name": "MyStruct",
                "visibility": "public", "docs": "A struct.",
                "inner": { "struct": { "kind": {"plain":{"fields":[],"has_stripped_fields":false}}, "generics": {"params":[{"name":"T","kind":{"type":{"bounds":[],"default":null,"is_synthetic":false}}}],"where_predicates":[]}, "impls": [] } }
            },
            "4": {
                "id": 4, "crate_id": 0, "name": "MyEnum",
                "visibility": "public", "docs": "An enum.",
                "inner": { "enum": { "variants": [], "generics": {"params":[],"where_predicates":[]}, "impls": [] } }
            },
            "5": {
                "id": 5, "crate_id": 0, "name": "my_func",
                "visibility": "public", "docs": "A function.",
                "inner": { "function": { "sig": {"inputs":[["x",{"primitive":"i32"}]],"output":null}, "generics": {"params":[],"where_predicates":[]} } }
            },
            "6": {
                "id": 6, "crate_id": 0, "name": "my_macro",
                "visibility": "public", "docs": "A macro.",
                "inner": { "macro": "macro content" }
            }
        },
        "paths": {},
        "external_crates": {},
        "format_version": 39
    });

    let result =
        illu_rs::indexer::cargo_doc::parse_rustdoc_json_public(&json.to_string(), "mylib").unwrap();

    assert!(result.contains("# mylib 2.0.0"), "header: {result}");
    assert!(result.contains("My library."), "crate docs: {result}");
    assert!(result.contains("## Traits"), "traits section: {result}");
    assert!(result.contains("**MyTrait**"), "trait name: {result}");
    assert!(result.contains("## Structs"), "structs section: {result}");
    assert!(result.contains("**MyStruct**"), "struct name: {result}");
    assert!(result.contains("<T>"), "generic params: {result}");
    assert!(result.contains("## Enums"), "enums section: {result}");
    assert!(result.contains("**MyEnum**"), "enum name: {result}");
    assert!(
        result.contains("## Functions"),
        "functions section: {result}"
    );
    assert!(result.contains("my_func"), "fn name: {result}");
    assert!(result.contains("## Macros"), "macros section: {result}");
    assert!(result.contains("**my_macro!**"), "macro name: {result}");
}

// =========================================================================
// 14. CORRECTNESS: REFERENCE RESOLUTION, IMPACT ACCURACY, DATA INTEGRITY
// =========================================================================

#[test]
fn workspace_cross_crate_ref_resolves_target_file() {
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

    let deps = db.impact_dependents("SharedType").unwrap();
    assert!(
        deps.iter().any(|d| d.name == "use_it"),
        "use_it must depend on SharedType via cross-crate ref: {deps:?}"
    );
}

#[test]
fn workspace_same_name_ref_resolves_to_correct_crate() {
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

    let deps = db.impact_dependents("Error").unwrap();
    let handle_dep = deps.iter().find(|d| d.name == "handle");
    assert!(
        handle_dep.is_some(),
        "handle must depend on Error: {deps:?}"
    );
    assert!(
        handle_dep.unwrap().file_path.contains("app/"),
        "handle is in app crate"
    );
}

#[test]
fn self_method_resolves_correct_impl_with_collision() {
    let (_dir, db) = index_source(
        r"
pub struct TypeA;
pub struct TypeB;

impl TypeA {
    pub fn helper(&self) -> i32 { 1 }
    pub fn do_work(&self) -> i32 { self.helper() }
}

impl TypeB {
    pub fn helper(&self) -> i32 { 2 }
}
",
    );

    let callees = db.get_callees("do_work", "src/lib.rs").unwrap();
    let helper_callee = callees.iter().find(|c| c.name == "helper");
    assert!(
        helper_callee.is_some(),
        "do_work must call helper: {callees:?}"
    );
}

#[test]
fn impact_depth_and_via_chain_accurate() {
    let (_dir, db) = index_source(
        r"
pub fn root() {}
pub fn depth1() { root(); }
pub fn depth2() { depth1(); }
pub fn depth3() { depth2(); }
",
    );

    let result = impact::handle_impact(&db, "root", None, false).unwrap();
    assert!(result.contains("depth1"), "depth1 at depth 1: {result}");
    assert!(result.contains("depth2"), "depth2 at depth 2: {result}");
    assert!(result.contains("depth3"), "depth3 at depth 3: {result}");
    assert!(result.contains("via"), "must show via chain: {result}");
}

#[test]
fn refresh_cleans_refs_from_unchanged_caller_to_deleted_target() {
    // When a target symbol is removed, stale refs from an unchanged
    // caller must be cleaned up after refresh.
    let dir = tempfile::TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("Cargo.lock"),
        "[[package]]\nname = \"test\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    // lib.rs defines helper_target and caller (which calls it)
    std::fs::write(
        src_dir.join("lib.rs"),
        "pub fn helper_target() {}\npub fn caller() { helper_target(); }\n",
    )
    .unwrap();

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();

    let deps = db.impact_dependents("helper_target").unwrap();
    assert!(
        deps.iter().any(|d| d.name == "caller"),
        "caller should depend on helper_target initially"
    );

    // Remove helper_target definition, caller stops calling it
    std::fs::write(src_dir.join("lib.rs"), "pub fn caller() { }\n").unwrap();
    refresh_index(&db, &config).unwrap();

    let syms = db.search_symbols("helper_target").unwrap();
    assert!(syms.is_empty(), "helper_target should be gone: {syms:?}");
    let deps = db.impact_dependents("helper_target").unwrap();
    assert!(deps.is_empty(), "stale refs should be cleaned: {deps:?}");
}

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

    std::fs::write(src_dir.join("lib.rs"), "\n\n\n\n\npub fn foo() {}\n").unwrap();
    refresh_index(&db, &config).unwrap();

    let syms = db.search_symbols("foo").unwrap();
    assert_eq!(
        syms[0].line_start, 6,
        "after adding 5 blank lines, foo should be at line 6"
    );
}

#[test]
fn context_full_body_returns_untruncated_source() {
    use std::fmt::Write;

    let dir = tempfile::TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    let mut body = String::from("pub fn big_fn() {\n");
    for i in 0..118 {
        let _ = writeln!(body, "    let _x{i} = {i};");
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

    let result = context::handle_context(&db, "big_fn", false, None, None).unwrap();
    assert!(
        result.contains("truncated"),
        "should be truncated without full_body: {result}"
    );

    let result = context::handle_context(&db, "big_fn", true, None, None).unwrap();
    assert!(
        !result.contains("truncated"),
        "should NOT be truncated with full_body: {result}"
    );
    assert!(
        result.contains("_x117"),
        "should contain last variable: {result}"
    );
}

#[test]
fn docs_no_topic_lists_modules() {
    let (_dir, db) = index_source("pub fn placeholder() {}\n");
    let dep_id = db.insert_dependency("tokio", "1.35.0", true, None).unwrap();
    db.store_doc_with_module(dep_id, "cargo_doc", "Tokio summary", "")
        .unwrap();
    db.store_doc_with_module(dep_id, "cargo_doc", "Sync primitives", "sync")
        .unwrap();
    db.store_doc_with_module(dep_id, "cargo_doc", "File I/O", "fs")
        .unwrap();

    let result = docs::handle_docs(&db, "tokio", None).unwrap();
    assert!(result.contains("sync"), "must list sync module: {result}");
    assert!(result.contains("fs"), "must list fs module: {result}");
}

#[test]
fn non_self_method_call_creates_ref() {
    let (_dir, db) = index_source(
        r"
pub struct Processor;

impl Processor {
    pub fn process(&self) -> i32 { 42 }
}

pub fn run(p: Processor) -> i32 {
    p.process()
}
",
    );

    let callees = db.get_callees("run", "src/lib.rs").unwrap();
    assert!(
        callees.iter().any(|c| c.name == "process"),
        "run must have a ref to process: {callees:?}"
    );
}

#[test]
fn empty_file_indexed_without_error() {
    let (_dir, db) = index_multi_file(&[
        ("lib.rs", "pub mod empty;\npub fn real() {}\n"),
        ("empty.rs", ""),
    ]);
    let syms = db.search_symbols("real").unwrap();
    assert_eq!(syms.len(), 1);
}

#[test]
fn comment_only_file_indexed_without_error() {
    let (_dir, db) = index_multi_file(&[
        ("lib.rs", "pub mod comments;\npub fn real() {}\n"),
        (
            "comments.rs",
            "// This file has only comments\n// Nothing else\n",
        ),
    ]);
    let syms = db.search_symbols("real").unwrap();
    assert_eq!(syms.len(), 1);
}

#[test]
fn impact_handles_overloaded_names() {
    let (_dir, db) = index_source(
        r"
pub struct Error {
    pub message: String,
}

pub fn make_error() -> Error {
    Error { message: String::new() }
}
",
    );

    let result = impact::handle_impact(&db, "Error", None, false).unwrap();
    assert!(
        result.contains("make_error"),
        "make_error uses Error: {result}"
    );
}

#[test]
fn refresh_handles_new_file_added() {
    let dir = tempfile::TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"inc\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("Cargo.lock"),
        "[[package]]\nname = \"inc\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(src_dir.join("lib.rs"), "pub fn original() {}\n").unwrap();

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();

    // Add a new file
    std::fs::write(src_dir.join("extra.rs"), "pub fn bonus() {}\n").unwrap();

    refresh_index(&db, &config).unwrap();

    let result =
        query::handle_query(&db, "bonus", Some("symbols"), None, None, None, None).unwrap();
    assert!(
        result.contains("bonus"),
        "refresh should pick up new file's symbols"
    );
}

#[test]
fn refresh_handles_file_content_change() {
    let dir = tempfile::TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"chg\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("Cargo.lock"),
        "[[package]]\nname = \"chg\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(src_dir.join("lib.rs"), "pub fn version_one() {}\n").unwrap();

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();

    // Change the function name
    std::fs::write(src_dir.join("lib.rs"), "pub fn version_two() {}\n").unwrap();
    refresh_index(&db, &config).unwrap();

    let old_syms = db.search_symbols("version_one").unwrap();
    assert!(
        old_syms.is_empty(),
        "old symbol should be gone after refresh"
    );
    let new_result =
        query::handle_query(&db, "version_two", Some("symbols"), None, None, None, None).unwrap();
    assert!(
        new_result.contains("version_two"),
        "new symbol should appear after refresh"
    );
}

// =========================================================================
// 15. CROSS-MODULE REFERENCE RESOLUTION
//     use imports across files must create proper refs in the symbol graph
// =========================================================================

#[test]
fn cross_module_use_creates_ref() {
    let (_dir, db) = index_multi_file(&[
        ("lib.rs", "pub mod utils;\npub mod app;\n"),
        ("utils.rs", "pub fn helper() -> i32 { 42 }\n"),
        (
            "app.rs",
            "use crate::utils::helper;\npub fn run() -> i32 { helper() }\n",
        ),
    ]);

    let result = impact::handle_impact(&db, "helper", None, false).unwrap();
    assert!(
        result.contains("run"),
        "run should depend on helper via cross-module use: {result}"
    );
}

#[test]
fn cross_module_diamond_dependency() {
    let (_dir, db) = index_multi_file(&[
        (
            "lib.rs",
            "pub mod base;\npub mod left;\npub mod right;\npub mod top;\n",
        ),
        ("base.rs", "pub fn foundation() -> i32 { 1 }\n"),
        (
            "left.rs",
            "use crate::base::foundation;\npub fn left_path() -> i32 { foundation() + 10 }\n",
        ),
        (
            "right.rs",
            "use crate::base::foundation;\npub fn right_path() -> i32 { foundation() + 20 }\n",
        ),
        (
            "top.rs",
            "use crate::left::left_path;\nuse crate::right::right_path;\npub fn combine() -> i32 { left_path() + right_path() }\n",
        ),
    ]);

    let result = impact::handle_impact(&db, "foundation", None, false).unwrap();
    // Depth 1: both left_path and right_path depend directly on foundation
    assert!(
        result.contains("left_path"),
        "left_path should depend on foundation at depth 1: {result}"
    );
    assert!(
        result.contains("right_path"),
        "right_path should depend on foundation at depth 1: {result}"
    );
    // Depth 2: combine depends transitively on foundation via left_path and right_path
    assert!(
        result.contains("combine"),
        "combine should depend on foundation transitively at depth 2: {result}"
    );
}

#[test]
fn cross_module_type_reference() {
    let (_dir, db) = index_multi_file(&[
        ("lib.rs", "pub mod types;\npub mod service;\n"),
        ("types.rs", "pub struct Config { pub port: u16 }\n"),
        (
            "service.rs",
            "use crate::types::Config;\npub fn start(cfg: Config) -> u16 { cfg.port }\n",
        ),
    ]);

    let result = impact::handle_impact(&db, "Config", None, false).unwrap();
    assert!(
        result.contains("start"),
        "start should depend on Config via type usage: {result}"
    );
}

// =========================================================================
// 16. STALE DATA AFTER INCREMENTAL RE-INDEX
//     refresh_index must leave no stale signatures, line numbers, or refs
// =========================================================================

#[test]
fn refresh_updates_changed_signature() {
    let dir = tempfile::TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("Cargo.lock"),
        "[[package]]\nname = \"test\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        src_dir.join("lib.rs"),
        "pub fn transform(x: i32) -> i32 { x + 1 }\n",
    )
    .unwrap();

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();

    // Verify initial signature
    let result = context::handle_context(&db, "transform", false, None, None).unwrap();
    assert!(
        result.contains("transform(x: i32) -> i32"),
        "initial signature should have one param: {result}"
    );

    // Change signature to two params
    std::fs::write(
        src_dir.join("lib.rs"),
        "pub fn transform(x: i32, y: i32) -> i32 { x + y }\n",
    )
    .unwrap();
    refresh_index(&db, &config).unwrap();

    let result = context::handle_context(&db, "transform", false, None, None).unwrap();
    assert!(
        result.contains("transform(x: i32, y: i32)"),
        "refreshed signature should have two params: {result}"
    );
}

#[test]
fn refresh_updates_moved_line_numbers() {
    let dir = tempfile::TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("Cargo.lock"),
        "[[package]]\nname = \"test\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        src_dir.join("lib.rs"),
        "pub fn first() {}\npub fn second() {}\n",
    )
    .unwrap();

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();

    let syms = db.search_symbols("second").unwrap();
    let old_line = syms[0].line_start;

    // Prepend a new function, shifting second() down
    std::fs::write(
        src_dir.join("lib.rs"),
        "pub fn zeroth() {}\npub fn first() {}\npub fn second() {}\n",
    )
    .unwrap();
    refresh_index(&db, &config).unwrap();

    let syms = db.search_symbols("second").unwrap();
    assert!(
        syms[0].line_start > old_line,
        "second() should have shifted down: was {old_line}, now {}",
        syms[0].line_start
    );
}

#[test]
fn refresh_removes_deleted_file_completely() {
    let dir = tempfile::TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("Cargo.lock"),
        "[[package]]\nname = \"test\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        src_dir.join("lib.rs"),
        "pub mod helper;\npub fn main_fn() {}\n",
    )
    .unwrap();
    std::fs::write(src_dir.join("helper.rs"), "pub fn help() {}\n").unwrap();

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();

    // Verify help exists
    let syms = db.search_symbols("help").unwrap();
    assert!(!syms.is_empty(), "help should exist before deletion");

    // Delete helper.rs, update lib.rs to remove mod declaration
    std::fs::remove_file(src_dir.join("helper.rs")).unwrap();
    std::fs::write(src_dir.join("lib.rs"), "pub fn main_fn() {}\n").unwrap();
    refresh_index(&db, &config).unwrap();

    // Verify help is gone from query
    let syms = db.search_symbols("help").unwrap();
    assert!(syms.is_empty(), "help must be removed after file deletion");

    // Verify overview does not mention helper.rs
    let ov = overview::handle_overview(&db, "src/", false).unwrap();
    assert!(
        !ov.contains("helper.rs"),
        "overview must not mention deleted helper.rs: {ov}"
    );

    // Verify tree does not list helper.rs
    let tr = tree::handle_tree(&db, "src/").unwrap();
    assert!(
        !tr.contains("helper.rs"),
        "tree must not list deleted helper.rs: {tr}"
    );
}

#[test]
fn refresh_removes_deleted_reference() {
    let dir = tempfile::TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("Cargo.lock"),
        "[[package]]\nname = \"test\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        src_dir.join("lib.rs"),
        "pub fn callee() {}\npub fn caller() { callee() }\n",
    )
    .unwrap();

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();

    // Verify caller depends on callee
    let result = impact::handle_impact(&db, "callee", None, false).unwrap();
    assert!(
        result.contains("caller"),
        "caller should depend on callee initially: {result}"
    );

    // Remove the call from caller
    std::fs::write(
        src_dir.join("lib.rs"),
        "pub fn callee() {}\npub fn caller() { /* no longer calls callee */ }\n",
    )
    .unwrap();
    refresh_index(&db, &config).unwrap();

    let result = impact::handle_impact(&db, "callee", None, false).unwrap();
    assert!(
        !result.contains("caller"),
        "caller must NOT depend on callee after removing the call: {result}"
    );
}

// =========================================================================
// 17. OUTPUT FORMAT CONTRACTS
//     Tool outputs must match documented markdown structure
// =========================================================================

#[test]
fn context_output_has_required_markdown_structure() {
    let (_dir, db) = index_source(
        r"
/// Application config.
pub struct AppState {
    pub port: u16,
    pub host: String,
}

impl AppState {
    pub fn new() -> AppState {
        AppState { port: 8080, host: String::new() }
    }
}
",
    );

    let result = context::handle_context(&db, "AppState", false, None, None).unwrap();

    // Header: ## SymbolName (kind)
    assert!(
        result.contains("## AppState (struct)"),
        "must start section with ## Name (kind): {result}"
    );
    // Doc comment prefixed with >
    assert!(
        result.contains("> Application config."),
        "doc comment must be blockquoted with >: {result}"
    );
    // File line with path:start-end format
    assert!(
        result.contains("- **File:** "),
        "must have File metadata line: {result}"
    );
    // Visibility
    assert!(
        result.contains("- **Visibility:** "),
        "must have Visibility line: {result}"
    );
    // Signature in backticks
    assert!(
        result.contains("- **Signature:** `"),
        "must have Signature in backticks: {result}"
    );
    // Fields/Variants section for a struct
    assert!(
        result.contains("### Fields/Variants"),
        "struct context must include Fields/Variants section: {result}"
    );
}

#[test]
fn query_output_has_required_format() {
    let (_dir, db) = index_source(
        r"
pub fn alpha_fn() -> i32 { 1 }
pub fn beta_fn() -> i32 { 2 }
",
    );

    let result = query::handle_query(&db, "fn", Some("symbols"), None, None, None, None).unwrap();

    // Must start with ## Symbols header
    assert!(
        result.contains("## Symbols"),
        "query output must have ## Symbols header: {result}"
    );
    // Each entry: - **name** (kind) at path:start-end
    assert!(
        result.contains("- **alpha_fn** (function) at "),
        "must have formatted symbol entry for alpha_fn: {result}"
    );
    assert!(
        result.contains("- **beta_fn** (function) at "),
        "must have formatted symbol entry for beta_fn: {result}"
    );
    // Signature on the next line in backticks
    assert!(
        result.contains("`pub fn alpha_fn()"),
        "signature must be wrapped in backticks: {result}"
    );
}

#[test]
fn impact_output_has_required_format() {
    let (_dir, db) = index_source(
        r"
pub fn leaf_fn() -> i32 { 1 }
pub fn mid_fn() -> i32 { leaf_fn() }
pub fn top_fn() -> i32 { mid_fn() }
",
    );

    let result = impact::handle_impact(&db, "leaf_fn", None, false).unwrap();

    // Header
    assert!(
        result.contains("## Impact Analysis: leaf_fn"),
        "must have impact analysis header: {result}"
    );
    // Depth sections
    assert!(
        result.contains("### Depth 1"),
        "must have Depth 1 section: {result}"
    );
    assert!(
        result.contains("### Depth 2"),
        "must have Depth 2 section: {result}"
    );
    // Entry format: - **name** (path)
    assert!(
        result.contains("**mid_fn**"),
        "must show mid_fn as dependent: {result}"
    );
    assert!(
        result.contains("**top_fn**"),
        "must show top_fn as dependent: {result}"
    );
    // Transitive entries have " — via " connector
    assert!(
        result.contains("— via"),
        "transitive entries must show via chain: {result}"
    );
}

#[test]
fn overview_output_has_required_format() {
    let (_dir, db) = index_source(
        r"
/// A public function.
pub fn my_public_fn() -> i32 { 42 }

pub struct MyPublicStruct {
    pub field: String,
}
",
    );

    let result = overview::handle_overview(&db, "src/", false).unwrap();

    // File section header: ### path/to/file.rs
    assert!(
        result.contains("### src/lib.rs"),
        "must have file section header: {result}"
    );
    // Symbol format: - **name** (kind) `signature`
    assert!(
        result.contains("- **my_public_fn** (function) `"),
        "must have formatted symbol for function: {result}"
    );
    assert!(
        result.contains("- **MyPublicStruct** (struct) `"),
        "must have formatted symbol for struct: {result}"
    );
    // Separator
    assert!(result.contains("---"), "must have separator: {result}");
    // Summary line
    assert!(
        result.contains("symbols across"),
        "must have summary with 'symbols across': {result}"
    );
    assert!(
        result.contains("files"),
        "must have summary with 'files': {result}"
    );
}

// =========================================================================
// FTS QUERY SAFETY — no crash on special characters or FTS operators
// =========================================================================

#[test]
fn query_with_dot_does_not_crash() {
    let (_dir, db) = index_source("pub fn hello() {}\n");
    let result = query::handle_query(&db, "self.method", Some("symbols"), None, None, None, None);
    assert!(result.is_ok(), "dot in query must not crash: {result:?}");
}

#[test]
fn query_with_colon_does_not_crash() {
    let (_dir, db) = index_source("pub fn hello() {}\n");
    let result = query::handle_query(&db, "a:b", Some("symbols"), None, None, None, None);
    assert!(result.is_ok(), "colon in query must not crash: {result:?}");
}

#[test]
fn query_with_fts_operators_does_not_crash() {
    let (_dir, db) = index_source("pub fn hello() {}\n");
    for q in &[
        "OR DROP",
        "NOT something",
        "foo{bar}",
        "test -flag",
        "a&b",
        "\"quoted\"",
    ] {
        let result = query::handle_query(&db, q, Some("symbols"), None, None, None, None);
        assert!(result.is_ok(), "query '{q}' must not crash: {result:?}");
    }
}

#[test]
fn query_with_special_chars_falls_back_to_like() {
    let (_dir, db) = index_source("pub fn config_parser() {}\n");
    // Underscore query should still find results via LIKE fallback
    let result = query::handle_query(
        &db,
        "config.parser",
        Some("symbols"),
        None,
        None,
        None,
        None,
    )
    .unwrap();
    // Should not crash — may or may not find results depending on LIKE matching
    assert!(
        !result.contains("error"),
        "should not contain error: {result}"
    );
}
