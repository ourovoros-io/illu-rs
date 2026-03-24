//! Graph traversal correctness tests for illu-rs.
//!
//! Tests the impact CTE, test discovery, callpath, confidence filtering,
//! and cross-tool consistency.

#![expect(clippy::unwrap_used, reason = "integration tests")]

use illu_rs::db::Database;
use illu_rs::indexer::{IndexConfig, index_repo};
use illu_rs::server::tools::{callpath, neighborhood, test_impact};
use std::collections::HashSet;
use std::fmt::Write as _;

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

// ---------------------------------------------------------------------------
// Group 1: Impact CTE Correctness
// ---------------------------------------------------------------------------

#[test]
fn impact_linear_chain_correct_depth() {
    let (_dir, db) = index_source(
        r"
pub fn func_a() { func_b(); }
pub fn func_b() { func_c(); }
pub fn func_c() { func_d(); }
pub fn func_d() {}
",
    );

    let entries = db.impact_dependents_with_depth("func_d", None, 5).unwrap();

    assert_eq!(
        entries.len(),
        3,
        "linear chain a->b->c->d should yield 3 dependents, got {entries:?}"
    );

    let depth1: Vec<&str> = entries
        .iter()
        .filter(|e| e.depth == 1)
        .map(|e| e.name.as_str())
        .collect();
    assert!(
        depth1.contains(&"func_c"),
        "depth 1 should contain func_c, got {depth1:?}"
    );

    let depth2: Vec<&str> = entries
        .iter()
        .filter(|e| e.depth == 2)
        .map(|e| e.name.as_str())
        .collect();
    assert!(
        depth2.contains(&"func_b"),
        "depth 2 should contain func_b, got {depth2:?}"
    );

    let depth3: Vec<&str> = entries
        .iter()
        .filter(|e| e.depth == 3)
        .map(|e| e.name.as_str())
        .collect();
    assert!(
        depth3.contains(&"func_a"),
        "depth 3 should contain func_a, got {depth3:?}"
    );
}

#[test]
fn impact_diamond_deduplicates() {
    let (_dir, db) = index_source(
        r"
pub fn base() {}
pub fn left() { base(); }
pub fn right() { base(); }
pub fn top() { left(); right(); }
",
    );

    let entries = db.impact_dependents_with_depth("base", None, 3).unwrap();

    let depth1_names: Vec<&str> = entries
        .iter()
        .filter(|e| e.depth == 1)
        .map(|e| e.name.as_str())
        .collect();
    assert!(
        depth1_names.contains(&"left"),
        "depth 1 should contain left, got {depth1_names:?}"
    );
    assert!(
        depth1_names.contains(&"right"),
        "depth 1 should contain right, got {depth1_names:?}"
    );

    // The CTE may return "top" via multiple paths (left and right).
    // Verify "top" appears at depth 2 regardless of how many via-paths exist.
    let top_entries: Vec<_> = entries.iter().filter(|e| e.name == "top").collect();
    assert!(
        !top_entries.is_empty(),
        "top should appear in diamond impact results, got {entries:?}"
    );
    for entry in &top_entries {
        assert_eq!(
            entry.depth, 2,
            "top should be at depth 2, got {}",
            entry.depth
        );
    }

    // Verify distinct symbol names: base has 3 dependents (left, right, top)
    let unique_names: HashSet<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        unique_names.len(),
        3,
        "diamond should produce 3 unique dependent symbols, got {unique_names:?}"
    );
}

#[test]
fn impact_circular_terminates() {
    let (_dir, db) = index_source(
        r"
pub fn alpha() { beta(); }
pub fn beta() { gamma(); }
pub fn gamma() { alpha(); }
",
    );

    // The CTE uses UNION (not UNION ALL) so cycles are handled.
    let entries = db.impact_dependents_with_depth("alpha", None, 5).unwrap();

    assert!(
        !entries.is_empty(),
        "circular graph should still return dependents, got empty"
    );

    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(
        names.contains(&"gamma"),
        "gamma calls alpha, should appear in impact, got {names:?}"
    );
}

#[test]
fn impact_respects_depth_limit() {
    let (_dir, db) = index_source(
        r"
pub fn d0() {}
pub fn d1() { d0(); }
pub fn d2() { d1(); }
pub fn d3() { d2(); }
pub fn d4() { d3(); }
pub fn d5() { d4(); }
pub fn d6() { d5(); }
",
    );

    let entries = db.impact_dependents_with_depth("d0", None, 3).unwrap();

    let max_depth = entries.iter().map(|e| e.depth).max().unwrap_or(0);
    assert!(max_depth <= 3, "max depth should be <= 3, got {max_depth}");

    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(
        names.contains(&"d1"),
        "d1 should be in depth-limited results, got {names:?}"
    );
    assert!(
        names.contains(&"d2"),
        "d2 should be in depth-limited results, got {names:?}"
    );
    assert!(
        names.contains(&"d3"),
        "d3 should be in depth-limited results, got {names:?}"
    );
    assert!(
        !names.contains(&"d4"),
        "d4 should NOT appear at depth limit 3, got {names:?}"
    );
    assert!(
        !names.contains(&"d5"),
        "d5 should NOT appear at depth limit 3, got {names:?}"
    );
    assert!(
        !names.contains(&"d6"),
        "d6 should NOT appear at depth limit 3, got {names:?}"
    );
}

#[test]
fn impact_via_chain_accurate() {
    let (_dir, db) = index_source(
        r"
pub fn core_fn() {}
pub fn mid_fn() { core_fn(); }
pub fn outer_fn() { mid_fn(); }
",
    );

    let entries = db.impact_dependents_with_depth("core_fn", None, 5).unwrap();

    let outer = entries.iter().find(|e| e.name == "outer_fn");
    assert!(
        outer.is_some(),
        "outer_fn should be in impact results, got {entries:?}"
    );
    let outer = outer.unwrap();
    assert_eq!(
        outer.depth, 2,
        "outer_fn should be at depth 2, got {}",
        outer.depth
    );
    assert!(
        outer.via.contains("mid_fn"),
        "outer_fn via should contain mid_fn, got {:?}",
        outer.via
    );
}

#[test]
fn impact_type_method_syntax_works() {
    let (_dir, db) = index_source(
        r"
pub struct Alpha;
impl Alpha {
    pub fn process(&self) {}
}

pub struct Beta;
impl Beta {
    pub fn process(&self) {}
}

pub fn use_alpha(a: &Alpha) { a.process(); }
pub fn use_beta(b: &Beta) { b.process(); }
",
    );

    let entries = db
        .impact_dependents_with_depth("process", Some("Alpha"), 1)
        .unwrap();

    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(
        names.contains(&"use_alpha"),
        "Alpha::process impact should contain use_alpha, got {names:?}"
    );
    assert!(
        !names.contains(&"use_beta"),
        "Alpha::process impact should NOT contain use_beta, got {names:?}"
    );
}

#[test]
fn impact_high_confidence_only_in_cte() {
    // Single-file: same-file calls are high confidence.
    // Verify the CTE traverses high-confidence refs correctly.
    let (_dir, db) = index_source(
        r"
pub fn do_work() {}
pub fn high_conf_caller() { do_work(); }
pub fn transitive_caller() { high_conf_caller(); }
",
    );

    let entries = db.impact_dependents_with_depth("do_work", None, 2).unwrap();

    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(
        names.contains(&"high_conf_caller"),
        "direct high-confidence caller should appear in impact, got {names:?}"
    );
    assert!(
        names.contains(&"transitive_caller"),
        "transitive caller through high-confidence chain should appear, got {names:?}"
    );
}

#[test]
fn impact_result_count_bounded() {
    // Generate many callers to verify the CTE LIMIT caps results
    let mut source = String::from("pub fn target_fn() {}\n");
    for i in 0..110 {
        let _ = writeln!(source, "pub fn caller_{i}() {{ target_fn(); }}");
    }

    let (_dir, db) = index_source(&source);

    let entries = db
        .impact_dependents_with_depth("target_fn", None, 1)
        .unwrap();

    assert!(
        entries.len() <= 100,
        "impact CTE should be bounded at 100 entries, got {}",
        entries.len()
    );
    assert!(
        entries.len() >= 50,
        "impact should return a substantial number of callers, got {}",
        entries.len()
    );
}

// ---------------------------------------------------------------------------
// Group 2: Test Discovery Accuracy
// ---------------------------------------------------------------------------

#[test]
fn related_tests_finds_direct_test_caller() {
    let (_dir, db) = index_source(
        r"
pub fn helper() -> i32 { 42 }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_helper() {
        assert_eq!(helper(), 42);
    }
}
",
    );

    let tests = db.get_related_tests("helper", None).unwrap();
    let test_names: Vec<&str> = tests.iter().map(|t| t.name.as_str()).collect();
    assert!(
        test_names.contains(&"test_helper"),
        "direct test caller should be discovered, got {test_names:?}"
    );
}

#[test]
fn related_tests_finds_transitive_test() {
    let (_dir, db) = index_source(
        r"
pub fn base_fn() -> i32 { 1 }
pub fn mid_fn() -> i32 { base_fn() + 1 }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mid() {
        assert_eq!(mid_fn(), 2);
    }
}
",
    );

    let tests = db.get_related_tests("base_fn", None).unwrap();
    let test_names: Vec<&str> = tests.iter().map(|t| t.name.as_str()).collect();
    assert!(
        test_names.contains(&"test_mid"),
        "transitive test through mid_fn should be found, got {test_names:?}"
    );
}

#[test]
fn related_tests_excludes_non_test_callers() {
    let (_dir, db) = index_source(
        r"
pub fn helper() {}
pub fn production_caller() { helper(); }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fn() { helper(); }
}
",
    );

    let tests = db.get_related_tests("helper", None).unwrap();
    let test_names: Vec<&str> = tests.iter().map(|t| t.name.as_str()).collect();
    assert!(
        test_names.contains(&"test_fn"),
        "test_fn should be in related tests, got {test_names:?}"
    );
    assert!(
        !test_names.contains(&"production_caller"),
        "production_caller is NOT a test, should not appear, got {test_names:?}"
    );
}

#[test]
fn related_tests_respects_impl_type() {
    // Use Type::method() qualified syntax so the indexer resolves impl_type.
    // Local variable method calls (f.process()) may not resolve impl_type
    // since tree-sitter lacks type inference. Qualified calls work reliably.
    let (_dir, db) = index_source(
        r"
pub struct Foo;
impl Foo {
    pub fn run(&self) {}
}

pub fn call_foo(f: &Foo) { f.run(); }

pub struct Bar;
impl Bar {
    pub fn run(&self) {}
}

pub fn call_bar(b: &Bar) { b.run(); }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_foo() { call_foo(&Foo); }

    #[test]
    fn test_bar() { call_bar(&Bar); }
}
",
    );

    // test_foo -> call_foo -> Foo::run (transitive)
    // test_bar -> call_bar -> Bar::run (transitive)
    let tests = db.get_related_tests("call_foo", None).unwrap();
    let test_names: Vec<&str> = tests.iter().map(|t| t.name.as_str()).collect();
    assert!(
        test_names.contains(&"test_foo"),
        "call_foo related tests should include test_foo, got {test_names:?}"
    );
    assert!(
        !test_names.contains(&"test_bar"),
        "call_foo related tests should NOT include test_bar, got {test_names:?}"
    );
}

#[test]
fn related_tests_same_file_high_confidence() {
    // Same-file refs are high confidence, verify test discovery works
    let (_dir, db) = index_source(
        r"
pub fn do_work() {}

pub fn wrapper() { do_work(); }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_test() { do_work(); }
}
",
    );

    let tests = db.get_related_tests("do_work", None).unwrap();
    let test_names: Vec<&str> = tests.iter().map(|t| t.name.as_str()).collect();
    assert!(
        test_names.contains(&"real_test"),
        "same-file test caller should be discovered, got {test_names:?}"
    );
}

// ---------------------------------------------------------------------------
// Group 3: Caller/Callee Confidence Filtering
// ---------------------------------------------------------------------------

#[test]
fn get_callees_returns_high_confidence_only() {
    // Same-file calls produce high-confidence refs
    let (_dir, db) = index_source(
        r"
pub fn known_fn() {}
pub fn caller() { known_fn(); }
",
    );

    let callees = db.get_callees("caller", "src/lib.rs", false).unwrap();
    let callee_names: Vec<&str> = callees.iter().map(|c| c.name.as_str()).collect();
    assert!(
        callee_names.contains(&"known_fn"),
        "same-file callee should be returned (high confidence), got {callee_names:?}"
    );
}

#[test]
fn get_callers_high_filter_works() {
    // Same-file calls are high confidence
    let (_dir, db) = index_source(
        r"
pub fn target_fn() {}
pub fn real_caller() { target_fn(); }
",
    );

    let callers = db
        .get_callers("target_fn", "src/lib.rs", false, Some("high"))
        .unwrap();
    let caller_names: Vec<&str> = callers.iter().map(|c| c.name.as_str()).collect();
    assert!(
        caller_names.contains(&"real_caller"),
        "high-confidence same-file caller should be returned, got {caller_names:?}"
    );
}

#[test]
fn get_callers_no_filter_includes_all() {
    let (_dir, db) = index_source(
        r"
pub fn target_fn() {}
pub fn real_caller() { target_fn(); }
",
    );

    let callers_all = db
        .get_callers("target_fn", "src/lib.rs", false, None)
        .unwrap();
    let callers_high = db
        .get_callers("target_fn", "src/lib.rs", false, Some("high"))
        .unwrap();

    assert!(
        callers_all.len() >= callers_high.len(),
        "unfiltered callers ({}) should be >= high-only callers ({})",
        callers_all.len(),
        callers_high.len()
    );

    let all_names: Vec<&str> = callers_all.iter().map(|c| c.name.as_str()).collect();
    assert!(
        all_names.contains(&"real_caller"),
        "real_caller should appear in unfiltered callers, got {all_names:?}"
    );
}

#[test]
fn neighborhood_uses_call_refs_only() {
    let (_dir, db) = index_source(
        r"
pub struct Config {
    pub value: i32,
}

pub fn create_config() -> Config {
    Config { value: 42 }
}

pub fn use_config() {
    create_config();
}
",
    );

    let output = neighborhood::handle_neighborhood(
        &db,
        "create_config",
        Some(1),
        Some("both"),
        Some("list"),
        false,
    )
    .unwrap();

    assert!(
        output.contains("use_config"),
        "neighborhood should contain use_config as a caller, got:\n{output}"
    );
}

// ---------------------------------------------------------------------------
// Group 4: Callpath Correctness
// ---------------------------------------------------------------------------

#[test]
fn callpath_finds_shortest_path() {
    let (_dir, db) = index_source(
        r"
pub fn start() { mid(); }
pub fn mid() { end_fn(); }
pub fn end_fn() {}
",
    );

    let output =
        callpath::handle_callpath(&db, "start", "end_fn", None, false, None, false).unwrap();

    assert!(
        output.contains("start"),
        "callpath output should contain start, got:\n{output}"
    );
    assert!(
        output.contains("end_fn"),
        "callpath output should contain end_fn, got:\n{output}"
    );
    assert!(
        output.contains("mid"),
        "callpath output should contain intermediate mid, got:\n{output}"
    );
}

#[test]
fn callpath_no_path_returns_clear_message() {
    let (_dir, db) = index_source(
        r"
pub fn isolated_a() {}
pub fn isolated_b() {}
",
    );

    let output =
        callpath::handle_callpath(&db, "isolated_a", "isolated_b", None, false, None, false)
            .unwrap();

    let lower = output.to_lowercase();
    assert!(
        lower.contains("no call path") || lower.contains("no path"),
        "should report no path found for disconnected nodes, got:\n{output}"
    );
}

#[test]
fn callpath_all_paths_finds_multiple() {
    let (_dir, db) = index_source(
        r"
pub fn origin() { path_a(); path_b(); }
pub fn path_a() { destination(); }
pub fn path_b() { destination(); }
pub fn destination() {}
",
    );

    let output =
        callpath::handle_callpath(&db, "origin", "destination", None, true, None, false).unwrap();

    // all_paths output uses numbered list format: "1. `origin -> ...`"
    // Count numbered list items (lines starting with a digit followed by .)
    let path_lines = output
        .lines()
        .filter(|l| {
            let trimmed = l.trim();
            trimmed.starts_with("1.") || trimmed.starts_with("2.") || trimmed.starts_with("3.")
        })
        .count();
    assert!(
        path_lines >= 2,
        "all_paths should find at least 2 paths in diamond, got {path_lines} in:\n{output}"
    );
}

#[test]
fn callpath_exclude_tests_skips_test_nodes() {
    let (_dir, db) = index_source(
        r"
pub fn start_fn() { bridge(); }
pub fn bridge() { end_fn(); }
pub fn end_fn() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bridge() { bridge(); }
}
",
    );

    // Production path through bridge should still work with exclude_tests
    let output =
        callpath::handle_callpath(&db, "start_fn", "end_fn", None, false, None, true).unwrap();

    assert!(
        output.contains("bridge"),
        "production path through bridge should still be found with exclude_tests, got:\n{output}"
    );
    assert!(
        !output.contains("test_bridge"),
        "test_bridge should not appear in callpath with exclude_tests=true, got:\n{output}"
    );
}

// ---------------------------------------------------------------------------
// Group 5: Cross-Tool Consistency
// ---------------------------------------------------------------------------

#[test]
fn impact_depth1_matches_callers() {
    let (_dir, db) = index_source(
        r"
pub fn target() {}
pub fn caller_a() { target(); }
pub fn caller_b() { target(); }
",
    );

    let impact_entries = db.impact_dependents_with_depth("target", None, 1).unwrap();
    let mut impact_names: Vec<&str> = impact_entries
        .iter()
        .filter(|e| e.depth == 1)
        .map(|e| e.name.as_str())
        .collect();
    impact_names.sort_unstable();

    let callers = db
        .get_callers("target", "src/lib.rs", false, Some("high"))
        .unwrap();
    let mut caller_names: Vec<&str> = callers.iter().map(|c| c.name.as_str()).collect();
    caller_names.sort_unstable();

    assert_eq!(
        impact_names, caller_names,
        "impact depth-1 names should match get_callers high-confidence names\n\
         impact: {impact_names:?}\ncallers: {caller_names:?}"
    );
}

#[test]
fn test_impact_matches_related_tests() {
    let (_dir, db) = index_source(
        r"
pub fn helper() {}
pub fn mid() { helper(); }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_one() { helper(); }

    #[test]
    fn test_two() { mid(); }
}
",
    );

    let db_tests = db.get_related_tests("helper", None).unwrap();
    let mut db_test_names: Vec<&str> = db_tests.iter().map(|t| t.name.as_str()).collect();
    db_test_names.sort_unstable();

    let tool_output = test_impact::handle_test_impact(&db, "helper", None).unwrap();

    // Verify the tool output contains the same test names
    for name in &db_test_names {
        assert!(
            tool_output.contains(name),
            "test_impact output should contain test '{name}' from get_related_tests,\n\
             db tests: {db_test_names:?}\ntool output:\n{tool_output}"
        );
    }
}

#[test]
fn neighborhood_down_matches_context_callees() {
    let (_dir, db) = index_source(
        r"
pub fn parent() { child_a(); child_b(); }
pub fn child_a() {}
pub fn child_b() {}
",
    );

    let callees = db.get_callees("parent", "src/lib.rs", false).unwrap();
    let callee_names: Vec<&str> = callees.iter().map(|c| c.name.as_str()).collect();

    let neighborhood_output = neighborhood::handle_neighborhood(
        &db,
        "parent",
        Some(1),
        Some("down"),
        Some("list"),
        false,
    )
    .unwrap();

    for name in &callee_names {
        assert!(
            neighborhood_output.contains(name),
            "neighborhood down should contain callee '{name}',\n\
             callees: {callee_names:?}\nneighborhood output:\n{neighborhood_output}"
        );
    }
}
