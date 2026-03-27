#![expect(clippy::unwrap_used, reason = "integration tests")]

use illu_rs::db::Database;
use illu_rs::indexer::{IndexConfig, index_repo};
use illu_rs::registry::{Registry, RepoEntry};
use illu_rs::server::tools::{cross_deps, cross_impact, cross_query, repos};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create an indexed repo on disk with `.illu/index.db`.
/// Returns `(TempDir, Database)` -- `TempDir` must stay alive.
fn create_indexed_repo(name: &str, lib_rs: &str, deps: &str) -> (tempfile::TempDir, Database) {
    let dir = tempfile::TempDir::new().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();

    let cargo_toml =
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n{deps}");
    std::fs::write(dir.path().join("Cargo.toml"), &cargo_toml).unwrap();
    std::fs::write(
        dir.path().join("Cargo.lock"),
        format!("[[package]]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
    )
    .unwrap();
    std::fs::write(src.join("lib.rs"), lib_rs).unwrap();

    let illu_dir = dir.path().join(".illu");
    std::fs::create_dir_all(&illu_dir).unwrap();
    let db = Database::open(&illu_dir.join("index.db")).unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();

    (dir, db)
}

fn build_registry(entries: &[(&str, &std::path::Path)]) -> (tempfile::TempDir, Registry) {
    let reg_dir = tempfile::TempDir::new().unwrap();
    let reg_path = reg_dir.path().join("registry.toml");
    let mut registry = Registry::load(&reg_path).unwrap();

    for &(name, path) in entries {
        registry.register(RepoEntry {
            name: name.to_string(),
            path: path.to_path_buf(),
            git_remote: None,
            git_common_dir: path.join(".git"),
            last_indexed: "2026-01-01T00:00:00Z".to_string(),
        });
    }

    (reg_dir, registry)
}

// ===========================================================================
// Group 1: Registry Correctness
// ===========================================================================

#[test]
fn registry_save_load_roundtrip() {
    let reg_dir = tempfile::TempDir::new().unwrap();
    let reg_path = reg_dir.path().join("registry.toml");
    let mut registry = Registry::load(&reg_path).unwrap();

    let path_a = PathBuf::from("/tmp/test-repo-a");
    let path_b = PathBuf::from("/tmp/test-repo-b");

    registry.register(RepoEntry {
        name: "alpha".to_string(),
        path: path_a.clone(),
        git_remote: Some("git@github.com:user/alpha.git".to_string()),
        git_common_dir: path_a.join(".git"),
        last_indexed: "2026-03-01T12:00:00Z".to_string(),
    });
    registry.register(RepoEntry {
        name: "beta".to_string(),
        path: path_b.clone(),
        git_remote: None,
        git_common_dir: path_b.join(".git"),
        last_indexed: "2026-03-02T08:30:00Z".to_string(),
    });

    registry.save().unwrap();

    let reloaded = Registry::load(&reg_path).unwrap();
    assert_eq!(reloaded.repos.len(), 2, "should have 2 repos after reload");

    let alpha = reloaded.repos.iter().find(|r| r.name == "alpha").unwrap();
    assert_eq!(alpha.path, path_a);
    assert_eq!(
        alpha.git_remote.as_deref(),
        Some("git@github.com:user/alpha.git")
    );
    assert_eq!(alpha.last_indexed, "2026-03-01T12:00:00Z");
    assert_eq!(alpha.git_common_dir, path_a.join(".git"));

    let beta = reloaded.repos.iter().find(|r| r.name == "beta").unwrap();
    assert_eq!(beta.path, path_b);
    assert!(beta.git_remote.is_none());
    assert_eq!(beta.last_indexed, "2026-03-02T08:30:00Z");
}

#[test]
fn registry_dedup_by_git_common_dir() {
    let reg_dir = tempfile::TempDir::new().unwrap();
    let reg_path = reg_dir.path().join("registry.toml");
    let mut registry = Registry::load(&reg_path).unwrap();

    let common_git = PathBuf::from("/tmp/shared/.git");

    registry.register(RepoEntry {
        name: "main-checkout".to_string(),
        path: PathBuf::from("/tmp/shared/main"),
        git_remote: None,
        git_common_dir: common_git.clone(),
        last_indexed: "2026-01-01T00:00:00Z".to_string(),
    });
    registry.register(RepoEntry {
        name: "worktree-checkout".to_string(),
        path: PathBuf::from("/tmp/shared/worktree"),
        git_remote: Some("git@github.com:user/repo.git".to_string()),
        git_common_dir: common_git,
        last_indexed: "2026-02-01T00:00:00Z".to_string(),
    });

    assert_eq!(
        registry.repos.len(),
        1,
        "dedup should keep only one entry: {:?}",
        registry.repos
    );
    assert_eq!(
        registry.repos[0].last_indexed, "2026-02-01T00:00:00Z",
        "latest timestamp should win"
    );
}

#[test]
fn registry_prune_removes_missing_repos() {
    let alive_dir = tempfile::TempDir::new().unwrap();
    let dead_dir = tempfile::TempDir::new().unwrap();
    let dead_path = dead_dir.path().to_path_buf();
    drop(dead_dir);

    let reg_dir = tempfile::TempDir::new().unwrap();
    let reg_path = reg_dir.path().join("registry.toml");
    let mut registry = Registry::load(&reg_path).unwrap();

    registry.register(RepoEntry {
        name: "alive".to_string(),
        path: alive_dir.path().to_path_buf(),
        git_remote: None,
        git_common_dir: alive_dir.path().join(".git"),
        last_indexed: "2026-01-01T00:00:00Z".to_string(),
    });
    registry.register(RepoEntry {
        name: "dead".to_string(),
        path: dead_path.clone(),
        git_remote: None,
        git_common_dir: dead_path.join(".git"),
        last_indexed: "2026-01-01T00:00:00Z".to_string(),
    });
    assert_eq!(registry.repos.len(), 2);

    registry.prune();

    assert_eq!(registry.repos.len(), 1, "prune should remove dead repo");
    assert_eq!(registry.repos[0].name, "alive");
}

#[test]
fn registry_other_repos_excludes_primary() {
    let dir_a = tempfile::TempDir::new().unwrap();
    let dir_b = tempfile::TempDir::new().unwrap();

    let (_reg_dir, registry) =
        build_registry(&[("repo-a", dir_a.path()), ("repo-b", dir_b.path())]);

    let others = registry.other_repos(dir_a.path());
    assert_eq!(others.len(), 1, "should exclude primary: {others:?}");
    assert_eq!(others[0].name, "repo-b");

    for entry in &others {
        assert_ne!(
            entry.path,
            dir_a.path(),
            "primary should never appear in other_repos"
        );
    }
}

// ===========================================================================
// Group 2: Cross-Query Accuracy
// ===========================================================================

#[test]
fn cross_query_finds_symbol_in_other_repo() {
    let (dir_a, _db_a) = create_indexed_repo("repo-a", "pub fn only_in_a() {}", "");
    let (dir_b, _db_b) = create_indexed_repo("repo-b", "pub fn unique_to_b() -> i32 { 42 }", "");

    let (_reg_dir, registry) =
        build_registry(&[("repo-a", dir_a.path()), ("repo-b", dir_b.path())]);

    let opts = cross_query::CrossQueryOpts {
        query: "unique_to_b",
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
        "should show repo-b header: {result}"
    );
    assert!(
        result.contains("unique_to_b"),
        "should find unique_to_b in repo-b: {result}"
    );
}

#[test]
fn cross_query_excludes_primary_repo_results() {
    let (dir_a, _db_a) = create_indexed_repo("repo-a", "pub fn shared_name() -> i32 { 1 }", "");
    let (dir_b, _db_b) = create_indexed_repo("repo-b", "pub fn shared_name() -> i32 { 2 }", "");

    let (_reg_dir, registry) =
        build_registry(&[("repo-a", dir_a.path()), ("repo-b", dir_b.path())]);

    let opts = cross_query::CrossQueryOpts {
        query: "shared_name",
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
        "should include repo-b results: {result}"
    );
    assert!(
        !result.contains("repo-a"),
        "should NOT include primary repo-a results: {result}"
    );
}

#[test]
fn cross_query_with_kind_filter_works() {
    let (dir_a, _db_a) = create_indexed_repo("repo-a", "pub fn placeholder() {}", "");
    let (dir_b, _db_b) = create_indexed_repo(
        "repo-b",
        r"
pub struct Widget { pub id: u32 }
pub fn widget_builder() -> u32 { 0 }
",
        "",
    );

    let (_reg_dir, registry) =
        build_registry(&[("repo-a", dir_a.path()), ("repo-b", dir_b.path())]);

    let opts = cross_query::CrossQueryOpts {
        query: "Widget",
        scope: Some("symbols"),
        kind: Some("struct"),
        attribute: None,
        signature: None,
        path: None,
        limit: None,
    };

    let result = cross_query::handle_cross_query(&registry, dir_a.path(), &opts).unwrap();

    assert!(
        result.contains("Widget"),
        "should find Widget struct: {result}"
    );
    assert!(
        !result.contains("widget_builder"),
        "should NOT include the function: {result}"
    );
}

#[test]
fn cross_query_returns_clear_message_when_no_repos() {
    let reg_dir = tempfile::TempDir::new().unwrap();
    let reg_path = reg_dir.path().join("registry.toml");
    let registry = Registry::load(&reg_path).unwrap();

    let dummy_path = PathBuf::from("/tmp/nonexistent");
    let opts = cross_query::CrossQueryOpts {
        query: "anything",
        scope: None,
        kind: None,
        attribute: None,
        signature: None,
        path: None,
        limit: None,
    };

    let result = cross_query::handle_cross_query(&registry, &dummy_path, &opts).unwrap();

    assert!(
        result.contains("No other repos"),
        "empty registry should say no repos: {result}"
    );
}

// ===========================================================================
// Group 3: Cross-Impact Accuracy
// ===========================================================================

#[test]
fn cross_impact_finds_name_based_refs() {
    let (dir_a, _db_a) = create_indexed_repo(
        "repo-a",
        "pub fn shared_helper(x: i32) -> i32 { x + 1 }",
        "",
    );
    let (dir_b, _db_b) = create_indexed_repo(
        "repo-b",
        r"
fn caller() -> i32 {
    shared_helper(42)
}
fn shared_helper(x: i32) -> i32 { x * 2 }
",
        "",
    );

    let (_reg_dir, registry) =
        build_registry(&[("repo-a", dir_a.path()), ("repo-b", dir_b.path())]);

    let result =
        cross_impact::handle_cross_impact(&registry, dir_a.path(), "shared_helper").unwrap();

    assert!(
        result.contains("Cross-Repo Impact"),
        "should have impact header: {result}"
    );
    assert!(
        result.contains("repo-b"),
        "should find references in repo-b: {result}"
    );
}

#[test]
fn cross_impact_respects_impl_type() {
    let (dir_a, _db_a) = create_indexed_repo(
        "repo-a",
        r"
pub struct Foo;
impl Foo {
    pub fn process() -> i32 { 1 }
}
",
        "",
    );
    let (dir_b, _db_b) = create_indexed_repo(
        "repo-b",
        r"
pub struct Bar;
impl Bar {
    pub fn process() -> i32 { 2 }
}
fn user() -> i32 { Bar::process() }
",
        "",
    );

    let (_reg_dir, registry) =
        build_registry(&[("repo-a", dir_a.path()), ("repo-b", dir_b.path())]);

    let result =
        cross_impact::handle_cross_impact(&registry, dir_a.path(), "Foo::process").unwrap();

    // Repo B only has Bar::process, not Foo::process.
    // With impl_type filtering, repo B's refs should not match.
    assert!(
        result.contains("No references") || !result.contains("repo-b"),
        "Foo::process should NOT match Bar::process in repo-b: {result}"
    );
}

#[test]
fn cross_impact_with_no_refs_returns_clear_message() {
    let (dir_a, _db_a) = create_indexed_repo("repo-a", "pub fn unique_to_a() -> bool { true }", "");
    let (dir_b, _db_b) =
        create_indexed_repo("repo-b", "pub fn unrelated_fn() -> bool { false }", "");

    let (_reg_dir, registry) =
        build_registry(&[("repo-a", dir_a.path()), ("repo-b", dir_b.path())]);

    let result = cross_impact::handle_cross_impact(&registry, dir_a.path(), "unique_to_a").unwrap();

    assert!(
        result.contains("No references") || result.contains("No cross-repo"),
        "should indicate no cross-repo refs: {result}"
    );
}

// ===========================================================================
// Group 4: Cross-Deps
// ===========================================================================

#[test]
fn cross_deps_finds_shared_crate_dependencies() {
    let dir_a = tempfile::TempDir::new().unwrap();
    let src_a = dir_a.path().join("src");
    std::fs::create_dir_all(&src_a).unwrap();
    std::fs::write(
        dir_a.path().join("Cargo.toml"),
        "[package]\nname = \"repo-a\"\nversion = \"0.1.0\"\n\
         edition = \"2021\"\n\n[dependencies]\nserde = \"1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        dir_a.path().join("Cargo.lock"),
        "[[package]]\nname = \"repo-a\"\nversion = \"0.1.0\"\n\n\
         [[package]]\nname = \"serde\"\nversion = \"1.0.210\"\n",
    )
    .unwrap();
    std::fs::write(src_a.join("lib.rs"), "").unwrap();

    let dir_b = tempfile::TempDir::new().unwrap();
    let src_b = dir_b.path().join("src");
    std::fs::create_dir_all(&src_b).unwrap();
    std::fs::write(
        dir_b.path().join("Cargo.toml"),
        "[package]\nname = \"repo-b\"\nversion = \"0.1.0\"\n\
         edition = \"2021\"\n\n[dependencies]\nserde = \"1.0\"\n\
         tokio = \"1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        dir_b.path().join("Cargo.lock"),
        "[[package]]\nname = \"repo-b\"\nversion = \"0.1.0\"\n\n\
         [[package]]\nname = \"serde\"\nversion = \"1.0.210\"\n\n\
         [[package]]\nname = \"tokio\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    std::fs::write(src_b.join("lib.rs"), "").unwrap();

    let (_reg_dir, registry) =
        build_registry(&[("repo-a", dir_a.path()), ("repo-b", dir_b.path())]);

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
fn cross_deps_with_no_overlap_returns_clear_message() {
    let dir_a = tempfile::TempDir::new().unwrap();
    let src_a = dir_a.path().join("src");
    std::fs::create_dir_all(&src_a).unwrap();
    std::fs::write(
        dir_a.path().join("Cargo.toml"),
        "[package]\nname = \"repo-a\"\nversion = \"0.1.0\"\n\
         edition = \"2021\"\n\n[dependencies]\nrand = \"0.8\"\n",
    )
    .unwrap();
    std::fs::write(
        dir_a.path().join("Cargo.lock"),
        "[[package]]\nname = \"repo-a\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(src_a.join("lib.rs"), "").unwrap();

    let dir_b = tempfile::TempDir::new().unwrap();
    let src_b = dir_b.path().join("src");
    std::fs::create_dir_all(&src_b).unwrap();
    std::fs::write(
        dir_b.path().join("Cargo.toml"),
        "[package]\nname = \"repo-b\"\nversion = \"0.1.0\"\n\
         edition = \"2021\"\n\n[dependencies]\ntokio = \"1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        dir_b.path().join("Cargo.lock"),
        "[[package]]\nname = \"repo-b\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(src_b.join("lib.rs"), "").unwrap();

    let (_reg_dir, registry) =
        build_registry(&[("repo-a", dir_a.path()), ("repo-b", dir_b.path())]);

    let result = cross_deps::handle_cross_deps(&registry).unwrap();

    assert!(
        !result.contains("Shared Dependencies"),
        "should NOT have shared deps section: {result}"
    );
}

#[test]
fn cross_deps_detects_path_dependencies() {
    let dir_a = tempfile::TempDir::new().unwrap();
    let src_a = dir_a.path().join("src");
    std::fs::create_dir_all(&src_a).unwrap();
    std::fs::write(
        dir_a.path().join("Cargo.toml"),
        "[package]\nname = \"repo-a\"\nversion = \"0.1.0\"\n\
         edition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir_a.path().join("Cargo.lock"),
        "[[package]]\nname = \"repo-a\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(src_a.join("lib.rs"), "pub fn a_func() {}").unwrap();

    let dir_b = tempfile::TempDir::new().unwrap();
    let src_b = dir_b.path().join("src");
    std::fs::create_dir_all(&src_b).unwrap();

    let a_path_str = dir_a.path().display();
    std::fs::write(
        dir_b.path().join("Cargo.toml"),
        format!(
            "[package]\nname = \"repo-b\"\nversion = \"0.1.0\"\n\
             edition = \"2021\"\n\n\
             [dependencies]\nrepo-a = {{ path = \"{a_path_str}\" }}\n"
        ),
    )
    .unwrap();
    std::fs::write(
        dir_b.path().join("Cargo.lock"),
        "[[package]]\nname = \"repo-b\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(src_b.join("lib.rs"), "").unwrap();

    let (_reg_dir, registry) =
        build_registry(&[("repo-a", dir_a.path()), ("repo-b", dir_b.path())]);

    let result = cross_deps::handle_cross_deps(&registry).unwrap();

    assert!(
        result.contains("Path Dependencies"),
        "should detect path dep from repo-b to repo-a: {result}"
    );
    assert!(
        result.contains("repo-b") && result.contains("repo-a"),
        "should name both repos in path dep: {result}"
    );
}

// ===========================================================================
// Group 5: Error Handling
// ===========================================================================

#[test]
fn cross_query_skips_repo_with_missing_db() {
    let (dir_a, _db_a) = create_indexed_repo("repo-a", "pub fn in_a() {}", "");

    // repo-b: directory exists but no .illu/index.db
    let dir_b = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(dir_b.path().join("src")).unwrap();

    let (dir_c, _db_c) = create_indexed_repo("repo-c", "pub fn found_in_c() -> bool { true }", "");

    let (_reg_dir, registry) = build_registry(&[
        ("repo-a", dir_a.path()),
        ("repo-b", dir_b.path()),
        ("repo-c", dir_c.path()),
    ]);

    let opts = cross_query::CrossQueryOpts {
        query: "found_in_c",
        scope: Some("symbols"),
        kind: None,
        attribute: None,
        signature: None,
        path: None,
        limit: None,
    };

    let result = cross_query::handle_cross_query(&registry, dir_a.path(), &opts).unwrap();

    assert!(
        result.contains("found_in_c"),
        "should find symbol in repo-c despite repo-b missing DB: {result}"
    );
    assert!(
        result.contains("repo-c"),
        "should show repo-c header: {result}"
    );
}

#[test]
fn cross_query_on_empty_other_repos() {
    let (dir_a, _db_a) = create_indexed_repo("repo-a", "pub fn only_repo() {}", "");

    let (_reg_dir, registry) = build_registry(&[("repo-a", dir_a.path())]);

    let opts = cross_query::CrossQueryOpts {
        query: "anything",
        scope: None,
        kind: None,
        attribute: None,
        signature: None,
        path: None,
        limit: None,
    };

    let result = cross_query::handle_cross_query(&registry, dir_a.path(), &opts).unwrap();

    assert!(
        result.contains("No other repos"),
        "single-repo registry should say no other repos: {result}"
    );
}

#[test]
fn cross_impact_on_nonexistent_symbol_returns_message() {
    let (dir_a, _db_a) = create_indexed_repo("repo-a", "pub fn real_fn() {}", "");
    let (dir_b, _db_b) = create_indexed_repo("repo-b", "pub fn other_fn() {}", "");

    let (_reg_dir, registry) =
        build_registry(&[("repo-a", dir_a.path()), ("repo-b", dir_b.path())]);

    let result = cross_impact::handle_cross_impact(
        &registry,
        dir_a.path(),
        "completely_nonexistent_symbol_xyz",
    )
    .unwrap();

    assert!(
        result.contains("No references") || result.contains("No cross-repo"),
        "nonexistent symbol should produce no-refs message: {result}"
    );
}

#[test]
fn cross_tools_handle_stale_registry_path() {
    let stale_dir = tempfile::TempDir::new().unwrap();
    let stale_path = stale_dir.path().to_path_buf();
    drop(stale_dir);

    let (dir_a, _db_a) = create_indexed_repo("repo-a", "pub fn alive() {}", "");

    let (_reg_dir, registry) =
        build_registry(&[("repo-a", dir_a.path()), ("stale-repo", &stale_path)]);

    let opts = cross_query::CrossQueryOpts {
        query: "alive",
        scope: Some("symbols"),
        kind: None,
        attribute: None,
        signature: None,
        path: None,
        limit: None,
    };
    let result = cross_query::handle_cross_query(&registry, dir_a.path(), &opts).unwrap();

    assert!(
        !result.is_empty(),
        "should return some output, not crash: {result}"
    );

    let impact_result =
        cross_impact::handle_cross_impact(&registry, dir_a.path(), "alive").unwrap();
    assert!(
        !impact_result.is_empty(),
        "cross_impact should not crash on stale path: {impact_result}"
    );
}

// ===========================================================================
// Group 6: Readonly DB Behavior
// ===========================================================================

#[test]
fn readonly_db_queries_work() {
    let (dir, _db) = create_indexed_repo(
        "readonly-test",
        r"
pub fn queryable_fn(x: i32) -> i32 { x }
pub struct QueryableStruct { pub val: u64 }
",
        "",
    );

    let db_path = dir.path().join(".illu/index.db");
    let ro_db = Database::open_readonly(&db_path).unwrap();

    let results = ro_db.search_symbols("queryable_fn").unwrap();
    assert!(
        !results.is_empty(),
        "readonly DB should support search_symbols"
    );
    assert_eq!(results[0].name, "queryable_fn");

    let cross_refs = ro_db.find_cross_refs("queryable_fn", None).unwrap();
    // May or may not have refs, but the call must succeed
    let _ = cross_refs;

    let struct_results = ro_db.search_symbols("QueryableStruct").unwrap();
    assert!(
        !struct_results.is_empty(),
        "readonly DB should find structs too"
    );
}

#[test]
fn readonly_db_cannot_write() {
    let (dir, _db) = create_indexed_repo("readonly-write-test", "pub fn immutable() {}", "");

    let db_path = dir.path().join(".illu/index.db");
    let ro_db = Database::open_readonly(&db_path).unwrap();

    // clear_code_index is the first write in index_repo and will
    // fail on a readonly connection.
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    let result = index_repo(&ro_db, &config);

    assert!(result.is_err(), "index_repo on readonly DB should fail");
}

// ===========================================================================
// Group 7: Repos Tool
// ===========================================================================

#[test]
fn repos_tool_shows_all_registered() {
    let (dir_a, _db_a) = create_indexed_repo("alpha", "pub fn a() {}", "");
    let (dir_b, _db_b) = create_indexed_repo("beta", "pub fn b() {}", "");
    let (dir_c, _db_c) = create_indexed_repo("gamma", "pub fn c() {}", "");

    let (_reg_dir, registry) = build_registry(&[
        ("alpha", dir_a.path()),
        ("beta", dir_b.path()),
        ("gamma", dir_c.path()),
    ]);

    let result = repos::handle_repos(&registry, dir_a.path()).unwrap();

    assert!(result.contains("alpha"), "should list alpha: {result}");
    assert!(result.contains("beta"), "should list beta: {result}");
    assert!(result.contains("gamma"), "should list gamma: {result}");
}

#[test]
fn repos_tool_marks_primary_as_active() {
    let (dir_a, _db_a) = create_indexed_repo("primary-repo", "pub fn p() {}", "");
    let (dir_b, _db_b) = create_indexed_repo("secondary-repo", "pub fn s() {}", "");

    let (_reg_dir, registry) = build_registry(&[
        ("primary-repo", dir_a.path()),
        ("secondary-repo", dir_b.path()),
    ]);

    let result = repos::handle_repos(&registry, dir_a.path()).unwrap();

    assert!(
        result.contains("active"),
        "primary should be marked active: {result}"
    );

    let lines: Vec<&str> = result.lines().collect();
    let secondary_line = lines.iter().find(|l| l.contains("secondary-repo")).unwrap();
    assert!(
        secondary_line.contains("indexed"),
        "secondary should be 'indexed', not 'active': {secondary_line}"
    );

    let primary_line = lines.iter().find(|l| l.contains("primary-repo")).unwrap();
    assert!(
        primary_line.contains("active"),
        "primary should be 'active': {primary_line}"
    );
}
