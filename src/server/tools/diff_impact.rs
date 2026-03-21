use crate::db::Database;
use std::fmt::Write;
use std::path::Path;

#[derive(Debug, PartialEq, Eq)]
pub struct DiffHunk {
    pub file_path: String,
    pub new_start: i64,
    pub new_count: i64,
}

/// Parse unified diff output to extract file paths and changed line ranges.
#[must_use]
pub fn parse_diff(diff_output: &str) -> Vec<DiffHunk> {
    let mut hunks = Vec::new();
    let mut current_file: Option<String> = None;

    for line in diff_output.lines() {
        if let Some(rest) = line.strip_prefix("+++ b/") {
            current_file = Some(rest.to_string());
        } else if line.starts_with("+++ ") {
            // Handle +++ /dev/null (deleted files) — skip
            current_file = None;
        } else if line.starts_with("@@ ") {
            let Some(file) = &current_file else {
                continue;
            };
            if let Some(hunk) = parse_hunk_header(line, file) {
                hunks.push(hunk);
            }
        }
    }

    hunks
}

fn parse_hunk_header(line: &str, file_path: &str) -> Option<DiffHunk> {
    // Format: @@ -old_start[,old_count] +new_start[,new_count] @@
    let after_at = line.strip_prefix("@@ ")?;
    let end_at = after_at.find(" @@")?;
    let range_part = &after_at[..end_at];

    let plus_idx = range_part.find('+')?;
    let new_range = &range_part[plus_idx + 1..];

    let (new_start, new_count) = if let Some((s, c)) = new_range.split_once(',') {
        (s.parse::<i64>().ok()?, c.parse::<i64>().ok()?)
    } else {
        (new_range.parse::<i64>().ok()?, 1)
    };

    // A count of 0 means no lines in new file (pure deletion)
    if new_count == 0 {
        return None;
    }

    Some(DiffHunk {
        file_path: file_path.to_string(),
        new_start,
        new_count,
    })
}

/// Run `git diff` with `--unified=0` and return the raw output.
pub fn run_git_diff(
    repo_path: &Path,
    git_ref: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut cmd = std::process::Command::new("git");
    cmd.current_dir(repo_path).arg("diff").arg("--unified=0");

    if let Some(r) = git_ref {
        if r.contains("..") {
            cmd.arg(r);
        } else {
            cmd.arg(format!("{r}..HEAD"));
        }
    }

    let output = cmd.output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git diff failed: {stderr}").into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Analyze the impact of changes from a git diff.
pub fn handle_diff_impact(
    db: &Database,
    repo_path: &Path,
    git_ref: Option<&str>,
    changes_only: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let diff_output = run_git_diff(repo_path, git_ref)?;
    if diff_output.trim().is_empty() {
        return Ok("No changes detected. Check the git ref \
             (e.g., git_ref: \"HEAD~1..HEAD\" for last commit, \
             or omit for unstaged changes)."
            .to_string());
    }

    let hunks = parse_diff(&diff_output);
    if hunks.is_empty() {
        return Ok("No changed lines found in the diff.".to_string());
    }

    // Group hunks by file path into line ranges
    let mut file_ranges: std::collections::BTreeMap<String, Vec<(i64, i64)>> =
        std::collections::BTreeMap::new();
    for hunk in &hunks {
        let end = hunk.new_start + hunk.new_count - 1;
        file_ranges
            .entry(hunk.file_path.clone())
            .or_default()
            .push((hunk.new_start, end));
    }

    // Map changed lines to symbols
    let mut changed_symbols: Vec<(String, crate::db::StoredSymbol)> = Vec::new();
    for (file, ranges) in &file_ranges {
        let symbols = db.get_symbols_at_lines(file, ranges)?;
        for sym in symbols {
            changed_symbols.push((file.clone(), sym));
        }
    }

    let mut output = String::new();

    if changed_symbols.is_empty() {
        let _ = writeln!(output, "## Changed Symbols\n");
        let _ = writeln!(
            output,
            "No indexed symbols overlap the changed lines. \
             Changes may be in comments, whitespace, or between function definitions."
        );
        return Ok(output);
    }

    // Group symbols by file for display
    let _ = writeln!(output, "## Changed Symbols\n");
    let mut current_file = String::new();
    for (file, sym) in &changed_symbols {
        if *file != current_file {
            current_file.clone_from(file);
            let _ = writeln!(output, "### {file}");
        }
        let _ = writeln!(
            output,
            "- **{}** ({}, line {}-{})",
            sym.name, sym.kind, sym.line_start, sym.line_end
        );
    }

    if changes_only {
        return Ok(output);
    }

    // Run impact analysis for each changed symbol
    let mut impact_sections = Vec::new();
    for (_file, sym) in &changed_symbols {
        let dependents = db.impact_dependents(&sym.name)?;
        if !dependents.is_empty() {
            impact_sections.push((sym.name.clone(), dependents));
        }
    }

    if !impact_sections.is_empty() {
        let _ = writeln!(output, "\n### Downstream Impact\n");
        for (name, dependents) in &impact_sections {
            let _ = writeln!(output, "#### {name}");
            for dep in dependents {
                let _ = writeln!(
                    output,
                    "- {} ({}) — depth {}",
                    dep.name, dep.file_path, dep.depth
                );
            }
        }
    }

    render_test_coverage(db, &mut output, &changed_symbols);

    Ok(output)
}

fn render_test_coverage(
    db: &Database,
    output: &mut String,
    changed_symbols: &[(String, crate::db::StoredSymbol)],
) {
    let mut all_tests = Vec::new();
    let mut seen_tests = std::collections::HashSet::new();
    let mut untested_symbols = Vec::new();
    for (_file, sym) in changed_symbols {
        if let Ok(tests) = db.get_related_tests(&sym.name) {
            if tests.is_empty() {
                let is_test = sym
                    .attributes
                    .as_deref()
                    .is_some_and(|a| a.contains("test"));
                if !is_test && sym.kind == crate::indexer::parser::SymbolKind::Function {
                    untested_symbols.push(sym);
                }
            }
            for t in tests {
                if seen_tests.insert(t.name.clone()) {
                    all_tests.push(t);
                }
            }
        }
    }

    if !untested_symbols.is_empty() {
        let _ = writeln!(output, "\n### Untested Changes\n");
        let _ = writeln!(output, "These changed functions have no test coverage:\n");
        for sym in &untested_symbols {
            let _ = writeln!(
                output,
                "- **{}** ({}:{}-{})",
                sym.name, sym.file_path, sym.line_start, sym.line_end
            );
        }
    }

    if !all_tests.is_empty() {
        let _ = writeln!(output, "\n### Related Tests\n");
        for t in &all_tests {
            let _ = writeln!(
                output,
                "- **{}** ({}:{})",
                t.name, t.file_path, t.line_start
            );
        }
        let test_names: Vec<&str> = all_tests.iter().map(|t| t.name.as_str()).collect();
        let _ = writeln!(output, "\nSuggested: `cargo test {}`", test_names.join(" "));
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::indexer::parser::{Symbol, SymbolKind, Visibility};
    use crate::indexer::store::store_symbols;

    #[test]
    fn test_parse_diff_basic() {
        let diff = "\
diff --git a/src/db.rs b/src/db.rs
index abc..def 100644
--- a/src/db.rs
+++ b/src/db.rs
@@ -450,3 +450,5 @@ impl Database {
@@ -500,0 +502,2 @@ impl Database {
";
        let hunks = parse_diff(diff);
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].file_path, "src/db.rs");
        assert_eq!(hunks[0].new_start, 450);
        assert_eq!(hunks[0].new_count, 5);
        assert_eq!(hunks[1].file_path, "src/db.rs");
        assert_eq!(hunks[1].new_start, 502);
        assert_eq!(hunks[1].new_count, 2);
    }

    #[test]
    fn test_parse_diff_multiple_files() {
        let diff = "\
diff --git a/src/db.rs b/src/db.rs
--- a/src/db.rs
+++ b/src/db.rs
@@ -10,2 +10,4 @@ fn foo()
diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,1 +1,3 @@ fn main()
";
        let hunks = parse_diff(diff);
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].file_path, "src/db.rs");
        assert_eq!(hunks[1].file_path, "src/main.rs");
    }

    #[test]
    fn test_parse_diff_empty() {
        let hunks = parse_diff("");
        assert!(hunks.is_empty());
    }

    #[test]
    fn test_parse_diff_pure_deletion() {
        // +0,0 means no lines in new file — should be skipped
        let diff = "\
diff --git a/src/db.rs b/src/db.rs
--- a/src/db.rs
+++ b/src/db.rs
@@ -10,3 +10,0 @@ fn foo()
";
        let hunks = parse_diff(diff);
        assert!(hunks.is_empty());
    }

    #[test]
    fn test_parse_diff_single_line_hunk() {
        // No comma means count=1
        let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -5 +5 @@ fn bar()
";
        let hunks = parse_diff(diff);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].new_start, 5);
        assert_eq!(hunks[0].new_count, 1);
    }

    #[test]
    fn test_handle_diff_impact_with_symbols() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "target_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 10,
                    line_end: 20,
                    signature: "pub fn target_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "caller_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 25,
                    line_end: 35,
                    signature: "pub fn caller_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        // Create a reference from caller_fn -> target_fn
        let target_id = db
            .get_symbol_id("target_fn", "src/lib.rs")
            .unwrap()
            .unwrap();
        let caller_id = db
            .get_symbol_id("caller_fn", "src/lib.rs")
            .unwrap()
            .unwrap();
        db.insert_symbol_ref(caller_id, target_id, "call", "high")
            .unwrap();

        // Simulate: lines 12-15 changed in src/lib.rs — overlaps target_fn
        let symbols = db.get_symbols_at_lines("src/lib.rs", &[(12, 15)]).unwrap();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "target_fn");

        // Verify impact shows caller_fn as dependent
        let dependents = db.impact_dependents("target_fn").unwrap();
        assert_eq!(dependents.len(), 1);
        assert_eq!(dependents[0].name, "caller_fn");
    }

    #[test]
    fn test_diff_impact_includes_related_tests() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "target_fn".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 10,
                    line_end: 20,
                    signature: "pub fn target_fn()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "test_target".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    file_path: "src/lib.rs".into(),
                    line_start: 25,
                    line_end: 35,
                    signature: "fn test_target()".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: Some("test".into()),
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        let target_id = db
            .get_symbol_id("target_fn", "src/lib.rs")
            .unwrap()
            .unwrap();
        let test_id = db
            .get_symbol_id("test_target", "src/lib.rs")
            .unwrap()
            .unwrap();
        db.insert_symbol_ref(test_id, target_id, "call", "high")
            .unwrap();

        // Verify get_related_tests finds the test
        let tests = db.get_related_tests("target_fn").unwrap();
        assert_eq!(tests.len(), 1);
        assert_eq!(tests[0].name, "test_target");
    }
}
