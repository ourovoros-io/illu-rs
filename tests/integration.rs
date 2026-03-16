#![expect(clippy::unwrap_used, reason = "integration tests")]

use illu_rs::db::Database;
use illu_rs::indexer::{IndexConfig, index_repo};
use illu_rs::server::tools::{context, docs, impact, query};

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

pub struct Config {
    pub host: String,
    pub port: u16,
}

impl Config {
    pub fn new(host: String, port: u16) -> Self {
        Self { host, port }
    }
}

pub fn parse_config(input: &str) -> Config {
    let _ = input;
    Config::new("localhost".into(), 8080)
}

pub trait Configurable {
    fn configure(&self) -> Config;
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

    let serde_id = db.get_dependency_id("serde").unwrap().unwrap();
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
    let result = query::handle_query(&db, "parse", Some("symbols")).unwrap();
    assert!(
        result.contains("parse_config"),
        "query should find parse_config"
    );
}

#[test]
fn test_query_tool_docs() {
    let (_dir, db) = setup_indexed_db();
    let result = query::handle_query(&db, "serializ", Some("docs")).unwrap();
    assert!(result.contains("Serde"), "query should find serde docs");
}

#[test]
fn test_query_tool_all() {
    let (_dir, db) = setup_indexed_db();
    let result = query::handle_query(&db, "Config", None).unwrap();
    assert!(result.contains("Config"), "query all should find Config");
}

#[test]
fn test_context_tool() {
    let (_dir, db) = setup_indexed_db();
    let result = context::handle_context(&db, "Config").unwrap();
    assert!(result.contains("Config"), "context should find Config");
    assert!(
        result.contains("src/lib.rs"),
        "context should include file path"
    );
}

#[test]
fn test_impact_tool() {
    let (_dir, db) = setup_indexed_db();
    let result = impact::handle_impact(&db, "Config").unwrap();
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
    assert!(result.contains("No documentation found"));
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
