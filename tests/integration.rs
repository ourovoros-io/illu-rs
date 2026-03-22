#![expect(clippy::unwrap_used, reason = "integration tests")]

use illu_rs::db::Database;
use illu_rs::indexer::{IndexConfig, index_repo};
use illu_rs::server::tools::{QueryScope, context, docs, impact, overview, query};

fn setup_indexed_db() -> (tempfile::TempDir, Database) {
    let dir = tempfile::TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();

    std::fs::write(
        dir.path().join("Cargo.toml"),
        r#"
[package]
name = "sample"
version = "0.1.0"
edition = "2021"

[dependencies]
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
name = "sample"
version = "0.1.0"
dependencies = ["serde"]
"#,
    )
    .unwrap();

    std::fs::write(
        src_dir.join("lib.rs"),
        r#"
use serde::Serialize;

/// Application configuration.
/// Holds host and port settings.
pub struct Config {
    pub host: String,
    pub port: u16,
}

impl Config {
    /// Create a new Config with defaults.
    pub fn new(host: String, port: u16) -> Self {
        Self { host, port }
    }
}

/// Parse configuration from input string.
pub fn parse_config(input: &str) -> Config {
    let _ = input;
    Config::new("localhost".into(), 8080)
}

pub trait Configurable {
    fn configure(&self) -> Config;
}

pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error(String),
}

impl std::fmt::Display for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}
"#,
    )
    .unwrap();

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();

    let serde_id = db.dependency_id("serde").unwrap().unwrap();
    db.store_doc(
        serde_id,
        "docs.rs",
        "Serde is a framework for serializing and deserializing Rust data structures",
    )
    .unwrap();

    (dir, db)
}

#[test]
fn test_query_tool_symbols() {
    let (_dir, db) = setup_indexed_db();
    let result = query::handle_query(
        &db,
        "parse",
        Some(QueryScope::Symbols),
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    assert!(
        result.contains("parse_config"),
        "query should find parse_config"
    );
}

#[test]
fn test_query_tool_docs() {
    let (_dir, db) = setup_indexed_db();
    let result = query::handle_query(
        &db,
        "serializ",
        Some(QueryScope::Docs),
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    assert!(result.contains("Serde"), "query should find serde docs");
}

#[test]
fn test_query_tool_all() {
    let (_dir, db) = setup_indexed_db();
    let result = query::handle_query(&db, "Config", None, None, None, None, None, None).unwrap();
    assert!(result.contains("Config"), "query all should find Config");
}

#[test]
fn test_context_tool() {
    let (_dir, db) = setup_indexed_db();
    let result = context::handle_context(&db, "Config", false, None, None, None, false).unwrap();
    assert!(result.contains("Config"), "context should find Config");
    assert!(
        result.contains("src/lib.rs"),
        "context should include file path"
    );
}

#[test]
fn test_impact_tool() {
    let (_dir, db) = setup_indexed_db();
    let result = impact::handle_impact(&db, "Config", None, false, false).unwrap();
    assert!(
        result.contains("Impact Analysis"),
        "impact should show header"
    );
}

#[test]
fn test_docs_tool() {
    let (_dir, db) = setup_indexed_db();
    let result = docs::handle_docs(&db, "serde", None).unwrap();
    assert!(result.contains("serializ"), "docs should return serde docs");
}

#[test]
fn test_docs_tool_with_topic() {
    let (_dir, db) = setup_indexed_db();
    let result = docs::handle_docs(&db, "serde", Some("serializ")).unwrap();
    assert!(result.contains("Serde"), "docs with topic should match");
}

#[test]
fn test_docs_tool_unknown_dependency() {
    let (_dir, db) = setup_indexed_db();
    let result = docs::handle_docs(&db, "unknown_crate", None).unwrap();
    assert!(result.contains("not a known dependency"));
}

#[test]
fn test_skill_file_generated() {
    let (dir, _db) = setup_indexed_db();
    let skill_path = dir.path().join(".claude").join("skills").join("illu-rs.md");
    assert!(skill_path.exists(), "Claude skill file should be generated");
    let content = std::fs::read_to_string(&skill_path).unwrap();
    assert!(content.contains("serde"));
    assert!(content.contains("query"));
}

fn setup_workspace_db() -> (tempfile::TempDir, Database) {
    let dir = tempfile::TempDir::new().unwrap();

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
"#,
    )
    .unwrap();

    // shared crate
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
        r"
pub struct SharedConfig {
    pub host: String,
    pub port: u16,
}

pub fn default_config() -> SharedConfig {
    SharedConfig { host: String::new(), port: 8080 }
}
",
    )
    .unwrap();

    // app crate depending on shared
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
        r"
pub fn run() -> SharedConfig {
    default_config()
}
",
    )
    .unwrap();

    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();

    (dir, db)
}

#[test]
fn test_workspace_query_across_crates() {
    let (_dir, db) = setup_workspace_db();
    let result = query::handle_query(
        &db,
        "SharedConfig",
        Some(QueryScope::Symbols),
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    assert!(
        result.contains("SharedConfig"),
        "query should find SharedConfig from shared crate"
    );
}

#[test]
fn test_workspace_impact_crate_summary() {
    let (_dir, db) = setup_workspace_db();
    let result = impact::handle_impact(&db, "SharedConfig", None, false, false).unwrap();
    assert!(
        result.contains("Affected Crates"),
        "impact should show crate-level summary"
    );
    assert!(
        result.contains("shared"),
        "impact should show defining crate"
    );
    assert!(result.contains("app"), "impact should show dependent crate");
}

#[test]
fn test_workspace_context_shows_file_path() {
    let (_dir, db) = setup_workspace_db();
    let result =
        context::handle_context(&db, "SharedConfig", false, None, None, None, false).unwrap();
    assert!(
        result.contains("shared/src/lib.rs"),
        "context should show crate-relative path"
    );
}

#[test]
fn test_workspace_skill_file() {
    let (dir, _db) = setup_workspace_db();
    let skill_path = dir.path().join(".claude").join("skills").join("illu-rs.md");
    assert!(skill_path.exists(), "skill file should exist for workspace");
    let content = std::fs::read_to_string(&skill_path).unwrap();
    assert!(
        content.contains("serde"),
        "skill file should list serde dep"
    );
}

#[test]
fn test_context_tool_enriched() {
    let (_dir, db) = setup_indexed_db();
    let result = context::handle_context(&db, "Config", false, None, None, None, false).unwrap();
    assert!(
        result.contains("Application configuration"),
        "context should include doc comment"
    );
    assert!(
        result.contains("host: String"),
        "context should include struct fields"
    );
    assert!(
        result.contains("pub struct Config"),
        "context should include source body"
    );
}

#[test]
fn test_context_trait_impls() {
    let (_dir, db) = setup_indexed_db();
    let result = context::handle_context(&db, "Config", false, None, None, None, false).unwrap();
    assert!(
        result.contains("Display"),
        "context should show Display trait impl for Config"
    );
}

#[test]
fn test_context_callees() {
    let (_dir, db) = setup_indexed_db();
    let result =
        context::handle_context(&db, "parse_config", false, None, None, None, false).unwrap();
    assert!(
        result.contains("Config") || result.contains("new"),
        "parse_config should show callees"
    );
}

#[test]
fn test_query_doc_snippet() {
    let (_dir, db) = setup_indexed_db();
    let result = query::handle_query(
        &db,
        "parse_config",
        Some(QueryScope::Symbols),
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    assert!(
        result.contains("Parse configuration"),
        "query should show doc comment snippet"
    );
}

#[test]
fn test_overview_tool() {
    let (_dir, db) = setup_indexed_db();
    let result = overview::handle_overview(&db, "src/", false, None).unwrap();
    assert!(result.contains("Config"), "overview should list Config");
    assert!(
        result.contains("parse_config"),
        "overview should list parse_config"
    );
    assert!(
        result.contains("Configurable"),
        "overview should list Configurable trait"
    );
    assert!(
        result.contains("LogLevel"),
        "overview should list LogLevel enum"
    );
}

#[test]
fn test_enum_details_in_context() {
    let (_dir, db) = setup_indexed_db();
    let result = context::handle_context(&db, "LogLevel", false, None, None, None, false).unwrap();
    assert!(result.contains("Debug"), "should show enum variants");
    assert!(
        result.contains("Error(String)"),
        "should show tuple variant"
    );
}
