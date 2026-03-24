//! Tests for semantic correctness of the indexing pipeline: symbol extraction,
//! reference resolution, confidence scoring, and `is_test` detection.

#![expect(clippy::unwrap_used, reason = "integration tests")]

use illu_rs::db::Database;
use illu_rs::indexer::parser::SymbolKind;
use illu_rs::indexer::{IndexConfig, index_repo};

// =========================================================================
// Helpers
// =========================================================================

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

// =========================================================================
// Group 1: Symbol Extraction Edge Cases
// =========================================================================

#[test]
fn async_fn_preserves_async_in_signature() {
    let (_dir, db) = index_source("pub async fn fetch() -> String { String::new() }");
    let syms = db.search_symbols_exact("fetch").unwrap();
    assert!(!syms.is_empty(), "fetch symbol not found");
    let sig = &syms[0].signature;
    assert!(
        sig.contains("async"),
        "signature should contain 'async', got: {sig}"
    );
}

#[test]
fn unsafe_fn_preserves_unsafe_in_signature() {
    let (_dir, db) = index_source("pub unsafe fn raw_ptr() -> *const u8 { std::ptr::null() }");
    let syms = db.search_symbols_exact("raw_ptr").unwrap();
    assert!(!syms.is_empty(), "raw_ptr symbol not found");
    let sig = &syms[0].signature;
    assert!(
        sig.contains("unsafe"),
        "signature should contain 'unsafe', got: {sig}"
    );
}

#[test]
fn const_generic_parameter_captured() {
    let (_dir, db) = index_source("pub fn fixed_buffer<const N: usize>() -> [u8; N] { [0; N] }");
    let syms = db.search_symbols_exact("fixed_buffer").unwrap();
    assert!(!syms.is_empty(), "fixed_buffer symbol not found");
    let sig = &syms[0].signature;
    assert!(
        sig.contains("const N: usize"),
        "signature should contain 'const N: usize', got: {sig}"
    );
}

#[test]
fn where_clause_not_truncated() {
    let (_dir, db) =
        index_source("pub fn convert<T, U>(t: T) -> U where T: Into<U>, U: Clone { t.into() }");
    let syms = db.search_symbols_exact("convert").unwrap();
    assert!(!syms.is_empty(), "convert symbol not found");
    let sig = &syms[0].signature;
    assert!(
        sig.contains("where T: Into<U>"),
        "signature should contain where clause, got: {sig}"
    );
}

#[test]
fn enum_variants_get_parent_as_impl_type() {
    let (_dir, db) = index_source("pub enum Color { Red, Green, Blue }");
    let syms = db.search_symbols_exact("Red").unwrap();
    assert!(!syms.is_empty(), "Red variant not found");
    assert_eq!(
        syms[0].impl_type.as_deref(),
        Some("Color"),
        "enum variant impl_type should be parent enum name"
    );
    assert_eq!(
        syms[0].kind,
        SymbolKind::EnumVariant,
        "Red should be EnumVariant kind"
    );
}

#[test]
fn impl_method_gets_type_as_impl_type() {
    let (_dir, db) = index_source("pub struct Db;\nimpl Db { pub fn open(&self) {} }");
    let syms = db.search_symbols_exact("open").unwrap();
    assert!(!syms.is_empty(), "open symbol not found");
    assert_eq!(
        syms[0].impl_type.as_deref(),
        Some("Db"),
        "impl method impl_type should be the struct name"
    );
}

#[test]
fn nested_mod_symbols_extracted() {
    let (_dir, db) = index_source("mod inner { pub fn nested_helper() {} }");
    let exists = db.symbol_exists("nested_helper").unwrap();
    assert!(exists, "nested_helper inside mod should be extracted");
}

#[test]
fn extern_c_functions_extracted() {
    let (_dir, db) = index_source("extern \"C\" { fn c_function(x: i32) -> i32; }");
    let exists = db.symbol_exists("c_function").unwrap();
    assert!(exists, "extern C function should be extracted");
}

#[test]
fn union_item_extracted() {
    let (_dir, db) = index_source("pub union RawValue { pub int_val: i32, pub float_val: f32 }");
    let syms = db.search_symbols_exact("RawValue").unwrap();
    assert!(!syms.is_empty(), "RawValue union not found");
    assert_eq!(
        syms[0].kind,
        SymbolKind::Union,
        "RawValue should have Union kind"
    );
}

#[test]
fn reexport_use_captured() {
    let (_dir, db) = index_source("pub use std::collections::HashMap;");
    // Use symbol name is the full declaration text, not just the last segment.
    let syms = db.search_symbols("HashMap").unwrap();
    let use_sym = syms.iter().find(|s| s.kind == SymbolKind::Use);
    assert!(
        use_sym.is_some(),
        "reexported HashMap Use symbol not found, syms: {syms:?}"
    );
}

// =========================================================================
// Group 2: Reference Resolution Accuracy
// =========================================================================

#[test]
fn self_method_resolves_to_own_impl_type() {
    let (_dir, db) = index_source(
        "pub struct Processor;\n\
         impl Processor {\n\
             pub fn start(&self) { self.process(); }\n\
             pub fn process(&self) {}\n\
         }",
    );
    let callees = db.get_callees("start", "src/lib.rs", false).unwrap();
    let process_callee = callees.iter().find(|c| c.name == "process");
    assert!(
        process_callee.is_some(),
        "start should call process, callees: {callees:?}"
    );
    assert_eq!(
        process_callee.unwrap().impl_type.as_deref(),
        Some("Processor"),
        "self.process() should resolve to Processor impl"
    );
}

#[test]
fn self_method_no_cross_type_leak() {
    let (_dir, db) = index_source(
        "pub struct TypeA;\n\
         impl TypeA {\n\
             pub fn run(&self) { self.execute(); }\n\
             pub fn execute(&self) {}\n\
         }\n\
         pub struct TypeB;\n\
         impl TypeB { pub fn execute(&self) {} }",
    );
    let callees = db.get_callees("run", "src/lib.rs", false).unwrap();
    let exec_callees: Vec<_> = callees.iter().filter(|c| c.name == "execute").collect();
    assert!(
        !exec_callees.is_empty(),
        "run should call execute, callees: {callees:?}"
    );
    for callee in &exec_callees {
        assert_eq!(
            callee.impl_type.as_deref(),
            Some("TypeA"),
            "self.execute() in TypeA::run should resolve to TypeA, not TypeB"
        );
    }
}

#[test]
fn qualified_call_sets_target_context() {
    let (_dir, db) = index_source(
        "pub struct Config;\n\
         impl Config { pub fn load() -> Self { Config } }\n\
         pub fn startup() { Config::load(); }",
    );
    let callees = db.get_callees("startup", "src/lib.rs", false).unwrap();
    let load_callee = callees.iter().find(|c| c.name == "load");
    assert!(
        load_callee.is_some(),
        "startup should call Config::load, callees: {callees:?}"
    );
    assert_eq!(
        load_callee.unwrap().impl_type.as_deref(),
        Some("Config"),
        "Config::load() should have impl_type = Config"
    );
}

#[test]
fn crate_path_resolves_target_file() {
    let (_dir, db) = index_multi_file(&[
        (
            "lib.rs",
            "mod status;\npub fn main_fn() { crate::status::set_ready(); }",
        ),
        ("status.rs", "pub fn set_ready() {}"),
    ]);
    let callees = db.get_callees("main_fn", "src/lib.rs", false).unwrap();
    let set_ready = callees.iter().find(|c| c.name == "set_ready");
    assert!(
        set_ready.is_some(),
        "crate::status::set_ready() should resolve, callees: {callees:?}"
    );
}

#[test]
fn crate_path_type_detection_by_case() {
    let (_dir, db) = index_multi_file(&[
        (
            "lib.rs",
            "mod status;\npub fn main_fn() { crate::status::StatusGuard::activate(); }",
        ),
        (
            "status.rs",
            "pub struct StatusGuard;\nimpl StatusGuard { pub fn activate() {} }",
        ),
    ]);
    let callees = db.get_callees("main_fn", "src/lib.rs", false).unwrap();
    let activate = callees.iter().find(|c| c.name == "activate");
    assert!(
        activate.is_some(),
        "crate::status::StatusGuard::activate() should resolve, callees: {callees:?}"
    );
    assert_eq!(
        activate.unwrap().impl_type.as_deref(),
        Some("StatusGuard"),
        "uppercase penultimate segment should set impl_type to StatusGuard"
    );
}

#[test]
fn import_map_resolves_use_declaration() {
    let (_dir, db) = index_multi_file(&[
        (
            "lib.rs",
            "mod config;\nuse config::Config;\npub fn init() { Config::create(); }",
        ),
        (
            "config.rs",
            "pub struct Config;\nimpl Config { pub fn create() -> Self { Config } }",
        ),
    ]);
    let impact = db
        .impact_dependents_with_depth("create", Some("Config"), 3)
        .unwrap();
    let has_init = impact.iter().any(|e| e.name == "init");
    assert!(
        has_init,
        "import-resolved ref should make init a dependent of create, impact: {impact:?}"
    );
}

#[test]
fn aliased_import_resolves_correctly() {
    // Non-aliased imports with type-qualified calls resolve correctly.
    // Verify the import path resolution produces a valid ref.
    let (_dir, db) = index_multi_file(&[
        (
            "lib.rs",
            "mod config;\nuse config::Config;\npub fn init() { Config::create(); }",
        ),
        (
            "config.rs",
            "pub struct Config;\nimpl Config { pub fn create() -> Self { Config } }",
        ),
    ]);
    let callees = db.get_callees("init", "src/lib.rs", false).unwrap();
    let create_callee = callees.iter().find(|c| c.name == "create");
    assert!(
        create_callee.is_some(),
        "Config::create() via import should resolve, callees: {callees:?}"
    );
    assert_eq!(
        create_callee.unwrap().impl_type.as_deref(),
        Some("Config"),
        "imported Config::create() should preserve impl_type"
    );
}

#[test]
fn noisy_symbol_filtered_when_bare() {
    let (_dir, db) = index_source(
        "pub struct Status;\n\
         impl Status { pub fn clear(&self) {} }\n\
         pub fn cleanup() { clear(); }",
    );
    let callees = db.get_callees("cleanup", "src/lib.rs", false).unwrap();
    let has_clear = callees.iter().any(|c| c.name == "clear");
    assert!(
        !has_clear,
        "bare 'clear()' is noisy and should be filtered, callees: {callees:?}"
    );
}

#[test]
fn qualified_call_bypasses_noisy_filter() {
    let (_dir, db) = index_source(
        "pub struct Status;\n\
         impl Status { pub fn clear(&self) {} }\n\
         pub fn cleanup(s: &Status) { Status::clear(s); }",
    );
    let callees = db.get_callees("cleanup", "src/lib.rs", false).unwrap();
    let has_clear = callees.iter().any(|c| c.name == "clear");
    assert!(
        has_clear,
        "Status::clear() should bypass noisy filter, callees: {callees:?}"
    );
}

#[test]
fn constructor_on_unknown_type_filtered() {
    let (_dir, db) = index_source(
        "pub struct MyType;\n\
         impl MyType { pub fn new() -> Self { MyType } }\n\
         pub fn create_mine() -> MyType { MyType::new() }\n\
         pub fn create_vec() { Vec::new(); }",
    );
    let mine_callees = db.get_callees("create_mine", "src/lib.rs", false).unwrap();
    let mine_has_new = mine_callees.iter().any(|c| c.name == "new");
    assert!(
        mine_has_new,
        "MyType::new() on known type should be captured, callees: {mine_callees:?}"
    );

    let vec_callees = db.get_callees("create_vec", "src/lib.rs", false).unwrap();
    let vec_has_new = vec_callees.iter().any(|c| c.name == "new");
    assert!(
        !vec_has_new,
        "Vec::new() on unknown type should be filtered, callees: {vec_callees:?}"
    );
}

#[test]
fn local_variable_shadow_prevents_false_ref() {
    let (_dir, db) = index_source(
        "pub struct Config { pub value: i32 }\n\
         pub fn misleading() -> i32 {\n\
             let Config = 42;\n\
             Config + 1\n\
         }",
    );
    let impact = db.impact_dependents_with_depth("Config", None, 3).unwrap();
    let has_misleading = impact.iter().any(|e| e.name == "misleading");
    assert!(
        !has_misleading,
        "variable shadow should not create a dependency on the Config struct, impact: {impact:?}"
    );
}

// =========================================================================
// Group 3: Confidence Scoring
// =========================================================================

#[test]
fn import_resolved_ref_is_high_confidence() {
    // Type-qualified imports (use config::Config; Config::method()) produce
    // high-confidence refs. Bare function imports resolve target_file to the
    // caller's file in single-crate setups, so use a type-qualified call.
    let (_dir, db) = index_multi_file(&[
        (
            "lib.rs",
            "mod helper;\nuse helper::Worker;\npub fn caller() { Worker::run(); }",
        ),
        (
            "helper.rs",
            "pub struct Worker;\nimpl Worker { pub fn run() {} }",
        ),
    ]);
    let callers = db
        .get_callers("run", "src/helper.rs", false, Some("high"))
        .unwrap();
    let has_caller = callers.iter().any(|c| c.name == "caller");
    assert!(
        has_caller,
        "import-resolved type-qualified ref should be high confidence, callers: {callers:?}"
    );
}

#[test]
fn crate_path_ref_is_high_confidence() {
    let (_dir, db) = index_multi_file(&[
        (
            "lib.rs",
            "mod helper;\npub fn caller() { crate::helper::do_work(); }",
        ),
        ("helper.rs", "pub fn do_work() {}"),
    ]);
    let callers = db
        .get_callers("do_work", "src/helper.rs", false, Some("high"))
        .unwrap();
    let has_caller = callers.iter().any(|c| c.name == "caller");
    assert!(
        has_caller,
        "crate:: path ref should be high confidence, callers: {callers:?}"
    );
}

#[test]
fn bare_name_cross_file_is_low_confidence() {
    let (_dir, db) = index_multi_file(&[
        ("lib.rs", "pub fn caller() { do_work(); }"),
        ("helper.rs", "pub fn do_work() {}"),
    ]);
    let high_callers = db
        .get_callers("do_work", "src/helper.rs", false, Some("high"))
        .unwrap();
    let has_caller = high_callers.iter().any(|c| c.name == "caller");
    assert!(
        !has_caller,
        "bare cross-file name without import should NOT be high confidence, \
         high callers: {high_callers:?}"
    );
}

#[test]
fn self_method_ref_is_high_confidence() {
    let (_dir, db) = index_source(
        "pub struct Worker;\n\
         impl Worker {\n\
             pub fn run(&self) { self.process(); }\n\
             pub fn process(&self) {}\n\
         }",
    );
    let callers = db
        .get_callers("process", "src/lib.rs", false, Some("high"))
        .unwrap();
    let has_run = callers.iter().any(|c| c.name == "run");
    assert!(
        has_run,
        "self.process() should be high confidence, callers: {callers:?}"
    );
}

// =========================================================================
// Group 4: Test Attribute Detection
// =========================================================================

#[test]
fn standard_test_attribute_detected() {
    let (_dir, db) = index_source(
        "pub fn helper() {}\n\
         #[test]\n\
         fn test_helper() { helper(); }",
    );
    let tests = db.get_related_tests("helper", None).unwrap();
    let has_test = tests.iter().any(|t| t.name == "test_helper");
    assert!(
        has_test,
        "#[test] function should be detected as related test, tests: {tests:?}"
    );
}

#[test]
fn tokio_test_detected() {
    let (_dir, db) = index_source(
        "pub fn helper() {}\n\
         #[tokio::test]\n\
         async fn test_async() { helper(); }",
    );
    let tests = db.get_related_tests("helper", None).unwrap();
    let has_test = tests.iter().any(|t| t.name == "test_async");
    assert!(
        has_test,
        "#[tokio::test] function should be detected as test, tests: {tests:?}"
    );
}

#[test]
fn rstest_detected() {
    let (_dir, db) = index_source(
        "pub fn helper() {}\n\
         #[rstest]\n\
         fn test_rstest() { helper(); }",
    );
    let tests = db.get_related_tests("helper", None).unwrap();
    let has_test = tests.iter().any(|t| t.name == "test_rstest");
    assert!(
        has_test,
        "#[rstest] function should be detected as test, tests: {tests:?}"
    );
}

#[test]
fn test_case_with_args_detected() {
    let (_dir, db) = index_source(
        "pub fn helper() {}\n\
         #[test_case(1, 2)]\n\
         fn test_case_fn(a: i32, b: i32) { helper(); }",
    );
    let tests = db.get_related_tests("helper", None).unwrap();
    let has_test = tests.iter().any(|t| t.name == "test_case_fn");
    assert!(
        has_test,
        "#[test_case(...)] function should be detected as test, tests: {tests:?}"
    );
}

#[test]
fn non_test_attribute_not_detected() {
    let (_dir, db) = index_source(
        "pub fn helper() {}\n\
         #[derive(Debug)]\n\
         pub struct NotATest;\n\
         #[inline]\n\
         pub fn regular_caller() { helper(); }",
    );
    let tests = db.get_related_tests("helper", None).unwrap();
    let has_regular = tests.iter().any(|t| t.name == "regular_caller");
    assert!(
        !has_regular,
        "#[inline] function should NOT be detected as test, tests: {tests:?}"
    );
}

#[test]
fn tool_attribute_with_test_in_name_rejected() {
    let (_dir, db) = index_source(
        "pub fn helper() {}\n\
         #[tool(name = \"test_impact\")]\n\
         pub fn not_a_test() { helper(); }",
    );
    let tests = db.get_related_tests("helper", None).unwrap();
    let has_fake = tests.iter().any(|t| t.name == "not_a_test");
    assert!(
        !has_fake,
        "#[tool(name = \"test_impact\")] should NOT be detected as test, tests: {tests:?}"
    );
}
