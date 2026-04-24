#![expect(clippy::unwrap_used, reason = "integration tests")]

use illu_rs::api::db::Database;
use illu_rs::api::indexer::{IndexConfig, index_repo};
use illu_rs::api::registry::{Registry, RepoEntry};
use illu_rs::api::server::tools::{cross_deps, cross_impact, cross_query, repos};

/// Create two indexed repos on disk with `.illu/index.db` files.
///
/// Repo A: `shared_helper` function + `SharedType` struct.
/// Repo B: `caller` function + its own `shared_helper`.
///
/// Returns `(TempDir, TempDir, Database, Database)` — temp dirs kept
/// alive so paths remain valid.
fn setup_two_repos() -> (tempfile::TempDir, tempfile::TempDir, Database, Database) {
    // --- Repo A ---
    let dir_a = tempfile::TempDir::new().unwrap();
    let src_a = dir_a.path().join("src");
    std::fs::create_dir_all(&src_a).unwrap();

    std::fs::write(
        dir_a.path().join("Cargo.toml"),
        r#"
[package]
name = "repo-a"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    std::fs::write(
        dir_a.path().join("Cargo.lock"),
        r#"
[[package]]
name = "repo-a"
version = "0.1.0"
"#,
    )
    .unwrap();

    std::fs::write(
        src_a.join("lib.rs"),
        r"
pub fn shared_helper(x: i32) -> i32 {
    x + 1
}

pub struct SharedType {
    pub value: i32,
}
",
    )
    .unwrap();

    let illu_a = dir_a.path().join(".illu");
    std::fs::create_dir_all(&illu_a).unwrap();
    let db_a = Database::open(&illu_a.join("index.db")).unwrap();
    let config_a = IndexConfig {
        repo_path: dir_a.path().to_path_buf(),
    };
    index_repo(&db_a, &config_a).unwrap();

    // --- Repo B ---
    let dir_b = tempfile::TempDir::new().unwrap();
    let src_b = dir_b.path().join("src");
    std::fs::create_dir_all(&src_b).unwrap();

    std::fs::write(
        dir_b.path().join("Cargo.toml"),
        r#"
[package]
name = "repo-b"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    std::fs::write(
        dir_b.path().join("Cargo.lock"),
        r#"
[[package]]
name = "repo-b"
version = "0.1.0"
"#,
    )
    .unwrap();

    std::fs::write(
        src_b.join("lib.rs"),
        r"
fn caller() -> i32 {
    shared_helper(42)
}

fn shared_helper(x: i32) -> i32 {
    x * 2
}
",
    )
    .unwrap();

    let illu_b = dir_b.path().join(".illu");
    std::fs::create_dir_all(&illu_b).unwrap();
    let db_b = Database::open(&illu_b.join("index.db")).unwrap();
    let config_b = IndexConfig {
        repo_path: dir_b.path().to_path_buf(),
    };
    index_repo(&db_b, &config_b).unwrap();

    (dir_a, dir_b, db_a, db_b)
}

fn make_registry(
    dir_a: &std::path::Path,
    dir_b: &std::path::Path,
) -> (tempfile::TempDir, Registry) {
    let reg_dir = tempfile::TempDir::new().unwrap();
    let reg_path = reg_dir.path().join("registry.toml");
    let mut registry = Registry::load(&reg_path).unwrap();

    registry.register(RepoEntry {
        name: "repo-a".to_string(),
        path: dir_a.to_path_buf(),
        git_remote: None,
        git_common_dir: dir_a.join(".git"),
        last_indexed: "2026-01-01T00:00:00Z".to_string(),
    });
    registry.register(RepoEntry {
        name: "repo-b".to_string(),
        path: dir_b.to_path_buf(),
        git_remote: None,
        git_common_dir: dir_b.join(".git"),
        last_indexed: "2026-01-01T00:00:00Z".to_string(),
    });

    (reg_dir, registry)
}

#[test]
fn cross_query_finds_symbols_in_other_repos() {
    let (dir_a, dir_b, _db_a, _db_b) = setup_two_repos();
    let (_reg_dir, registry) = make_registry(dir_a.path(), dir_b.path());

    let opts = cross_query::CrossQueryOpts {
        query: "shared_helper",
        scope: Some("symbols"),
        kind: None,
        attribute: None,
        signature: None,
        path: None,
        limit: None,
    };

    let result = cross_query::handle_cross_query(&registry, dir_a.path(), &opts).unwrap();

    assert!(
        result.contains("repo-b"),
        "cross_query should show repo-b header: {result}"
    );
    assert!(
        result.contains("shared_helper"),
        "cross_query should find shared_helper in repo-b: {result}"
    );
}

#[test]
fn repos_shows_registered_repos() {
    let (dir_a, dir_b, _db_a, _db_b) = setup_two_repos();
    let (_reg_dir, registry) = make_registry(dir_a.path(), dir_b.path());

    let result = repos::handle_repos(&registry, dir_a.path()).unwrap();

    assert!(
        result.contains("repo-a"),
        "repos should list repo-a: {result}"
    );
    assert!(
        result.contains("repo-b"),
        "repos should list repo-b: {result}"
    );
    assert!(
        result.contains("active"),
        "repo-a should be marked active: {result}"
    );
}

#[test]
fn cross_deps_finds_shared_dependencies() {
    let dir_a = tempfile::TempDir::new().unwrap();
    let src_a = dir_a.path().join("src");
    std::fs::create_dir_all(&src_a).unwrap();

    std::fs::write(
        dir_a.path().join("Cargo.toml"),
        r#"
[package]
name = "repo-a"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = "1.0"
"#,
    )
    .unwrap();

    std::fs::write(src_a.join("lib.rs"), "").unwrap();

    let dir_b = tempfile::TempDir::new().unwrap();
    let src_b = dir_b.path().join("src");
    std::fs::create_dir_all(&src_b).unwrap();

    std::fs::write(
        dir_b.path().join("Cargo.toml"),
        r#"
[package]
name = "repo-b"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = "1.0"
tokio = "1.0"
"#,
    )
    .unwrap();

    std::fs::write(src_b.join("lib.rs"), "").unwrap();

    let (_reg_dir, registry) = make_registry(dir_a.path(), dir_b.path());

    let result = cross_deps::handle_cross_deps(&registry).unwrap();

    assert!(
        result.contains("Shared Dependencies"),
        "should have shared deps section: {result}"
    );
    assert!(
        result.contains("serde"),
        "serde should appear as shared dep: {result}"
    );
}

#[test]
fn cross_impact_finds_references() {
    let (dir_a, dir_b, _db_a, _db_b) = setup_two_repos();
    let (_reg_dir, registry) = make_registry(dir_a.path(), dir_b.path());

    let result =
        cross_impact::handle_cross_impact(&registry, dir_a.path(), "shared_helper", None).unwrap();

    assert!(
        result.contains("Cross-Repo Impact"),
        "should have cross-repo impact header: {result}"
    );
    // Repo B has its own shared_helper called by caller(),
    // so there should be references in repo-b's index.
    assert!(
        result.contains("repo-b"),
        "should find references in repo-b: {result}"
    );
}
