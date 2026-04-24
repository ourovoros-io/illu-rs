//! Tests that incremental re-indexing (`refresh_index`) correctly
//! cleans up stale symbols, refs, and line numbers without leaving
//! ghost data behind.

#![expect(clippy::unwrap_used, reason = "integration tests")]

use illu_rs::api::db::Database;
use illu_rs::api::indexer::{IndexConfig, index_repo, refresh_index};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn setup_git_repo(dir: &std::path::Path) {
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(dir)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "-m", "init"])
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@test.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@test.com")
        .output()
        .unwrap();
}

/// Create a fully on-disk indexed project with git, suitable for
/// refresh tests. Returns (dir, db, config).
fn setup_refresh_project(files: &[(&str, &str)]) -> (tempfile::TempDir, Database, IndexConfig) {
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

    let illu_dir = dir.path().join(".illu");
    std::fs::create_dir_all(&illu_dir).unwrap();
    let db = Database::open(&illu_dir.join("index.db")).unwrap();
    let config = IndexConfig {
        repo_path: dir.path().to_path_buf(),
    };
    index_repo(&db, &config).unwrap();

    // Init git after indexing so we have a baseline commit
    setup_git_repo(dir.path());

    (dir, db, config)
}

/// Stage all changes and commit so that subsequent `refresh_index`
/// calls see the working tree as clean relative to HEAD.
fn git_commit(dir: &std::path::Path, msg: &str) {
    std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(dir)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "-m", msg])
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@test.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@test.com")
        .output()
        .unwrap();
}

// ---------------------------------------------------------------------------
// Group 1: Symbol Lifecycle
// ---------------------------------------------------------------------------

#[test]
fn refresh_adds_new_symbol() {
    let (dir, db, config) = setup_refresh_project(&[("lib.rs", "pub fn original() {}\n")]);

    // Add a second function
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn original() {}\npub fn added() {}\n",
    )
    .unwrap();

    let refreshed = refresh_index(&db, &config).unwrap();
    assert!(refreshed > 0, "refresh should re-index the changed file");

    let syms = db.search_symbols_exact("added").unwrap();
    assert!(
        !syms.is_empty(),
        "newly added symbol should be found after refresh"
    );
    let syms = db.search_symbols_exact("original").unwrap();
    assert!(
        !syms.is_empty(),
        "original symbol should still exist after refresh"
    );
}

#[test]
fn refresh_removes_deleted_symbol() {
    let (dir, db, config) =
        setup_refresh_project(&[("lib.rs", "pub fn keeper() {}\npub fn doomed() {}\n")]);

    assert!(
        !db.search_symbols_exact("doomed").unwrap().is_empty(),
        "doomed should exist before refresh"
    );

    // Rewrite file without doomed
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn keeper() {}\n").unwrap();

    refresh_index(&db, &config).unwrap();

    let syms = db.search_symbols_exact("doomed").unwrap();
    assert!(
        syms.is_empty(),
        "deleted symbol 'doomed' should be gone after refresh"
    );
    let syms = db.search_symbols_exact("keeper").unwrap();
    assert!(
        !syms.is_empty(),
        "kept symbol 'keeper' should still exist after refresh"
    );
}

#[test]
fn refresh_updates_changed_signature() {
    let (dir, db, config) = setup_refresh_project(&[("lib.rs", "pub fn evolve(x: i32) {}\n")]);

    let before = db.search_symbols_exact("evolve").unwrap();
    assert!(
        !before[0].signature.contains("y: String"),
        "signature should not contain y: String before edit"
    );

    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn evolve(x: i32, y: String) {}\n",
    )
    .unwrap();

    refresh_index(&db, &config).unwrap();

    let after = db.search_symbols_exact("evolve").unwrap();
    assert!(
        after[0].signature.contains("y: String"),
        "signature should contain y: String after refresh, got: {}",
        after[0].signature
    );
}

#[test]
fn refresh_updates_moved_line_numbers() {
    let (dir, db, config) = setup_refresh_project(&[("lib.rs", "pub fn target() {}\n")]);

    let before = db.search_symbols_exact("target").unwrap();
    assert_eq!(
        before[0].line_start, 1,
        "target should start at line 1 initially"
    );

    // Prepend 5 blank lines
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "\n\n\n\n\npub fn target() {}\n",
    )
    .unwrap();

    refresh_index(&db, &config).unwrap();

    let after = db.search_symbols_exact("target").unwrap();
    assert_eq!(
        after[0].line_start, 6,
        "target should start at line 6 after prepending 5 blank lines"
    );
}

#[test]
fn refresh_preserves_unchanged_symbols() {
    let (dir, db, config) = setup_refresh_project(&[
        ("lib.rs", "pub mod other;\npub fn lib_fn(x: i32) {}\n"),
        ("other.rs", "pub fn other_fn(a: bool) {}\n"),
    ]);

    let before = db.search_symbols_exact("other_fn").unwrap();
    assert_eq!(before.len(), 1, "other_fn should exist before refresh");
    let sig_before = before[0].signature.clone();

    // Edit only lib.rs
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub mod other;\npub fn lib_fn(x: i32, z: f64) {}\n",
    )
    .unwrap();

    refresh_index(&db, &config).unwrap();

    let after = db.search_symbols_exact("other_fn").unwrap();
    assert_eq!(
        after.len(),
        1,
        "other_fn should still exist after editing lib.rs only"
    );
    assert_eq!(
        after[0].signature, sig_before,
        "other_fn signature should be unchanged"
    );

    // Verify lib_fn was updated
    let lib_after = db.search_symbols_exact("lib_fn").unwrap();
    assert!(
        lib_after[0].signature.contains("z: f64"),
        "lib_fn should have updated signature"
    );
}

// ---------------------------------------------------------------------------
// Group 2: Reference Lifecycle
// ---------------------------------------------------------------------------

#[test]
fn refresh_removes_refs_from_deleted_file() {
    let (dir, db, config) = setup_refresh_project(&[
        ("lib.rs", "pub mod extra;\npub fn target() {}\n"),
        ("extra.rs", "pub fn caller() { crate::target(); }\n"),
    ]);

    // Verify caller symbol exists initially
    let syms = db.search_symbols_exact("caller").unwrap();
    assert!(
        !syms.is_empty(),
        "caller should exist in extra.rs before deletion"
    );

    // Delete extra.rs and update lib.rs
    std::fs::remove_file(dir.path().join("src/extra.rs")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn target() {}\n").unwrap();

    refresh_index(&db, &config).unwrap();

    let syms = db.search_symbols_exact("caller").unwrap();
    assert!(
        syms.is_empty(),
        "symbol 'caller' from deleted file should be gone"
    );

    // Target should still exist
    let syms = db.search_symbols_exact("target").unwrap();
    assert!(
        !syms.is_empty(),
        "target should survive after extra.rs deletion"
    );
}

#[test]
fn refresh_removes_refs_to_deleted_symbol() {
    let (dir, db, config) = setup_refresh_project(&[(
        "lib.rs",
        "pub fn helper() {}\npub fn caller() { helper(); }\n",
    )]);

    // Verify initial state
    assert!(
        !db.search_symbols_exact("helper").unwrap().is_empty(),
        "helper should exist initially"
    );

    // Rename helper to renamed_helper and update call
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn renamed_helper() {}\npub fn caller() { renamed_helper(); }\n",
    )
    .unwrap();

    refresh_index(&db, &config).unwrap();

    let old = db.search_symbols_exact("helper").unwrap();
    assert!(
        old.is_empty(),
        "old name 'helper' should not exist after rename"
    );

    let new = db.search_symbols_exact("renamed_helper").unwrap();
    assert!(
        !new.is_empty(),
        "'renamed_helper' should exist after refresh"
    );

    let callees = db.get_callees("caller", "src/lib.rs", false).unwrap();
    let calls_renamed = callees.iter().any(|c| c.name == "renamed_helper");
    assert!(
        calls_renamed,
        "caller should call renamed_helper, got callees: {:?}",
        callees.iter().map(|c| &c.name).collect::<Vec<_>>()
    );
}

#[test]
fn refresh_updates_ref_when_call_removed() {
    let (dir, db, config) = setup_refresh_project(&[(
        "lib.rs",
        "pub fn target() {}\npub fn caller() { target(); }\n",
    )]);

    let impact = db.impact_dependents_with_depth("target", None, 5).unwrap();
    assert!(
        impact.iter().any(|e| e.name == "caller"),
        "caller should be in impact of target initially"
    );

    // Remove the call
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn target() {}\npub fn caller() {}\n",
    )
    .unwrap();

    refresh_index(&db, &config).unwrap();

    let impact = db.impact_dependents_with_depth("target", None, 5).unwrap();
    assert!(
        !impact.iter().any(|e| e.name == "caller"),
        "caller should NOT be in impact of target after call removal"
    );
}

#[test]
fn refresh_adds_ref_when_call_added() {
    let (dir, db, config) =
        setup_refresh_project(&[("lib.rs", "pub fn target() {}\npub fn caller() {}\n")]);

    let impact = db.impact_dependents_with_depth("target", None, 5).unwrap();
    assert!(
        !impact.iter().any(|e| e.name == "caller"),
        "caller should NOT be in impact of target initially"
    );

    // Add the call
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn target() {}\npub fn caller() { target(); }\n",
    )
    .unwrap();

    refresh_index(&db, &config).unwrap();

    let impact = db.impact_dependents_with_depth("target", None, 5).unwrap();
    assert!(
        impact.iter().any(|e| e.name == "caller"),
        "caller should appear in impact of target after call added"
    );
}

#[test]
fn refresh_refs_from_untouched_files_survive() {
    // Two files: lib.rs and other.rs. Edit only lib.rs, verify
    // symbols and data in other.rs survive the partial refresh.
    let (dir, db, config) = setup_refresh_project(&[
        (
            "lib.rs",
            "pub mod other;\npub fn lib_target() -> i32 { 1 }\n",
        ),
        (
            "other.rs",
            "pub fn other_caller() -> i32 { other_helper() }\npub fn other_helper() -> i32 { 99 }\n",
        ),
    ]);

    // Verify initial state: other_caller calls other_helper (same-file)
    let callees = db
        .get_callees("other_caller", "src/other.rs", false)
        .unwrap();
    assert!(
        callees.iter().any(|c| c.name == "other_helper"),
        "other_caller should call other_helper initially, got: {:?}",
        callees.iter().map(|c| &c.name).collect::<Vec<_>>()
    );

    // Edit only lib.rs (change body)
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub mod other;\npub fn lib_target() -> i32 { 42 }\n",
    )
    .unwrap();

    refresh_index(&db, &config).unwrap();

    // other.rs was not touched — its refs should survive
    let callees = db
        .get_callees("other_caller", "src/other.rs", false)
        .unwrap();
    assert!(
        callees.iter().any(|c| c.name == "other_helper"),
        "other_caller -> other_helper ref should survive partial refresh"
    );

    // Symbols from untouched file should be fully intact
    let syms = db.search_symbols_exact("other_caller").unwrap();
    assert!(
        !syms.is_empty(),
        "other_caller should still exist after partial refresh"
    );
    let syms = db.search_symbols_exact("other_helper").unwrap();
    assert!(
        !syms.is_empty(),
        "other_helper should still exist after partial refresh"
    );
}

// ---------------------------------------------------------------------------
// Group 3: Stale Data Scenarios
// ---------------------------------------------------------------------------

#[test]
fn no_ghost_symbols_after_file_delete() {
    let (dir, db, config) = setup_refresh_project(&[
        ("lib.rs", "pub mod extra;\npub fn lib_fn() {}\n"),
        ("extra.rs", "pub fn ghost() {}\n"),
    ]);

    assert!(
        !db.search_symbols_exact("ghost").unwrap().is_empty(),
        "ghost should exist before file deletion"
    );

    // Delete extra.rs and remove mod declaration
    std::fs::remove_file(dir.path().join("src/extra.rs")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn lib_fn() {}\n").unwrap();

    refresh_index(&db, &config).unwrap();

    let syms = db.search_symbols_exact("ghost").unwrap();
    assert!(
        syms.is_empty(),
        "ghost symbol from deleted file should be completely gone"
    );
}

#[test]
fn no_ghost_refs_after_symbol_rename() {
    let (dir, db, config) = setup_refresh_project(&[(
        "lib.rs",
        "pub fn helper() {}\npub fn user_fn() { helper(); }\n",
    )]);

    // Rename helper -> new_helper, update all calls
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn new_helper() {}\npub fn user_fn() { new_helper(); }\n",
    )
    .unwrap();

    refresh_index(&db, &config).unwrap();

    let old_syms = db.search_symbols_exact("helper").unwrap();
    assert!(
        old_syms.is_empty(),
        "old symbol name 'helper' should not exist after rename"
    );

    let old_impact = db.impact_dependents_with_depth("helper", None, 5).unwrap();
    assert!(
        old_impact.is_empty(),
        "no references to old name 'helper' should remain"
    );

    let new_impact = db
        .impact_dependents_with_depth("new_helper", None, 5)
        .unwrap();
    assert!(
        new_impact.iter().any(|e| e.name == "user_fn"),
        "user_fn should reference new_helper"
    );
}

#[test]
fn version_match_refresh_returns_zero_when_unchanged() {
    let (_dir, db, config) = setup_refresh_project(&[("lib.rs", "pub fn stable() {}\n")]);

    // Refresh immediately with no changes
    let refreshed = refresh_index(&db, &config).unwrap();
    assert_eq!(
        refreshed, 0,
        "refresh on up-to-date index with no changes should return 0"
    );

    // Symbols should still be intact
    let syms = db.search_symbols_exact("stable").unwrap();
    assert!(
        !syms.is_empty(),
        "stable symbol should still exist after no-op refresh"
    );
}

#[test]
fn refresh_handles_file_added_and_deleted_simultaneously() {
    let (dir, db, config) = setup_refresh_project(&[
        ("lib.rs", "pub mod old;\npub fn lib_fn() {}\n"),
        ("old.rs", "pub fn old_fn() {}\n"),
    ]);

    assert!(
        !db.search_symbols_exact("old_fn").unwrap().is_empty(),
        "old_fn should exist initially"
    );

    // Simultaneously: delete old.rs, add new.rs, update lib.rs
    std::fs::remove_file(dir.path().join("src/old.rs")).unwrap();
    let new_path = dir.path().join("src/new.rs");
    std::fs::write(&new_path, "pub fn new_fn() {}\n").unwrap();
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub mod new;\npub fn lib_fn() {}\n",
    )
    .unwrap();

    refresh_index(&db, &config).unwrap();

    let old_syms = db.search_symbols_exact("old_fn").unwrap();
    assert!(
        old_syms.is_empty(),
        "old_fn from deleted file should be gone"
    );

    let new_syms = db.search_symbols_exact("new_fn").unwrap();
    assert!(
        !new_syms.is_empty(),
        "new_fn from added file should be present"
    );
}

// ---------------------------------------------------------------------------
// Group 4: Edge Cases
// ---------------------------------------------------------------------------

#[test]
fn refresh_handles_empty_file_after_edit() {
    let (dir, db, config) = setup_refresh_project(&[("lib.rs", "pub fn doomed() {}\n")]);

    assert!(
        !db.search_symbols_exact("doomed").unwrap().is_empty(),
        "doomed should exist before emptying file"
    );

    // Rewrite to empty
    std::fs::write(dir.path().join("src/lib.rs"), "").unwrap();

    refresh_index(&db, &config).unwrap();

    let syms = db.search_symbols_exact("doomed").unwrap();
    assert!(syms.is_empty(), "doomed should be gone after file emptied");
}

#[test]
fn refresh_handles_syntax_error_gracefully() {
    let (dir, db, config) =
        setup_refresh_project(&[("lib.rs", "pub fn valid() {}\npub fn also_valid() {}\n")]);

    // Introduce syntax error (tree-sitter does partial parsing)
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn valid() {}\nfn broken( {}\n",
    )
    .unwrap();

    // Should not panic
    let result = refresh_index(&db, &config);
    assert!(result.is_ok(), "refresh should not crash on syntax errors");

    // valid() should still be parseable via tree-sitter partial parsing
    let syms = db.search_symbols_exact("valid").unwrap();
    assert!(
        !syms.is_empty(),
        "valid() should survive partial parsing after syntax error"
    );
}

#[test]
fn refresh_with_no_changes_is_noop() {
    let (_dir, db, config) = setup_refresh_project(&[("lib.rs", "pub fn stable_fn() {}\n")]);

    let refreshed = refresh_index(&db, &config).unwrap();
    assert_eq!(refreshed, 0, "refresh with no file changes should return 0");
}

// ---------------------------------------------------------------------------
// Group 5: Content Hash
// ---------------------------------------------------------------------------

#[test]
fn identical_content_rewrite_preserves_symbols() {
    let original = "pub fn idempotent() {}\n";
    let (dir, db, config) = setup_refresh_project(&[("lib.rs", original)]);

    // Write identical content back
    std::fs::write(dir.path().join("src/lib.rs"), original).unwrap();

    let refreshed = refresh_index(&db, &config).unwrap();
    // Content hash matches, so even if git reports it as modified,
    // collect_dirty_files should skip it.
    // Either way, the symbol must survive.
    let syms = db.search_symbols_exact("idempotent").unwrap();
    assert!(
        !syms.is_empty(),
        "symbol should survive identical-content rewrite, refreshed={refreshed}"
    );
}

#[test]
fn whitespace_change_triggers_reindex() {
    let (dir, db, config) = setup_refresh_project(&[("lib.rs", "pub fn ws_test() {}\n")]);

    // Add trailing newlines (different content hash)
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn ws_test() {}\n\n\n\n").unwrap();

    let refreshed = refresh_index(&db, &config).unwrap();
    // Git should report the file as modified, and the content hash
    // differs due to extra newlines, so refresh should process it.
    // The symbol should survive in either case.
    let syms = db.search_symbols_exact("ws_test").unwrap();
    assert!(
        !syms.is_empty(),
        "symbol should survive whitespace change, refreshed={refreshed}"
    );
}

#[test]
fn refresh_preserves_other_file_data_on_partial_refresh() {
    let (dir, db, config) = setup_refresh_project(&[
        (
            "lib.rs",
            "pub mod alpha;\npub mod beta;\npub fn lib_root() {}\n",
        ),
        ("alpha.rs", "pub fn alpha_fn(x: i32) {}\n"),
        ("beta.rs", "pub fn beta_fn(y: bool) {}\n"),
    ]);

    // Capture pre-refresh state of untouched files
    let alpha_before = db.search_symbols_exact("alpha_fn").unwrap();
    let beta_before = db.search_symbols_exact("beta_fn").unwrap();
    assert_eq!(alpha_before.len(), 1, "alpha_fn should exist");
    assert_eq!(beta_before.len(), 1, "beta_fn should exist");
    let alpha_line = alpha_before[0].line_start;
    let alpha_sig = alpha_before[0].signature.clone();
    let beta_line = beta_before[0].line_start;
    let beta_sig = beta_before[0].signature.clone();

    // Edit only lib.rs
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub mod alpha;\npub mod beta;\npub fn lib_root() { /* changed */ }\n",
    )
    .unwrap();

    refresh_index(&db, &config).unwrap();

    // Verify untouched files are completely preserved
    let alpha_after = db.search_symbols_exact("alpha_fn").unwrap();
    assert_eq!(
        alpha_after.len(),
        1,
        "alpha_fn should still exist after partial refresh"
    );
    assert_eq!(
        alpha_after[0].line_start, alpha_line,
        "alpha_fn line number should be unchanged"
    );
    assert_eq!(
        alpha_after[0].signature, alpha_sig,
        "alpha_fn signature should be unchanged"
    );

    let beta_after = db.search_symbols_exact("beta_fn").unwrap();
    assert_eq!(
        beta_after.len(),
        1,
        "beta_fn should still exist after partial refresh"
    );
    assert_eq!(
        beta_after[0].line_start, beta_line,
        "beta_fn line number should be unchanged"
    );
    assert_eq!(
        beta_after[0].signature, beta_sig,
        "beta_fn signature should be unchanged"
    );
}

// ---------------------------------------------------------------------------
// Bonus: Multi-step refresh scenarios
// ---------------------------------------------------------------------------

#[test]
fn double_refresh_is_stable() {
    let (dir, db, config) = setup_refresh_project(&[("lib.rs", "pub fn alpha() {}\n")]);

    // First edit + refresh
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn alpha() {}\npub fn beta() {}\n",
    )
    .unwrap();
    refresh_index(&db, &config).unwrap();
    git_commit(dir.path(), "add beta");

    // Second edit + refresh
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn alpha() {}\npub fn beta() {}\npub fn gamma() {}\n",
    )
    .unwrap();
    refresh_index(&db, &config).unwrap();

    let alpha = db.search_symbols_exact("alpha").unwrap();
    let beta = db.search_symbols_exact("beta").unwrap();
    let gamma = db.search_symbols_exact("gamma").unwrap();
    assert!(!alpha.is_empty(), "alpha should exist after two refreshes");
    assert!(!beta.is_empty(), "beta should exist after two refreshes");
    assert!(!gamma.is_empty(), "gamma should exist after two refreshes");
}

#[test]
fn refresh_add_then_remove_leaves_no_trace() {
    let (dir, db, config) = setup_refresh_project(&[("lib.rs", "pub fn permanent() {}\n")]);

    // Add a temporary symbol
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn permanent() {}\npub fn temporary() {}\n",
    )
    .unwrap();
    refresh_index(&db, &config).unwrap();

    assert!(
        !db.search_symbols_exact("temporary").unwrap().is_empty(),
        "temporary should exist after first refresh"
    );

    // Commit so git baseline advances, then remove the temporary symbol
    git_commit(dir.path(), "add temporary");

    std::fs::write(dir.path().join("src/lib.rs"), "pub fn permanent() {}\n").unwrap();
    refresh_index(&db, &config).unwrap();

    let syms = db.search_symbols_exact("temporary").unwrap();
    assert!(
        syms.is_empty(),
        "temporary should be completely gone after second refresh"
    );
    let syms = db.search_symbols_exact("permanent").unwrap();
    assert!(
        !syms.is_empty(),
        "permanent should survive add-then-remove cycle"
    );
}

// ---------------------------------------------------------------------------
// Group: cross-file refs and symbol-universe changes
// ---------------------------------------------------------------------------

/// Regression: when a previously-unknown symbol name is added to one file,
/// refs in other, *unmodified* files that mention that name must get
/// re-extracted. Before the fix this leaked silently — ~37% of `symbol_refs`
/// were missing on self-indexed repos.
#[test]
fn refresh_backfills_refs_in_non_dirty_files_when_universe_grows() {
    let (dir, db, config) = setup_refresh_project(&[
        // Caller file references a name that does not yet exist as a symbol.
        (
            "callers.rs",
            "pub fn caller() {\n    let _ = target();\n}\n",
        ),
        // Placeholder file that does NOT define `target` on the first pass.
        ("targets.rs", "pub fn placeholder() {}\n"),
    ]);

    // Baseline: target doesn't exist, so caller's body cannot have a ref
    // to it. Verify the starting state matches the bug scenario.
    let target_before = db.search_symbols_exact("target").unwrap();
    assert!(
        target_before.is_empty(),
        "target should not yet be in known_symbols"
    );
    let callers_of_target_before = db.get_callers_by_name("target", None, false).unwrap();
    assert!(
        callers_of_target_before.is_empty(),
        "no refs to non-existent target yet"
    );

    // Now introduce `target` in a *different* file (targets.rs becomes
    // dirty). callers.rs is not edited, so its file hash is unchanged and
    // it would be skipped by the old incremental path.
    git_commit(dir.path(), "baseline");
    std::fs::write(
        dir.path().join("src/targets.rs"),
        "pub fn placeholder() {}\npub fn target() {}\n",
    )
    .unwrap();

    refresh_index(&db, &config).unwrap();

    // The newly-resolvable ref must now exist. Before the fix this was
    // `callers_of_target.is_empty()`.
    let callers_of_target = db.get_callers_by_name("target", None, false).unwrap();
    assert_eq!(
        callers_of_target.len(),
        1,
        "caller should now resolve to target after universe grew: {callers_of_target:?}"
    );
    let (caller_name, caller_file) = &callers_of_target[0];
    assert_eq!(caller_name, "caller");
    assert_eq!(caller_file, "src/callers.rs");
}

/// Symmetric case: when a symbol is *removed*, existing refs in non-dirty
/// files that targeted it should not linger as stale rows. `delete_stale_refs`
/// already covers the target-deleted case, but the universe-change detector
/// also runs here — make sure the combination stays sane.
#[test]
fn refresh_drops_refs_when_universe_shrinks() {
    let (dir, db, config) = setup_refresh_project(&[
        (
            "callers.rs",
            "pub fn caller() {\n    let _ = target();\n}\n",
        ),
        ("targets.rs", "pub fn target() {}\n"),
    ]);

    // Baseline: ref is resolved.
    let before = db.get_callers_by_name("target", None, false).unwrap();
    assert_eq!(before.len(), 1, "baseline ref should exist: {before:?}");

    git_commit(dir.path(), "baseline");
    std::fs::write(
        dir.path().join("src/targets.rs"),
        "pub fn placeholder() {}\n",
    )
    .unwrap();

    refresh_index(&db, &config).unwrap();

    let after = db.get_callers_by_name("target", None, false).unwrap();
    assert!(
        after.is_empty(),
        "ref should be gone once target is removed: {after:?}"
    );
}
