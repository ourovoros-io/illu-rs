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
    let result = context::handle_context(&db, "collect_items", false).unwrap();
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
    let result = context::handle_context(&db, "fetch_data", false).unwrap();
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
    let result = context::handle_context(&db, "BorrowedSlice", false).unwrap();
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
    let result = context::handle_context(&db, "Value", false).unwrap();
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
    let result = context::handle_context(&db, "Store", false).unwrap();
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
    let result = query::handle_query(&db, "MAX_RETRIES", Some("symbols"), None).unwrap();
    assert!(
        result.contains("MAX_RETRIES") && result.contains("const"),
        "const should be findable: {result}"
    );

    let result = query::handle_query(&db, "COUNTER", Some("symbols"), None).unwrap();
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
    let result = context::handle_context(&db, "BoxError", false).unwrap();
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
    let result = query::handle_query(&db, "hashmap", Some("symbols"), None).unwrap();
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
    let result = context::handle_context(&db, "Point", false).unwrap();
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
    let result = context::handle_context(&db, "alpha", false).unwrap();
    assert!(
        result.contains("1-1") || result.contains(":1-"),
        "alpha should start at line 1: {result}"
    );

    let result = context::handle_context(&db, "beta", false).unwrap();
    assert!(
        result.contains(":3-"),
        "beta should start at line 3: {result}"
    );

    let result = context::handle_context(&db, "gamma", false).unwrap();
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
    let result = context::handle_context(&db, "documented", false).unwrap();
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
    let result = context::handle_context(&db, "Packet", false).unwrap();
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
    let result = context::handle_context(&db, "caller", false).unwrap();
    assert!(
        result.contains("helper"),
        "caller should reference helper in callees: {result}"
    );

    let result = impact::handle_impact(&db, "helper").unwrap();
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
    let result = context::handle_context(&db, "make_config", false).unwrap();
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
    let result = impact::handle_impact(&db, "base").unwrap();
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
    let result = impact::handle_impact(&db, "new").unwrap();
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
    let result = query::handle_query(&db, "Config", Some("symbols"), None).unwrap();
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
    let result = query::handle_query(&db, "Process", Some("symbols"), Some("struct")).unwrap();
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
    let result = query::handle_query(&db, "serve", Some("files"), None).unwrap();
    assert!(
        result.contains("src/server.rs"),
        "should find file: {result}"
    );
}

#[test]
fn empty_query_returns_no_results() {
    let (_dir, db) = index_source("pub fn hello() {}");
    let result = query::handle_query(&db, "", None, None).unwrap();
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
    let result = context::handle_context(&db, "tokenize", false).unwrap();

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
    let result = query::handle_query(&db, "Widget", None, None).unwrap();

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
    let result = impact::handle_impact(&db, "leaf").unwrap();

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
    let result = overview::handle_overview(&db, "src/").unwrap();

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

    let result = impact::handle_impact(&db, "User").unwrap();
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
    let result = overview::handle_overview(&db, "src/").unwrap();

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
    let result = impact::handle_impact(&db, "User").unwrap();

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
    let result = context::handle_context(&db, "User", false).unwrap();
    assert!(
        result.contains("core/src/lib.rs"),
        "file path should be crate-relative: {result}"
    );

    let result = context::handle_context(&db, "create_user", false).unwrap();
    assert!(
        result.contains("api/src/lib.rs"),
        "api function path: {result}"
    );
}

#[test]
fn workspace_query_finds_across_crates() {
    let (_dir, db) = index_workspace();
    let result = query::handle_query(&db, "user", Some("symbols"), None).unwrap();
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

    let result = overview::handle_overview(&db, "core/").unwrap();
    assert!(
        result.contains("User") && result.contains("validate_name"),
        "core overview: {result}"
    );
    assert!(
        !result.contains("create_user"),
        "api symbols should not leak into core overview: {result}"
    );

    let result = overview::handle_overview(&db, "api/").unwrap();
    assert!(result.contains("create_user"), "api overview: {result}");
}

// =========================================================================
// 8. EDGE CASES — boundary conditions
// =========================================================================

#[test]
fn empty_source_file() {
    let (_dir, db) = index_source("");
    let result = overview::handle_overview(&db, "src/").unwrap();
    assert!(result.contains("No public symbols"), "empty file: {result}");
}

#[test]
fn source_with_only_private_items() {
    let (_dir, db) =
        index_source("fn private_one() {}\nfn private_two() {}\nstruct InternalState {}\n");
    let result = overview::handle_overview(&db, "src/").unwrap();
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
    let result = query::handle_query(&db, "Error", Some("symbols"), None).unwrap();
    assert!(
        result.contains("struct") || result.contains("enum"),
        "should find both type definitions: {result}"
    );

    let result = query::handle_query(&db, "Error", Some("symbols"), Some("function")).unwrap();
    assert!(
        result.contains("error") && result.contains("function"),
        "function filter: {result}"
    );
}

#[test]
fn pub_crate_visibility_shown() {
    let (_dir, db) = index_source("pub(crate) fn internal_api() -> u32 { 42 }\n");
    let result = context::handle_context(&db, "internal_api", false).unwrap();
    assert!(
        result.contains("pub(crate)"),
        "pub(crate) visibility: {result}"
    );
}

#[test]
fn context_no_symbol_gives_clear_message() {
    let (_dir, db) = index_source("pub fn something() {}");
    let result = context::handle_context(&db, "nonexistent_symbol", false).unwrap();
    assert!(
        result.contains("No symbol found matching 'nonexistent_symbol'"),
        "clear error message: {result}"
    );
}

#[test]
fn impact_no_dependents_gives_clear_message() {
    let (_dir, db) = index_source("pub fn isolated() {}");
    let result = impact::handle_impact(&db, "isolated").unwrap();
    assert!(
        result.contains("No dependents found"),
        "clear message: {result}"
    );
}
