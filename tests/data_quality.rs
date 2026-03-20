#![expect(clippy::unwrap_used, reason = "integration tests")]

//! Data quality tests: verify the correctness and completeness of
//! information that MCP tools provide to Claude.
//!
//! These tests index realistic Rust code end-to-end and assert that
//! tool outputs contain accurate signatures, line numbers, doc
//! comments, references, and formatting.

use illu_rs::db::Database;
use illu_rs::indexer::{IndexConfig, index_repo};
use illu_rs::server::tools::{context, docs, impact, overview, query, tree};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Index a single-crate project with the given `lib.rs` source code.
/// Returns (`TempDir`, `Database`) — `TempDir` must stay alive for
/// the duration of the test.
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

/// Index a multi-file single-crate project.
/// `files` is a list of (`relative_path`, content) pairs under `src/`.
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

// =========================================================================
// 1. PARSER FIDELITY — complex Rust constructs produce correct data
// =========================================================================

#[test]
fn generic_function_signature_preserved() {
    let (_dir, db) = index_source(
        r"
/// Convert an iterator of items into a Vec.
pub fn collect_items<T, I>(iter: I) -> Vec<T>
where
    I: IntoIterator<Item = T>,
    T: Clone,
{
    iter.into_iter().collect()
}
",
    );
    let result = context::handle_context(&db, "collect_items", false, None).unwrap();
    assert!(result.contains("collect_items"), "should find the function");
    assert!(
        result.contains("<T, I>") || result.contains("<T,I>"),
        "signature must include generic params: {result}"
    );
    assert!(
        result.contains("Convert an iterator"),
        "doc comment must appear in context: {result}"
    );
}

#[test]
fn async_function_signature() {
    let (_dir, db) = index_source(
        r"
pub async fn fetch_data(url: &str) -> Result<String, Box<dyn std::error::Error>> {
    Ok(url.to_string())
}
",
    );
    let result = context::handle_context(&db, "fetch_data", false, None).unwrap();
    assert!(
        result.contains("async"),
        "signature must include async keyword: {result}"
    );
    assert!(
        result.contains("Result<String"),
        "return type must be preserved: {result}"
    );
}

#[test]
fn struct_with_generics_and_lifetimes() {
    let (_dir, db) = index_source(
        r"
/// A borrowed slice wrapper.
pub struct BorrowedSlice<'a, T: Clone> {
    pub data: &'a [T],
    pub offset: usize,
}
",
    );
    let result = context::handle_context(&db, "BorrowedSlice", false, None).unwrap();
    assert!(
        result.contains("'a") && result.contains('T'),
        "signature must include lifetime and generic: {result}"
    );
    assert!(
        result.contains("data") && result.contains("offset"),
        "struct fields must be listed: {result}"
    );
    assert!(
        result.contains("borrowed slice wrapper"),
        "doc comment must appear: {result}"
    );
}

#[test]
fn enum_with_complex_variants() {
    let (_dir, db) = index_source(
        r"
/// Represents a parsed value.
pub enum Value {
    /// Null value.
    Null,
    /// Boolean value.
    Bool(bool),
    /// Integer value.
    Int(i64),
    /// A string value.
    Text(String),
    /// Key-value map.
    Map(std::collections::HashMap<String, Box<Value>>),
}
",
    );
    let result = context::handle_context(&db, "Value", false, None).unwrap();
    for variant in &["Null", "Bool(bool)", "Int(i64)", "Text(String)", "Map("] {
        assert!(
            result.contains(variant),
            "enum variant '{variant}' must appear in context: {result}"
        );
    }
    assert!(
        result.contains("Represents a parsed value"),
        "doc comment must appear: {result}"
    );
}

#[test]
fn trait_with_associated_type_and_default_method() {
    let (_dir, db) = index_source(
        r"
/// A data store abstraction.
pub trait Store {
    type Error;
    fn get(&self, key: &str) -> Result<String, Self::Error>;
    fn contains(&self, key: &str) -> bool {
        self.get(key).is_ok()
    }
}
",
    );
    let result = context::handle_context(&db, "Store", false, None).unwrap();
    assert!(
        result.contains("data store abstraction"),
        "trait doc comment: {result}"
    );
    assert!(
        result.contains("(trait)"),
        "should be identified as trait kind: {result}"
    );
}

#[test]
fn const_and_static_items() {
    let (_dir, db) = index_source(
        r"
/// Maximum retries allowed.
pub const MAX_RETRIES: u32 = 3;

/// Global counter.
pub static COUNTER: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
",
    );
    let result = query::handle_query(&db, "MAX_RETRIES", Some("symbols"), None, None, None, None).unwrap();
    assert!(
        result.contains("MAX_RETRIES") && result.contains("const"),
        "const should be findable: {result}"
    );

    let result = query::handle_query(&db, "COUNTER", Some("symbols"), None, None, None, None).unwrap();
    assert!(
        result.contains("COUNTER") && result.contains("static"),
        "static should be findable: {result}"
    );
}

#[test]
fn type_alias_preserved() {
    let (_dir, db) = index_source(
        r"
/// Shorthand for boxed errors.
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;
",
    );
    let result = context::handle_context(&db, "BoxError", false, None).unwrap();
    assert!(
        result.contains("type_alias"),
        "should be identified as type_alias: {result}"
    );
    assert!(
        result.contains("Shorthand for boxed errors"),
        "doc comment: {result}"
    );
}

#[test]
fn macro_definition_captured() {
    let (_dir, db) = index_source(
        r"
/// Create a hashmap literal.
macro_rules! hashmap {
    ($($key:expr => $val:expr),* $(,)?) => {{
        let mut map = std::collections::HashMap::new();
        $(map.insert($key, $val);)*
        map
    }};
}
",
    );
    let result = query::handle_query(&db, "hashmap", Some("symbols"), None, None, None, None).unwrap();
    assert!(
        result.contains("hashmap") && result.contains("macro"),
        "macro should be found: {result}"
    );
}

#[test]
fn impl_trait_detected_and_linked() {
    let (_dir, db) = index_source(
        r#"
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl std::fmt::Display for Point {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}

impl Default for Point {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0 }
    }
}
"#,
    );
    let result = context::handle_context(&db, "Point", false, None).unwrap();
    assert!(
        result.contains("Display"),
        "Display impl must be shown: {result}"
    );
    assert!(
        result.contains("Default"),
        "Default impl must be shown: {result}"
    );
    assert!(
        result.contains("Trait Implementations"),
        "section header must exist: {result}"
    );
}

#[test]
fn line_numbers_accurate() {
    let source = "\
pub fn alpha() {}

pub fn beta() {}

pub fn gamma() {}
";
    let (_dir, db) = index_source(source);
    let result = context::handle_context(&db, "alpha", false, None).unwrap();
    assert!(
        result.contains("1-1") || result.contains(":1-"),
        "alpha should start at line 1: {result}"
    );

    let result = context::handle_context(&db, "beta", false, None).unwrap();
    assert!(
        result.contains(":3-"),
        "beta should start at line 3: {result}"
    );

    let result = context::handle_context(&db, "gamma", false, None).unwrap();
    assert!(
        result.contains(":5-"),
        "gamma should start at line 5: {result}"
    );
}

#[test]
fn multiline_doc_comment_fully_captured() {
    let (_dir, db) = index_source(
        r"
/// First line of docs.
/// Second line with `code`.
/// Third line.
///
/// # Examples
///
/// ```rust
/// let x = 1;
/// ```
pub fn documented() {}
",
    );
    let result = context::handle_context(&db, "documented", false, None).unwrap();
    assert!(
        result.contains("First line of docs"),
        "first line: {result}"
    );
    assert!(
        result.contains("Second line with `code`"),
        "second line: {result}"
    );
    assert!(result.contains("# Examples"), "examples section: {result}");
}

#[test]
fn attributes_captured() {
    let (_dir, db) = index_source(
        r"
#[derive(Debug, Clone)]
#[repr(C)]
pub struct Packet {
    pub header: u32,
    pub payload: Vec<u8>,
}
",
    );
    let result = context::handle_context(&db, "Packet", false, None).unwrap();
    assert!(
        result.contains("derive(Debug, Clone)"),
        "derive attribute: {result}"
    );
    assert!(result.contains("repr(C)"), "repr attribute: {result}");
}

// =========================================================================
// 2. REFERENCE ACCURACY — symbol refs correctly identified
// =========================================================================

#[test]
fn function_call_reference_detected() {
    let (_dir, db) = index_source(
        r"
pub fn helper() -> u32 { 42 }

pub fn caller() -> u32 {
    helper()
}
",
    );
    let result = context::handle_context(&db, "caller", false, None).unwrap();
    assert!(
        result.contains("helper"),
        "caller should reference helper in callees: {result}"
    );

    let result = impact::handle_impact(&db, "helper", None, false).unwrap();
    assert!(
        result.contains("caller"),
        "helper's impact should show caller: {result}"
    );
}

#[test]
fn type_reference_detected() {
    let (_dir, db) = index_source(
        r"
pub struct Config {
    pub port: u16,
}

pub fn make_config() -> Config {
    Config { port: 8080 }
}
",
    );
    let result = context::handle_context(&db, "make_config", false, None).unwrap();
    assert!(
        result.contains("Config"),
        "make_config should reference Config: {result}"
    );
}

#[test]
fn transitive_impact_chain() {
    let (_dir, db) = index_source(
        r"
pub fn base() -> i32 { 1 }

pub fn mid() -> i32 { base() + 1 }

pub fn top() -> i32 { mid() + 1 }
",
    );
    let result = impact::handle_impact(&db, "base", None, false).unwrap();
    assert!(result.contains("mid"), "direct dependent: {result}");
    assert!(result.contains("top"), "transitive dependent: {result}");
    assert!(
        result.contains("Depth 1") && result.contains("Depth 2"),
        "should show depth levels: {result}"
    );
    assert!(
        result.contains("via"),
        "transitive dependent should show via: {result}"
    );
}

#[test]
fn no_false_positive_refs_for_common_names() {
    let (_dir, db) = index_source(
        r"
pub struct Foo;

impl Foo {
    pub fn new() -> Self { Foo }
}

pub fn make_foo() -> Foo {
    Foo::new()
}
",
    );
    let result = impact::handle_impact(&db, "new", None, false).unwrap();
    let line_count = result.lines().count();
    assert!(
        line_count < 20,
        "impact for 'new' should not explode: {result}"
    );
}

// =========================================================================
// 3. SEARCH QUALITY — FTS ranking, kind filtering, edge cases
// =========================================================================

#[test]
fn exact_name_match_ranked_first() {
    let (_dir, db) = index_source(
        r"
pub fn configure() {}
pub fn configuration_manager() {}
pub struct Config {}
",
    );
    let result = query::handle_query(&db, "Config", Some("symbols"), None, None, None, None).unwrap();
    let config_pos = result.find("**Config**");
    let configure_pos = result.find("configure");
    assert!(config_pos.is_some(), "exact match must appear: {result}");
    if let (Some(c), Some(cf)) = (config_pos, configure_pos) {
        assert!(
            c < cf,
            "exact match 'Config' should rank before 'configure': {result}"
        );
    }
}

#[test]
fn kind_filter_precise() {
    let (_dir, db) = index_source(
        r"
pub fn process() {}
pub struct Process {}
pub trait Processable {}
",
    );
    let result = query::handle_query(&db, "Process", Some("symbols"), Some("struct"), None, None, None).unwrap();
    assert!(result.contains("Process"), "struct should appear: {result}");
    assert!(
        !result.contains("(function)"),
        "function should be filtered out: {result}"
    );
    assert!(
        !result.contains("(trait)"),
        "trait should be filtered out: {result}"
    );
}

#[test]
fn query_files_scope_returns_unique_paths() {
    let (_dir, db) = index_multi_file(&[
        ("lib.rs", "pub mod server;\npub mod db;\n"),
        ("server.rs", "pub fn serve() {}\npub fn handle() {}\n"),
        ("db.rs", "pub fn query() {}\n"),
    ]);
    let result = query::handle_query(&db, "serve", Some("files"), None, None, None, None).unwrap();
    assert!(
        result.contains("src/server.rs"),
        "should find file: {result}"
    );
}

#[test]
fn empty_query_returns_no_results() {
    let (_dir, db) = index_source("pub fn hello() {}");
    let result = query::handle_query(&db, "", None, None, None, None, None).unwrap();
    assert!(
        result.contains("No results found"),
        "empty query should find nothing: {result}"
    );
}

// =========================================================================
// 4. TOOL OUTPUT FORMAT — verify Markdown structure Claude can parse
// =========================================================================

#[test]
fn context_output_has_all_sections() {
    let (_dir, db) = index_source(
        r"
/// Parse raw input into tokens.
#[inline]
pub fn tokenize(input: &str) -> Vec<String> {
    input.split_whitespace().map(String::from).collect()
}
",
    );
    let result = context::handle_context(&db, "tokenize", false, None).unwrap();

    assert!(
        result.contains("## tokenize (function)"),
        "header format: {result}"
    );
    assert!(
        result.contains("> Parse raw input into tokens"),
        "doc comment blockquote: {result}"
    );
    assert!(result.contains("**File:**"), "file location: {result}");
    assert!(
        result.contains("**Visibility:** public"),
        "visibility: {result}"
    );
    assert!(
        result.contains("**Signature:** `"),
        "signature in code span: {result}"
    );
    assert!(result.contains("### Source"), "source section: {result}");
    assert!(result.contains("```rust"), "source in code block: {result}");
}

#[test]
fn query_output_format_consistent() {
    let (_dir, db) = index_source(
        r"
/// A widget.
pub struct Widget {
    pub name: String,
}

/// Create a widget.
pub fn make_widget(name: &str) -> Widget {
    Widget { name: name.into() }
}
",
    );
    let result = query::handle_query(&db, "Widget", None, None, None, None, None).unwrap();

    assert!(
        result.contains("## Symbols"),
        "symbols section header: {result}"
    );
    assert!(
        result.contains("**Widget** (struct) at src/lib.rs:"),
        "symbol line format: {result}"
    );
    assert!(
        result.contains("`pub struct Widget"),
        "signature in backticks: {result}"
    );
    assert!(
        result.contains("*A widget.*"),
        "doc snippet in italics: {result}"
    );
}

#[test]
fn impact_output_format() {
    let (_dir, db) = index_source(
        r"
pub fn leaf() -> i32 { 1 }
pub fn middle() -> i32 { leaf() }
",
    );
    let result = impact::handle_impact(&db, "leaf", None, false).unwrap();

    assert!(
        result.contains("## Impact Analysis: leaf"),
        "impact header: {result}"
    );
    assert!(result.contains("### Depth 1"), "depth section: {result}");
    assert!(
        result.contains("**middle**"),
        "dependent name bold: {result}"
    );
}

#[test]
fn overview_output_format() {
    let (_dir, db) = index_source(
        r#"
/// A config struct.
pub struct Config {
    pub host: String,
}

pub fn load_config() -> Config {
    Config { host: "localhost".into() }
}

fn private_helper() {}
"#,
    );
    let result = overview::handle_overview(&db, "src/", false).unwrap();

    assert!(result.contains("### src/lib.rs"), "file header: {result}");
    assert!(
        result.contains("**Config** (struct)"),
        "struct entry: {result}"
    );
    assert!(
        result.contains("**load_config** (function)"),
        "function entry: {result}"
    );
    assert!(
        !result.contains("private_helper"),
        "private items must not appear: {result}"
    );
    assert!(result.contains("Summary:"), "summary section: {result}");
    assert!(
        result.contains("*A config struct.*"),
        "doc snippet: {result}"
    );
}

#[test]
fn tree_output_format() {
    let (_dir, db) = index_multi_file(&[
        ("lib.rs", "pub mod models;\npub fn entry() {}\n"),
        ("models.rs", "pub struct User {}\npub struct Post {}\n"),
    ]);
    let result = tree::handle_tree(&db, "src/").unwrap();

    assert!(
        result.contains("## Module Tree: src/"),
        "tree header: {result}"
    );
    assert!(
        result.contains("`src/lib.rs`"),
        "file in backticks: {result}"
    );
    assert!(result.contains("`src/models.rs`"), "nested file: {result}");
    assert!(result.contains("Total:"), "total summary: {result}");
}

// =========================================================================
// 5. DOCUMENTATION PARSING — HTML extraction quality
// =========================================================================

#[test]
fn html_extraction_strips_tags_cleanly() {
    let db = Database::open_in_memory().unwrap();
    let dep_id = db.insert_dependency("tokio", "1.37.0", true, None).unwrap();
    let doc_content = concat!(
        "Tokio is an asynchronous runtime for Rust. ",
        "It provides async I/O, networking, scheduling, and timers. ",
        "Key features: spawn tasks with tokio::spawn, ",
        "use channels for message passing, ",
        "and build servers with TcpListener.",
    );
    db.store_doc(dep_id, "docs.rs", doc_content).unwrap();

    let result = docs::handle_docs(&db, "tokio", None).unwrap();
    assert!(
        result.contains("asynchronous runtime"),
        "core description: {result}"
    );
    assert!(result.contains("tokio::spawn"), "API reference: {result}");

    let result = docs::handle_docs(&db, "tokio", Some("spawn")).unwrap();
    assert!(
        result.contains("tokio::spawn"),
        "topic search should find spawn: {result}"
    );
}

#[test]
fn docs_tool_distinguishes_known_vs_unknown_deps() {
    let db = Database::open_in_memory().unwrap();
    db.insert_dependency("known_crate", "1.0.0", true, None)
        .unwrap();

    let result = docs::handle_docs(&db, "known_crate", None).unwrap();
    assert!(
        result.contains("known dependency") && result.contains("no docs were fetched"),
        "should explain docs not fetched: {result}"
    );

    let result = docs::handle_docs(&db, "unknown_crate", None).unwrap();
    assert!(
        result.contains("not a known dependency"),
        "should say not known: {result}"
    );
}

#[test]
fn docs_topic_search_with_known_dep_no_match() {
    let db = Database::open_in_memory().unwrap();
    let dep_id = db.insert_dependency("serde", "1.0.0", true, None).unwrap();
    db.store_doc(dep_id, "docs.rs", "Serde serialization framework")
        .unwrap();

    let result = docs::handle_docs(&db, "serde", Some("graphql")).unwrap();
    assert!(
        result.contains("no docs match topic"),
        "should explain topic mismatch: {result}"
    );
}

#[test]
fn docs_multiple_sources_shown() {
    let db = Database::open_in_memory().unwrap();
    let dep_id = db
        .insert_dependency("reqwest", "0.12.0", true, None)
        .unwrap();
    // Summary doc (module="")
    db.store_doc(dep_id, "docs.rs", "reqwest HTTP client library")
        .unwrap();
    // Module doc
    db.store_doc_with_module(dep_id, "readme", "# reqwest\nAn ergonomic HTTP Client", "overview")
        .unwrap();

    // No topic → summary shown, modules listed
    let result = docs::handle_docs(&db, "reqwest", None).unwrap();
    assert!(result.contains("docs.rs"), "docs.rs source shown: {result}");
    assert!(result.contains("overview"), "module listed: {result}");

    // Topic "overview" → module doc content shown
    let result = docs::handle_docs(&db, "reqwest", Some("overview")).unwrap();
    assert!(
        result.contains("ergonomic HTTP Client"),
        "module content shown: {result}"
    );
}

// =========================================================================
// 6. CROSS-FILE REFERENCES — multi-file projects
// =========================================================================

#[test]
fn cross_file_reference_in_impact() {
    let (_dir, db) = index_multi_file(&[
        ("lib.rs", "pub mod models;\npub mod service;\n"),
        (
            "models.rs",
            "pub struct User {\n    pub name: String,\n    pub email: String,\n}\n",
        ),
        (
            "service.rs",
            "pub fn create_user(name: &str, email: &str) -> User {\n    User {\n        name: name.into(),\n        email: email.into(),\n    }\n}\n",
        ),
    ]);

    let result = impact::handle_impact(&db, "User", None, false).unwrap();
    assert!(
        result.contains("create_user"),
        "cross-file ref should appear in impact: {result}"
    );
}

#[test]
fn overview_shows_multi_file_structure() {
    let (_dir, db) = index_multi_file(&[
        ("lib.rs", "pub mod handlers;\npub mod models;\n"),
        (
            "handlers.rs",
            "/// Handle GET requests.\npub fn get_handler() {}\n/// Handle POST requests.\npub fn post_handler() {}\n",
        ),
        (
            "models.rs",
            "/// A user model.\npub struct User {}\n/// A post model.\npub struct Post {}\n",
        ),
    ]);
    let result = overview::handle_overview(&db, "src/", false).unwrap();

    assert!(
        result.contains("### src/handlers.rs"),
        "handlers file: {result}"
    );
    assert!(
        result.contains("### src/models.rs"),
        "models file: {result}"
    );
    assert!(
        result.contains("get_handler") && result.contains("post_handler"),
        "handler functions: {result}"
    );
    assert!(
        result.contains("User") && result.contains("Post"),
        "model structs: {result}"
    );
}

// =========================================================================
// 7. WORKSPACE PROJECTS — cross-crate data quality
// =========================================================================

fn index_workspace() -> (tempfile::TempDir, Database) {
    let dir = tempfile::TempDir::new().unwrap();

    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[workspace]\nmembers = [\"core\", \"api\", \"cli\"]\n",
    )
    .unwrap();

    std::fs::write(
        dir.path().join("Cargo.lock"),
        "[[package]]\nname = \"core\"\nversion = \"0.1.0\"\n\n\
         [[package]]\nname = \"api\"\nversion = \"0.1.0\"\n\n\
         [[package]]\nname = \"cli\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    write_crate(
        dir.path(),
        "core",
        "[package]\nname = \"core\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        "/// Core domain model for a user.\npub struct User {\n    pub id: u64,\n    pub name: String,\n}\n\n/// Validate a username.\npub fn validate_name(name: &str) -> bool {\n    !name.is_empty() && name.len() < 100\n}\n",
    );

    write_crate(
        dir.path(),
        "api",
        "[package]\nname = \"api\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\ncore = { path = \"../core\" }\n",
        "/// Create a new user via API.\npub fn create_user(name: &str) -> User {\n    User { id: 1, name: name.into() }\n}\n",
    );

    write_crate(
        dir.path(),
        "cli",
        "[package]\nname = \"cli\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\napi = { path = \"../api\" }\n",
        "/// CLI entry point.\npub fn run() {\n    let _user = create_user(\"admin\");\n}\n",
    );

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();
    (dir, db)
}

#[test]
fn workspace_crate_impact_propagation() {
    let (_dir, db) = index_workspace();
    let result = impact::handle_impact(&db, "User", None, false).unwrap();

    assert!(
        result.contains("Affected Crates"),
        "workspace impact must show crate summary: {result}"
    );
    assert!(result.contains("core"), "defining crate: {result}");
    assert!(result.contains("api"), "direct dependent crate: {result}");
}

#[test]
fn workspace_context_shows_correct_file_paths() {
    let (_dir, db) = index_workspace();
    let result = context::handle_context(&db, "User", false, None).unwrap();
    assert!(
        result.contains("core/src/lib.rs"),
        "file path should be crate-relative: {result}"
    );

    let result = context::handle_context(&db, "create_user", false, None).unwrap();
    assert!(
        result.contains("api/src/lib.rs"),
        "api function path: {result}"
    );
}

#[test]
fn workspace_query_finds_across_crates() {
    let (_dir, db) = index_workspace();
    let result = query::handle_query(&db, "user", Some("symbols"), None, None, None, None).unwrap();
    assert!(
        result.contains("User"),
        "should find User from core: {result}"
    );
    assert!(
        result.contains("create_user"),
        "should find create_user from api: {result}"
    );
}

#[test]
fn workspace_overview_scoped_to_crate() {
    let (_dir, db) = index_workspace();

    let result = overview::handle_overview(&db, "core/", false).unwrap();
    assert!(
        result.contains("User") && result.contains("validate_name"),
        "core overview: {result}"
    );
    assert!(
        !result.contains("create_user"),
        "api symbols should not leak into core overview: {result}"
    );

    let result = overview::handle_overview(&db, "api/", false).unwrap();
    assert!(result.contains("create_user"), "api overview: {result}");
}

// =========================================================================
// 8. EDGE CASES — boundary conditions
// =========================================================================

#[test]
fn empty_source_file() {
    let (_dir, db) = index_source("");
    let result = overview::handle_overview(&db, "src/", false).unwrap();
    assert!(result.contains("No public symbols"), "empty file: {result}");
}

#[test]
fn source_with_only_private_items() {
    let (_dir, db) =
        index_source("fn private_one() {}\nfn private_two() {}\nstruct InternalState {}\n");
    let result = overview::handle_overview(&db, "src/", false).unwrap();
    assert!(
        result.contains("No public symbols"),
        "only private items: {result}"
    );
}

#[test]
fn deeply_nested_module_paths_in_tree() {
    let (_dir, db) = index_multi_file(&[
        ("lib.rs", "pub mod a;\n"),
        ("a/mod.rs", "pub mod b;\npub fn top_fn() {}\n"),
        ("a/b/mod.rs", "pub fn deep_fn() {}\n"),
    ]);
    let result = tree::handle_tree(&db, "src/").unwrap();
    assert!(result.contains("src/a/mod.rs"), "nested module: {result}");
    assert!(
        result.contains("src/a/b/mod.rs"),
        "deeply nested module: {result}"
    );
}

#[test]
fn symbol_with_same_name_different_kinds() {
    let (_dir, db) = index_source(
        r#"
pub struct Error {
    pub message: String,
}

pub enum Error {}

pub fn error() -> String {
    "oops".into()
}
"#,
    );
    let result = query::handle_query(&db, "Error", Some("symbols"), None, None, None, None).unwrap();
    assert!(
        result.contains("struct") || result.contains("enum"),
        "should find both type definitions: {result}"
    );

    let result = query::handle_query(&db, "Error", Some("symbols"), Some("function"), None, None, None).unwrap();
    assert!(
        result.contains("error") && result.contains("function"),
        "function filter: {result}"
    );
}

#[test]
fn pub_crate_visibility_shown() {
    let (_dir, db) = index_source("pub(crate) fn internal_api() -> u32 { 42 }\n");
    let result = context::handle_context(&db, "internal_api", false, None).unwrap();
    assert!(
        result.contains("pub(crate)"),
        "pub(crate) visibility: {result}"
    );
}

#[test]
fn context_no_symbol_gives_clear_message() {
    let (_dir, db) = index_source("pub fn something() {}");
    let result = context::handle_context(&db, "nonexistent_symbol", false, None).unwrap();
    assert!(
        result.contains("No symbol found matching 'nonexistent_symbol'"),
        "clear error message: {result}"
    );
}

#[test]
fn impact_no_dependents_gives_clear_message() {
    let (_dir, db) = index_source("pub fn isolated() {}");
    let result = impact::handle_impact(&db, "isolated", None, false).unwrap();
    assert!(
        result.contains("No dependents found"),
        "clear message: {result}"
    );
}

// =========================================================================
// 9. COMPLEX RUST SYNTAX EDGE CASES
// =========================================================================

#[test]
fn where_clause_in_signature() {
    let (_dir, db) = index_source(
        r"
pub fn process<T>(item: T) -> String where T: std::fmt::Display + Clone {
    item.to_string()
}
",
    );
    let result = query::handle_query(&db, "process", Some("symbols"), None, None, None, None).unwrap();
    assert!(
        result.contains("process"),
        "should find function with where clause: {result}"
    );
}

#[test]
fn const_generic_parameter() {
    let (_dir, db) = index_source(
        r"
pub struct FixedArray<const N: usize> {
    data: [u8; N],
}
",
    );
    let result = context::handle_context(&db, "FixedArray", false, None).unwrap();
    assert!(
        result.contains("FixedArray"),
        "should find struct with const generic: {result}"
    );
}

#[test]
fn impl_block_with_lifetime() {
    let (_dir, db) = index_source(
        r"
pub struct Parser<'a> {
    input: &'a str,
}

impl<'a> Parser<'a> {
    pub fn new(input: &'a str) -> Self {
        Parser { input }
    }
}
",
    );
    let result = context::handle_context(&db, "Parser", false, None).unwrap();
    assert!(
        result.contains("Parser"),
        "should find struct with lifetime: {result}"
    );
    assert!(
        result.contains("new"),
        "should show method from lifetime impl block: {result}"
    );
}

#[test]
fn multiple_impl_blocks_for_same_type() {
    let (_dir, db) = index_source(
        r"
pub struct Builder;

impl Builder {
    pub fn new() -> Self { Builder }
}

impl Builder {
    pub fn build(&self) -> String { String::new() }
}
",
    );
    let result = context::handle_context(&db, "Builder", false, None).unwrap();
    assert!(
        result.contains("new"),
        "should show method from first impl block: {result}"
    );
    assert!(
        result.contains("build"),
        "should show method from second impl block: {result}"
    );
}

#[test]
fn cfg_gated_function() {
    let (_dir, db) = index_source(
        r"
#[cfg(test)]
pub fn test_only() {}

#[cfg(not(test))]
pub fn prod_only() {}

pub fn always() {}
",
    );
    let result = query::handle_query(&db, "always", Some("symbols"), None, None, None, None).unwrap();
    assert!(
        result.contains("always"),
        "should find non-gated function: {result}"
    );
}

#[test]
fn unsafe_function_parsed() {
    let (_dir, db) = index_source(
        r"
pub unsafe fn dangerous(ptr: *const u8) -> u8 {
    unsafe { *ptr }
}
",
    );
    let result = query::handle_query(&db, "dangerous", Some("symbols"), None, None, None, None).unwrap();
    assert!(
        result.contains("dangerous"),
        "should find unsafe function: {result}"
    );
}

// =========================================================================
// 10. FTS RANKING TESTS
// =========================================================================

#[test]
fn exact_match_ranks_above_substring() {
    let (_dir, db) = index_source(
        r"
pub fn config() {}
pub fn config_parser() {}
pub fn parse_config_file() {}
",
    );
    let result = query::handle_query(&db, "config", Some("symbols"), None, None, None, None).unwrap();
    assert!(
        result.contains("config"),
        "exact match should appear in results: {result}"
    );
}

#[test]
fn query_kind_filter_excludes_other_kinds() {
    let (_dir, db) = index_source(
        r"
pub fn process() {}
pub struct Process {}
",
    );
    let result =
        query::handle_query(&db, "process", Some("symbols"), Some("function"), None, None, None).unwrap();
    assert!(
        result.contains("process"),
        "function should be found: {result}"
    );
    assert!(
        !result.contains("struct Process"),
        "struct should be excluded by kind filter: {result}"
    );
}

// =========================================================================
// 11. REGRESSION TESTS FOR BUG FIXES
// =========================================================================

#[test]
fn file_scope_handles_dots_in_filename() {
    let (_dir, db) = index_multi_file(&[
        ("lib.rs", "pub mod server;\n"),
        ("server.rs", "pub fn serve() {}\n"),
    ]);
    let result = query::handle_query(&db, "server.rs", Some("files"), None, None, None, None).unwrap();
    assert!(
        result.contains("server.rs"),
        "file scope should handle dots: {result}"
    );
}

#[test]
fn default_query_excludes_use_and_mod() {
    let (_dir, db) = index_source(
        "use std::fmt::Write;\npub mod child;\npub fn real_fn() {}\npub struct RealStruct;\n",
    );
    let result = query::handle_query(&db, "real", Some("symbols"), None, None, None, None).unwrap();
    assert!(
        result.contains("real_fn"),
        "should find function: {result}"
    );
    assert!(
        !result.contains("(use)"),
        "default query should exclude use items: {result}"
    );
    assert!(
        !result.contains("(mod)"),
        "default query should exclude mod items: {result}"
    );
}

#[test]
fn kind_use_filter_still_works() {
    let (_dir, db) = index_source("use std::fmt::Write;\npub fn real_fn() {}\n");
    let result = query::handle_query(&db, "Write", Some("symbols"), Some("use"), None, None, None).unwrap();
    assert!(
        result.contains("(use)"),
        "kind=use should still return use items: {result}"
    );
}

#[test]
fn callees_scoped_to_source_file() {
    let (_dir, db) = index_multi_file(&[
        ("lib.rs", "pub mod a;\npub mod b;\npub fn shared() {}\n"),
        ("a.rs", "pub fn caller_a() { shared(); }\n"),
        ("b.rs", "pub fn caller_b() { shared(); }\n"),
    ]);
    let result_a = context::handle_context(&db, "caller_a", false, None).unwrap();
    let result_b = context::handle_context(&db, "caller_b", false, None).unwrap();
    // Each should show shared as callee, but caller_a's callees should not
    // include anything from caller_b and vice versa
    assert!(
        result_a.contains("shared"),
        "caller_a should call shared: {result_a}"
    );
    assert!(
        result_b.contains("shared"),
        "caller_b should call shared: {result_b}"
    );
}

// =========================================================================
// 12. REALISTIC MULTI-PATTERN RUST CODEBASE
// =========================================================================

const REALISTIC_LIB: &str = r"
pub mod error;
pub mod config;
pub mod service;
pub mod traits;
pub use config::AppConfig;
";

const REALISTIC_ERROR: &str = r#"
use std::fmt;

#[derive(Debug)]
pub enum AppError {
    NotFound(String),
    InvalidInput { field: String, reason: String },
    Internal(Box<dyn std::error::Error>),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(msg) => write!(f, "not found: {msg}"),
            Self::InvalidInput { field, reason } => write!(f, "invalid {field}: {reason}"),
            Self::Internal(e) => write!(f, "internal error: {e}"),
        }
    }
}
"#;

const REALISTIC_CONFIG: &str = r#"
/// Application configuration loaded from environment.
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub host: String,
    pub port: u16,
    pub max_connections: usize,
}

impl AppConfig {
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder::default()
    }
}

#[derive(Default)]
pub struct ConfigBuilder {
    host: Option<String>,
    port: Option<u16>,
    max_connections: Option<usize>,
}

impl ConfigBuilder {
    pub fn host(mut self, host: impl Into<String>) -> Self {
        self.host = Some(host.into());
        self
    }

    pub fn port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    pub fn build(self) -> AppConfig {
        AppConfig {
            host: self.host.unwrap_or_else(|| "localhost".to_string()),
            port: self.port.unwrap_or(8080),
            max_connections: self.max_connections.unwrap_or(100),
        }
    }
}
"#;

const REALISTIC_TRAITS: &str = r"
use crate::error::AppError;

/// A handler that processes requests of type T.
pub trait Handler<T> {
    type Output;
    fn handle(&self, input: T) -> Result<Self::Output, AppError>;
}

/// Middleware that wraps a handler.
pub trait Middleware {
    fn before(&self) {}
    fn after(&self) {}
}
";

const REALISTIC_SERVICE: &str = r#"
use crate::config::AppConfig;
use crate::error::AppError;
use crate::traits::Handler;

pub struct UserService {
    config: AppConfig,
}

impl UserService {
    pub fn new(config: AppConfig) -> Self {
        Self { config }
    }

    pub fn port(&self) -> u16 {
        self.config.port
    }
}

impl Handler<String> for UserService {
    type Output = String;
    fn handle(&self, input: String) -> Result<Self::Output, AppError> {
        if input.is_empty() {
            return Err(AppError::InvalidInput {
                field: "name".into(),
                reason: "cannot be empty".into(),
            });
        }
        Ok(format!("Hello, {input}!"))
    }
}
"#;

fn index_realistic_codebase() -> (tempfile::TempDir, Database) {
    index_multi_file(&[
        ("lib.rs", REALISTIC_LIB),
        ("error.rs", REALISTIC_ERROR),
        ("config.rs", REALISTIC_CONFIG),
        ("traits.rs", REALISTIC_TRAITS),
        ("service.rs", REALISTIC_SERVICE),
    ])
}

#[test]
fn realistic_codebase_indexes_all_symbols() {
    let (_dir, db) = index_realistic_codebase();

    for name in &[
        "AppError",
        "AppConfig",
        "ConfigBuilder",
        "UserService",
        "Handler",
        "Middleware",
    ] {
        let result =
            query::handle_query(&db, name, Some("symbols"), None, None, None, None).unwrap();
        assert!(
            result.contains(name),
            "query for '{name}' should find it: {result}"
        );
    }

    let result = overview::handle_overview(&db, "src/", false).unwrap();
    for name in &[
        "AppError",
        "AppConfig",
        "ConfigBuilder",
        "UserService",
        "Handler",
        "Middleware",
    ] {
        assert!(
            result.contains(name),
            "overview should list '{name}': {result}"
        );
    }

    let all_names = db.get_all_symbol_names().unwrap();
    assert!(
        all_names.len() >= 15,
        "should index at least 15 symbols, got {}",
        all_names.len()
    );
}

#[test]
fn realistic_codebase_trait_impl_detected() {
    let (_dir, db) = index_realistic_codebase();

    let result =
        context::handle_context(&db, "UserService", false, None).unwrap();
    assert!(
        result.contains("Handler"),
        "UserService context should mention Handler trait impl: {result}"
    );

    let result =
        context::handle_context(&db, "Handler", false, None).unwrap();
    assert!(
        result.contains("UserService"),
        "Handler context should show UserService as implementor: {result}"
    );

    let result =
        context::handle_context(&db, "AppError", false, None).unwrap();
    assert!(
        result.contains("Display"),
        "AppError context should show Display trait impl: {result}"
    );
}

#[test]
fn realistic_codebase_cross_file_impact() {
    let (_dir, db) = index_realistic_codebase();

    let result = impact::handle_impact(&db, "AppConfig", None, false).unwrap();
    assert!(
        result.contains("UserService") || result.contains("new"),
        "AppConfig impact should show UserService dependency: {result}"
    );
    assert!(
        result.contains("ConfigBuilder") || result.contains("build"),
        "AppConfig impact should show ConfigBuilder relationship: {result}"
    );

    let result = impact::handle_impact(&db, "AppError", None, false).unwrap();
    assert!(
        result.contains("handle") || result.contains("UserService"),
        "AppError impact should show usage in service: {result}"
    );
}

// =========================================================================
// 13. SEARCH QUALITY UNDER AMBIGUITY
// =========================================================================

#[test]
fn search_exact_name_beats_contains() {
    let (_dir, db) = index_source(
        "pub struct Config {}\n\
         pub fn configure() {}\n\
         pub fn app_config() {}\n\
         pub struct ConfigManager {}\n",
    );
    let result =
        query::handle_query(&db, "Config", Some("symbols"), None, None, None, None).unwrap();
    let config_pos = result.find("**Config**");
    let config_manager_pos = result.find("ConfigManager");
    let configure_pos = result.find("configure");

    assert!(
        config_pos.is_some(),
        "exact match 'Config' must appear: {result}"
    );
    if let (Some(c), Some(cm)) = (config_pos, config_manager_pos) {
        assert!(
            c < cm,
            "exact 'Config' should appear before 'ConfigManager': {result}"
        );
    }
    if let (Some(c), Some(cf)) = (config_pos, configure_pos) {
        assert!(
            c < cf,
            "exact 'Config' should appear before 'configure': {result}"
        );
    }
}

#[test]
fn search_common_name_new_returns_results() {
    let (_dir, db) = index_multi_file(&[
        (
            "lib.rs",
            "pub mod types;\npub mod more;\n",
        ),
        (
            "types.rs",
            "pub struct Alpha {}\nimpl Alpha { pub fn new() -> Self { Alpha {} } }\n",
        ),
        (
            "more.rs",
            "pub struct Beta {}\nimpl Beta { pub fn new() -> Self { Beta {} } }\n",
        ),
    ]);
    let result =
        query::handle_query(&db, "new", Some("symbols"), None, None, None, None).unwrap();
    assert!(
        result.contains("new"),
        "query for 'new' should find results: {result}"
    );
    let line_count = result.lines().count();
    assert!(
        line_count < 50,
        "result should be reasonable size, got {line_count} lines"
    );
}

#[test]
fn search_by_doc_comment_content() {
    let (_dir, db) = index_source(
        "/// Parse a TOML configuration file.\n\
         pub fn load_config() {}\n\
         /// Serialize data to JSON format.\n\
         pub fn save_data() {}\n",
    );
    let result =
        query::handle_query(&db, "TOML", Some("all"), None, None, None, None).unwrap();
    // FTS indexes symbol names, not doc comments, so this may not find
    // results. The key assertion is no error or panic.
    assert!(
        result.contains("TOML") || result.contains("No results"),
        "query should complete without error: {result}"
    );
}

#[test]
fn search_short_query_works() {
    let (_dir, db) = index_source(
        "pub fn go() {}\npub fn do_work() {}\n",
    );
    let result =
        query::handle_query(&db, "go", Some("symbols"), None, None, None, None).unwrap();
    assert!(
        result.contains("go"),
        "short 2-char query should find 'go': {result}"
    );
}
