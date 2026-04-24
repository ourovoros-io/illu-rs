use std::fmt::Write;
use std::path::Path;

use super::diff_impact;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GateStatus {
    Pass,
    Warn,
    Blocked,
}

impl GateStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Warn => "WARN",
            Self::Blocked => "BLOCKED",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct QualityGateRequest<'a> {
    pub(crate) task: &'a str,
    pub(crate) plan: &'a str,
    pub(crate) docs_verified: &'a [String],
    pub(crate) impact_checked: &'a [String],
    pub(crate) tests_run: &'a [String],
    pub(crate) performance_evidence: Option<&'a str>,
    pub(crate) safety_notes: Option<&'a str>,
}

struct QualityGateEvidence<'a> {
    task: &'a str,
    plan: &'a str,
    docs_verified: &'a [String],
    impact_checked: &'a [String],
    tests_run: &'a [String],
    performance_evidence: Option<&'a str>,
    safety_notes: Option<&'a str>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct DiffFindings {
    rust_files: Vec<String>,
    public_edits: Vec<String>,
    unwraps: Vec<String>,
    unsafe_uses: Vec<String>,
}

pub(crate) fn handle_quality_gate(
    repo_path: &Path,
    request: QualityGateRequest<'_>,
    git_ref: Option<&str>,
) -> Result<String, crate::IlluError> {
    let diff = diff_impact::run_git_diff(repo_path, git_ref)?;
    let evidence = QualityGateEvidence {
        task: request.task,
        plan: request.plan,
        docs_verified: request.docs_verified,
        impact_checked: request.impact_checked,
        tests_run: request.tests_run,
        performance_evidence: request.performance_evidence,
        safety_notes: request.safety_notes,
    };
    Ok(analyze_quality_gate_with_repo(
        &diff,
        &evidence,
        Some(repo_path),
    ))
}

#[cfg(test)]
fn analyze_quality_gate(diff: &str, evidence: &QualityGateEvidence<'_>) -> String {
    analyze_quality_gate_with_repo(diff, evidence, None)
}

fn analyze_quality_gate_with_repo(
    diff: &str,
    evidence: &QualityGateEvidence<'_>,
    repo_path: Option<&Path>,
) -> String {
    let findings = analyze_diff(diff, repo_path);
    let rust_diff = !findings.rust_files.is_empty();
    let mut blockers = Vec::new();
    let mut warnings = Vec::new();

    if rust_diff {
        if !has_text_evidence(evidence.plan) {
            blockers.push("Missing design plan for Rust diff.".to_string());
        }
        if !has_list_evidence(evidence.docs_verified) {
            blockers.push(
                "Missing docs verification. Include std_docs/docs/context evidence for the APIs used."
                    .to_string(),
            );
        }
        if !has_list_evidence(evidence.tests_run) {
            blockers.push("Missing test evidence. Record the exact commands run.".to_string());
        }
    }

    if !findings.unwraps.is_empty() {
        blockers.push(format!(
            "New non-test `unwrap(` detected: {}",
            findings.unwraps.join("; ")
        ));
    }

    if !findings.unsafe_uses.is_empty() && !has_optional_evidence(evidence.safety_notes) {
        blockers.push(format!(
            "`unsafe` appears in the diff without safety notes: {}",
            findings.unsafe_uses.join("; ")
        ));
    }

    if claims_performance(evidence.task, evidence.plan)
        && !has_optional_evidence(evidence.performance_evidence)
    {
        blockers.push(
            "Task or plan claims speed/latency/performance work without performance_evidence."
                .to_string(),
        );
    }

    if !findings.public_edits.is_empty() && !has_list_evidence(evidence.impact_checked) {
        warnings.push(format!(
            "Public API-looking edits found without impact evidence: {}",
            findings.public_edits.join("; ")
        ));
    }

    if diff.trim().is_empty() {
        warnings.push("No git diff was found; gate only checked supplied evidence.".to_string());
    }

    let status = if !blockers.is_empty() {
        GateStatus::Blocked
    } else if !warnings.is_empty() {
        GateStatus::Warn
    } else {
        GateStatus::Pass
    };

    render_report(status, &findings, &blockers, &warnings, evidence)
}

fn render_report(
    status: GateStatus,
    findings: &DiffFindings,
    blockers: &[String],
    warnings: &[String],
    evidence: &QualityGateEvidence<'_>,
) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "# Quality Gate: {}\n", status.as_str());
    let _ = writeln!(output, "## Diff Checks\n");
    let _ = writeln!(
        output,
        "- Rust files changed: {}",
        findings.rust_files.len()
    );
    let _ = writeln!(
        output,
        "- Public API-looking edits: {}",
        findings.public_edits.len()
    );
    let _ = writeln!(output, "- New non-test unwraps: {}", findings.unwraps.len());
    let _ = writeln!(output, "- Unsafe additions: {}", findings.unsafe_uses.len());

    if !blockers.is_empty() {
        let _ = writeln!(output, "\n## Blocking Issues\n");
        for blocker in blockers {
            let _ = writeln!(output, "- {blocker}");
        }
    }

    if !warnings.is_empty() {
        let _ = writeln!(output, "\n## Warnings\n");
        for warning in warnings {
            let _ = writeln!(output, "- {warning}");
        }
    }

    let _ = writeln!(output, "\n## Evidence Received\n");
    let _ = writeln!(
        output,
        "- Plan: {}",
        if has_text_evidence(evidence.plan) {
            "present"
        } else {
            "missing"
        }
    );
    let _ = writeln!(
        output,
        "- Docs verified: {}",
        format_list_status(evidence.docs_verified)
    );
    let _ = writeln!(
        output,
        "- Impact checked: {}",
        format_list_status(evidence.impact_checked)
    );
    let _ = writeln!(
        output,
        "- Tests run: {}",
        format_list_status(evidence.tests_run)
    );
    let _ = writeln!(
        output,
        "- Performance evidence: {}",
        if has_optional_evidence(evidence.performance_evidence) {
            "present"
        } else {
            "not supplied"
        }
    );
    let _ = writeln!(
        output,
        "- Safety notes: {}",
        if has_optional_evidence(evidence.safety_notes) {
            "present"
        } else {
            "not supplied"
        }
    );

    output
}

fn analyze_diff(diff: &str, repo_path: Option<&Path>) -> DiffFindings {
    let mut findings = DiffFindings::default();
    let mut current_file: Option<String> = None;
    let mut test_context = RustTestContext::default();
    let mut new_line_number: Option<usize> = None;

    for line in diff.lines() {
        if let Some((old_file, new_file)) = parse_diff_header(line) {
            let file = if new_file == "/dev/null" {
                old_file
            } else {
                new_file
            };
            record_current_file(&mut current_file, &mut findings, file);
            test_context = RustTestContext::default();
            new_line_number = None;
            continue;
        }

        if let Some(file) = parse_diff_file_marker(line) {
            record_current_file(&mut current_file, &mut findings, file);
            test_context = RustTestContext::default();
            new_line_number = None;
            continue;
        }

        if let Some(start) = parse_hunk_new_start(line) {
            new_line_number = Some(start);
            test_context = RustTestContext::default();
            continue;
        }

        let Some(file) = current_file.as_deref() else {
            continue;
        };
        if !is_rust_file(file) {
            continue;
        }

        match diff_source_line(line) {
            Some(DiffSourceLine::Added(added)) => {
                let added_line_number = new_line_number;
                inspect_added_line(
                    file,
                    added,
                    is_test_code(file, &test_context, repo_path, added_line_number),
                    &mut findings,
                );
                test_context.observe(added);
                increment_new_line(&mut new_line_number);
            }
            Some(DiffSourceLine::Removed(removed)) if is_public_api_line(removed) => findings
                .public_edits
                .push(format!("{}: {}", file, summarize_line(removed))),
            Some(DiffSourceLine::Context(context)) => {
                test_context.observe(context);
                increment_new_line(&mut new_line_number);
            }
            Some(DiffSourceLine::Removed(_)) | None => {}
        }
    }

    findings.rust_files.sort();
    findings.rust_files.dedup();
    findings
}

fn increment_new_line(line_number: &mut Option<usize>) {
    if let Some(line) = line_number {
        *line += 1;
    }
}

enum DiffSourceLine<'a> {
    Added(&'a str),
    Removed(&'a str),
    Context(&'a str),
}

fn diff_source_line(line: &str) -> Option<DiffSourceLine<'_>> {
    if line.starts_with("+++") || line.starts_with("---") {
        return None;
    }
    if let Some(added) = line.strip_prefix('+') {
        return Some(DiffSourceLine::Added(added));
    }
    if let Some(removed) = line.strip_prefix('-') {
        return Some(DiffSourceLine::Removed(removed));
    }
    line.strip_prefix(' ').map(DiffSourceLine::Context)
}

#[derive(Default)]
struct RustTestContext {
    brace_depth: usize,
    pending_cfg_test: bool,
    test_module_depth: Option<usize>,
}

impl RustTestContext {
    fn observe(&mut self, line: &str) {
        let trimmed = line.trim_start();
        if trimmed.starts_with("#[cfg(test)]") {
            self.pending_cfg_test = true;
        }

        let opens_test_module = self.pending_cfg_test && is_mod_tests_line(trimmed);
        let previous_depth = self.brace_depth;
        self.update_brace_depth(line);

        if opens_test_module {
            self.test_module_depth = Some(previous_depth);
            self.pending_cfg_test = false;
        } else if !trimmed.starts_with("#[") && !trimmed.is_empty() {
            self.pending_cfg_test = false;
        }

        if let Some(depth) = self.test_module_depth
            && self.brace_depth <= depth
        {
            self.test_module_depth = None;
        }
    }

    fn in_test_code(&self, file: &str) -> bool {
        is_test_file(file) || self.test_module_depth.is_some()
    }

    fn update_brace_depth(&mut self, line: &str) {
        for byte in line.bytes() {
            match byte {
                b'{' => self.brace_depth = self.brace_depth.saturating_add(1),
                b'}' => self.brace_depth = self.brace_depth.saturating_sub(1),
                _ => {}
            }
        }
    }
}

fn record_current_file(current_file: &mut Option<String>, findings: &mut DiffFindings, file: &str) {
    if file == "/dev/null" {
        return;
    }

    *current_file = Some(file.to_string());
    if is_rust_file(file) && !findings.rust_files.iter().any(|seen| seen == file) {
        findings.rust_files.push(file.to_string());
    }
}

fn inspect_added_line(file: &str, line: &str, in_test_code: bool, findings: &mut DiffFindings) {
    if is_public_api_line(line) {
        findings
            .public_edits
            .push(format!("{}: {}", file, summarize_line(line)));
    }
    if !in_test_code && line.contains("unwrap(") {
        findings
            .unwraps
            .push(format!("{}: {}", file, summarize_line(line)));
    }
    if is_unsafe_code_line(line) {
        findings
            .unsafe_uses
            .push(format!("{}: {}", file, summarize_line(line)));
    }
}

fn parse_diff_header(line: &str) -> Option<(&str, &str)> {
    let mut paths = line.strip_prefix("diff --git ")?.split_whitespace();
    let old_file = paths.next()?.strip_prefix("a/")?;
    let new_file = paths.next()?.strip_prefix("b/")?;
    Some((old_file, new_file))
}

fn parse_diff_file_marker(line: &str) -> Option<&str> {
    line.strip_prefix("+++ b/")
        .or_else(|| line.strip_prefix("--- a/"))
        .or_else(|| line.strip_prefix("+++ /dev/null").map(|_| "/dev/null"))
        .or_else(|| line.strip_prefix("--- /dev/null").map(|_| "/dev/null"))
}

fn parse_hunk_new_start(line: &str) -> Option<usize> {
    if !line.starts_with("@@") {
        return None;
    }
    line.split_whitespace()
        .find_map(|part| part.strip_prefix('+'))
        .and_then(|range| range.split(',').next())
        .and_then(|start| start.parse().ok())
}

fn is_rust_file(file: &str) -> bool {
    std::path::Path::new(file)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))
}

fn is_test_file(file: &str) -> bool {
    file.starts_with("tests/")
        || file.contains("/tests/")
        || file.starts_with("benches/")
        || file.ends_with("_test.rs")
        || file.ends_with("_tests.rs")
}

fn is_mod_tests_line(line: &str) -> bool {
    let trimmed = line.trim_start_matches("pub(crate) ").trim_start();
    trimmed.starts_with("mod tests")
}

fn is_test_code(
    file: &str,
    context: &RustTestContext,
    repo_path: Option<&Path>,
    line_number: Option<usize>,
) -> bool {
    context.in_test_code(file)
        || match (repo_path, line_number) {
            (Some(repo_path), Some(line_number)) => {
                source_line_in_test_module(repo_path, file, line_number).unwrap_or(false)
            }
            _ => false,
        }
}

fn source_line_in_test_module(repo_path: &Path, file: &str, line_number: usize) -> Option<bool> {
    let source = std::fs::read_to_string(repo_path.join(file)).ok()?;
    let mut context = RustTestContext::default();
    for (index, line) in source.lines().enumerate() {
        if index + 1 == line_number {
            return Some(context.in_test_code(file));
        }
        context.observe(line);
    }
    None
}

fn is_unsafe_code_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//")
        || trimmed.starts_with("///")
        || trimmed.starts_with("//!")
        || trimmed.starts_with("#[")
    {
        return false;
    }

    contains_rust_keyword(&code_without_line_comments_and_strings(trimmed), "unsafe")
}

fn code_without_line_comments_and_strings(line: &str) -> String {
    let mut code = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '/' && chars.peek().is_some_and(|next| *next == '/') {
            break;
        }
        if ch == '"' {
            code.push(' ');
            let mut escaped = false;
            for string_ch in chars.by_ref() {
                if escaped {
                    escaped = false;
                    continue;
                }
                if string_ch == '\\' {
                    escaped = true;
                    continue;
                }
                if string_ch == '"' {
                    break;
                }
            }
            continue;
        }
        code.push(ch);
    }
    code
}

fn contains_rust_keyword(code: &str, keyword: &str) -> bool {
    code.match_indices(keyword).any(|(index, _)| {
        let before = code[..index].chars().next_back();
        let after = code[index + keyword.len()..].chars().next();
        !before.is_some_and(is_rust_ident_continue) && !after.is_some_and(is_rust_ident_continue)
    })
}

fn is_rust_ident_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn is_public_api_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    [
        "pub fn ",
        "pub struct ",
        "pub enum ",
        "pub trait ",
        "pub type ",
        "pub const ",
        "pub static ",
        "pub mod ",
        "pub(crate) fn ",
        "pub(crate) struct ",
        "pub(crate) enum ",
        "pub(crate) trait ",
        "pub(crate) type ",
        "pub(crate) const ",
        "pub(crate) static ",
        "pub(crate) mod ",
    ]
    .iter()
    .any(|prefix| trimmed.starts_with(prefix))
}

fn summarize_line(line: &str) -> String {
    crate::truncate_at(line.trim(), 140).into_owned()
}

fn has_text_evidence(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty()
        && !matches!(
            text.to_ascii_lowercase().as_str(),
            "none" | "n/a" | "na" | "not run" | "not applicable"
        )
}

fn has_optional_evidence(text: Option<&str>) -> bool {
    text.is_some_and(has_text_evidence)
}

fn has_list_evidence(items: &[String]) -> bool {
    items.iter().any(|item| has_text_evidence(item))
}

fn format_list_status(items: &[String]) -> String {
    if has_list_evidence(items) {
        let count = items.iter().filter(|item| has_text_evidence(item)).count();
        format!("{count} item(s)")
    } else {
        "missing".to_string()
    }
}

fn claims_performance(task: &str, plan: &str) -> bool {
    let text = format!("{task} {plan}").to_ascii_lowercase();
    [
        "performance",
        "speed",
        "faster",
        "latency",
        "throughput",
        "optimize",
        "optimise",
        "benchmark",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    fn evidence<'a>(
        plan: &'a str,
        docs: &'a [String],
        impact: &'a [String],
        tests: &'a [String],
    ) -> QualityGateEvidence<'a> {
        QualityGateEvidence {
            task: "change rust code",
            plan,
            docs_verified: docs,
            impact_checked: impact,
            tests_run: tests,
            performance_evidence: None,
            safety_notes: None,
        }
    }

    #[test]
    fn clean_rust_diff_passes_with_evidence() {
        let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@
+fn helper() -> u8 { 1 }
";
        let docs = vec!["std_docs HashMap".to_string()];
        let impact = vec!["impact helper".to_string()];
        let tests = vec!["cargo test".to_string()];
        let report = analyze_quality_gate(
            diff,
            &evidence("use a small helper fn", &docs, &impact, &tests),
        );
        assert!(report.contains("Quality Gate: PASS"));
    }

    #[test]
    fn rust_diff_blocks_missing_plan_docs_and_tests() {
        let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@
+fn helper() -> u8 { 1 }
";
        let report = analyze_quality_gate(diff, &evidence("", &[], &[], &[]));
        assert!(report.contains("Quality Gate: BLOCKED"));
        assert!(report.contains("Missing design plan"));
        assert!(report.contains("Missing docs verification"));
        assert!(report.contains("Missing test evidence"));
    }

    #[test]
    fn deleted_rust_file_blocks_missing_plan_docs_and_tests() {
        let diff = "\
diff --git a/src/old.rs b/src/old.rs
deleted file mode 100644
--- a/src/old.rs
+++ /dev/null
@@
-pub fn removed() {}
";
        let report = analyze_quality_gate(diff, &evidence("", &[], &[], &[]));
        assert!(report.contains("Quality Gate: BLOCKED"));
        assert!(report.contains("Missing design plan"));
        assert!(report.contains("Missing docs verification"));
        assert!(report.contains("Missing test evidence"));
    }

    #[test]
    fn deleted_rust_file_edits_stay_attributed_to_deleted_path() {
        let diff = "\
diff --git a/src/keep.rs b/src/keep.rs
--- a/src/keep.rs
+++ b/src/keep.rs
@@
+fn keep() {}
diff --git a/src/delete.rs b/src/delete.rs
deleted file mode 100644
--- a/src/delete.rs
+++ /dev/null
@@
-pub fn removed() {}
";
        let docs = vec!["std_docs Result".to_string()];
        let tests = vec!["cargo test".to_string()];
        let report = analyze_quality_gate(diff, &evidence("remove old API", &docs, &[], &tests));
        assert!(report.contains("Quality Gate: WARN"));
        assert!(report.contains("src/delete.rs: pub fn removed() {}"));
        assert!(!report.contains("src/keep.rs: pub fn removed() {}"));
    }

    #[test]
    fn public_api_edit_warns_without_impact() {
        let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@
+pub fn helper() -> u8 { 1 }
";
        let docs = vec!["std_docs Result".to_string()];
        let tests = vec!["cargo test".to_string()];
        let report = analyze_quality_gate(diff, &evidence("add helper", &docs, &[], &tests));
        assert!(report.contains("Quality Gate: WARN"));
        assert!(report.contains("Public API-looking edits"));
    }

    #[test]
    fn unwrap_blocks_in_non_test_rust() {
        let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@
+fn helper(input: Option<u8>) -> u8 { input.unwrap() }
";
        let docs = vec!["std_docs Option::unwrap".to_string()];
        let impact = vec!["impact helper".to_string()];
        let tests = vec!["cargo test".to_string()];
        let report = analyze_quality_gate(diff, &evidence("add helper", &docs, &impact, &tests));
        assert!(report.contains("Quality Gate: BLOCKED"));
        assert!(report.contains("unwrap"));
    }

    #[test]
    fn unsafe_requires_safety_notes() {
        let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@
+pub unsafe fn helper() {}
";
        let docs = vec!["std_docs unsafe".to_string()];
        let impact = vec!["impact helper".to_string()];
        let tests = vec!["cargo test".to_string()];
        let report = analyze_quality_gate(diff, &evidence("add helper", &docs, &impact, &tests));
        assert!(report.contains("Quality Gate: BLOCKED"));
        assert!(report.contains("safety notes"));
    }

    #[test]
    fn unsafe_word_in_docs_does_not_require_safety_notes() {
        let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@
+/// Required when the diff adds or changes unsafe code.
+pub fn helper() {}
";
        let docs = vec!["std_docs fn".to_string()];
        let impact = vec!["impact helper".to_string()];
        let tests = vec!["cargo test".to_string()];
        let report = analyze_quality_gate(diff, &evidence("add docs", &docs, &impact, &tests));
        assert!(!report.contains("without safety notes"));
    }

    #[test]
    fn unsafe_block_in_expression_requires_safety_notes() {
        let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@
+pub fn helper(ptr: *const u8) -> u8 { read_byte(unsafe { ptr.read() }) }
";
        let docs = vec!["std_docs pointer read".to_string()];
        let impact = vec!["impact helper".to_string()];
        let tests = vec!["cargo test".to_string()];
        let report = analyze_quality_gate(diff, &evidence("add helper", &docs, &impact, &tests));
        assert!(report.contains("Quality Gate: BLOCKED"));
        assert!(report.contains("without safety notes"));
    }

    #[test]
    fn unsafe_word_in_string_literal_does_not_require_safety_notes() {
        let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@
+pub fn helper() -> &'static str { \"unsafe is a word here\" }
";
        let docs = vec!["std_docs str".to_string()];
        let impact = vec!["impact helper".to_string()];
        let tests = vec!["cargo test".to_string()];
        let report = analyze_quality_gate(diff, &evidence("add helper", &docs, &impact, &tests));
        assert!(!report.contains("without safety notes"));
    }

    #[test]
    fn performance_claim_requires_evidence() {
        let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@
+fn helper() {}
";
        let docs = vec!["std_docs Vec".to_string()];
        let impact = vec!["impact helper".to_string()];
        let tests = vec!["cargo test".to_string()];
        let mut evidence = evidence("make it faster", &docs, &impact, &tests);
        evidence.task = "optimize latency";
        let report = analyze_quality_gate(diff, &evidence);
        assert!(report.contains("Quality Gate: BLOCKED"));
        assert!(report.contains("performance_evidence"));
    }

    #[test]
    fn test_file_unwrap_does_not_block() {
        let diff = "\
diff --git a/tests/lib.rs b/tests/lib.rs
--- a/tests/lib.rs
+++ b/tests/lib.rs
@@
+fn helper(input: Option<u8>) -> u8 { input.unwrap() }
";
        let docs = vec!["std_docs Option::unwrap".to_string()];
        let impact = vec!["impact helper".to_string()];
        let tests = vec!["cargo test".to_string()];
        let report =
            analyze_quality_gate(diff, &evidence("add test helper", &docs, &impact, &tests));
        assert!(!report.contains("New non-test `unwrap(` detected"));
    }

    #[test]
    fn cfg_test_module_unwrap_does_not_block() {
        let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@
 #[cfg(test)]
 mod tests {
+    #[test]
+    fn unwrap_in_unit_test() {
+        let value = Some(1).unwrap();
+        assert_eq!(value, 1);
+    }
 }
";
        let docs = vec!["std_docs Option::unwrap".to_string()];
        let impact = vec!["impact helper".to_string()];
        let tests = vec!["cargo test".to_string()];
        let report = analyze_quality_gate(diff, &evidence("add unit test", &docs, &impact, &tests));
        assert!(!report.contains("New non-test `unwrap(` detected"));
    }

    #[test]
    fn cfg_test_module_unwrap_detected_from_source_when_hunk_lacks_context() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("lib.rs"),
            "\
pub fn helper() {}

#[cfg(test)]
mod tests {
    #[test]
    fn unwrap_in_unit_test() {
        let value = Some(1).unwrap();
        assert_eq!(value, 1);
    }
}
",
        )
        .unwrap();
        let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -6,0 +7,1 @@
+        let value = Some(1).unwrap();
";
        let docs = vec!["std_docs Option::unwrap".to_string()];
        let impact = vec!["impact helper".to_string()];
        let tests = vec!["cargo test".to_string()];
        let report = analyze_quality_gate_with_repo(
            diff,
            &evidence("add unit test", &docs, &impact, &tests),
            Some(dir.path()),
        );
        assert!(!report.contains("New non-test `unwrap(` detected"));
    }
}
