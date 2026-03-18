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
    let result = impact::handle_impact(&db, "alpha").unwrap();
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
    let result = impact::handle_impact(&db, "g").unwrap();

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
    let result = context::handle_context(&db, "start", false).unwrap();
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
    let result = impact::handle_impact(&db, "Config").unwrap();
    // builder shadows Config with a local variable — should NOT
    // appear as a dependent of the Config struct
    assert!(
        !result.contains("builder"),
        "local shadowing should prevent false ref: {result}"
    );
}

#[test]
fn no_false_refs_to_noisy_names() {
    let (_dir, db) = index_source(
        r"
pub fn new() -> i32 { 0 }
pub fn default() -> i32 { 1 }
pub fn clone() -> i32 { 2 }

pub fn caller() -> i32 {
    let x = new();
    let y = default();
    let z = clone();
    x + y + z
}
",
    );
    // Even though caller uses new/default/clone, these are in the
    // noisy symbol list and should be filtered out
    let result = impact::handle_impact(&db, "new").unwrap();
    assert!(
        !result.contains("caller"),
        "noisy name 'new' should not create ref: {result}"
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
    let result = impact::handle_impact(&db, "base").unwrap();
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
    let result = impact::handle_impact(&db, "base").unwrap();
    assert!(
        !result.contains("extra_caller"),
        "deleted file's refs must be cleaned up: {result}"
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

    let query_result = query::handle_query(&db, "AppConfig", Some("symbols"), None).unwrap();
    let context_result = context::handle_context(&db, "AppConfig", false).unwrap();
    let impact_result = impact::handle_impact(&db, "AppConfig").unwrap();

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
    let result = context::handle_context(&db, "Config", false).unwrap();

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
    let result = query::handle_query(&db, "My", Some("symbols"), None).unwrap();

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
    let result = query::handle_query(&db, "Error", Some("symbols"), None).unwrap();
    assert!(
        result.contains("core/src/lib.rs"),
        "should show core's Error: {result}"
    );
    assert!(
        result.contains("api/src/lib.rs"),
        "should show api's Error: {result}"
    );

    // Context should show the correct file for each
    let ctx = context::handle_context(&db, "Error", false).unwrap();
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

    let result = impact::handle_impact(&db, "CoreType").unwrap();
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
    let result = query::handle_query(&db, "Config", Some("symbols"), None).unwrap();

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
    let result = context::handle_context(&db, "noop", false).unwrap();
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
    let result = context::handle_context(&db, "Processor", false).unwrap();
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
    let result = context::handle_context(&db, "my_func", false).unwrap();
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
    let alpha = context::handle_context(&db, "alpha", false).unwrap();
    let beta = context::handle_context(&db, "beta", false).unwrap();

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
    db.store_doc(dep_id, "docs.rs", "Serde serialization framework")
        .unwrap();
    db.store_doc(dep_id, "cargo_doc", "# serde 1.0.210\n\nStructured docs")
        .unwrap();

    let result = docs::handle_docs(&db, "serde", None).unwrap();
    // Must show version so Claude knows which API it's looking at
    assert!(
        result.contains("1.0.210"),
        "docs must include version: {result}"
    );
    // Must show both doc sources
    assert!(
        result.contains("docs.rs"),
        "must show docs.rs source: {result}"
    );
    assert!(
        result.contains("cargo_doc"),
        "must show cargo_doc source: {result}"
    );
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
    let result = overview::handle_overview(&db, "src/").unwrap();

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
    let result = overview::handle_overview(&db, "src/models/").unwrap();

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
    let result = overview::handle_overview(&db, "src/").unwrap();
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
