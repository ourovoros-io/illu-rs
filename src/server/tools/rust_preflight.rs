use std::fmt::Write;
use std::path::Path;

use serde::Deserialize;

use crate::agents::instruction_md::RUST_QUALITY_QUERY;
use crate::db::Database;

const PLAYBOOK: &str = include_str!("../../../docs/rust-playbook.md");
const MODEL_FAILURES_JSON: &str = include_str!("../../../assets/model_failures.json");

#[derive(Debug, Deserialize)]
struct ModelFailureCase {
    id: String,
    title: String,
    trigger_terms: Vec<String>,
    bad_pattern: String,
    good_pattern: String,
    rule: String,
    source: String,
}

pub fn handle_rust_preflight(
    db: &Database,
    repo_path: &Path,
    task: &str,
    symbols: &[String],
    std_items: &[String],
    dependencies: &[String],
    git_ref: Option<&str>,
) -> Result<String, crate::IlluError> {
    let mut output = String::new();
    let _ = writeln!(output, "# Rust Preflight\n");
    let _ = writeln!(output, "## Task\n\n{}\n", task.trim());
    render_required_template(&mut output);
    render_playbook_excerpt(&mut output);
    render_axioms(&mut output, "Baseline axioms", RUST_QUALITY_QUERY)?;
    render_axioms(&mut output, "Task axioms", task)?;
    render_model_failures(&mut output, task, symbols, std_items, dependencies)?;
    render_symbol_evidence(&mut output, db, symbols)?;
    render_std_docs(&mut output, std_items)?;
    render_dependency_docs(&mut output, db, dependencies)?;
    render_diff_context(&mut output, db, repo_path, git_ref)?;
    Ok(output)
}

fn render_required_template(output: &mut String) {
    let _ = writeln!(output, "## Required Design Plan Template\n");
    let _ = writeln!(
        output,
        "Fill this in before coding. Do not treat this preflight as the design itself.\n"
    );
    let _ = writeln!(output, "- Goal and non-goals:");
    let _ = writeln!(output, "- Data flow:");
    let _ = writeln!(output, "- Data structures and why each one fits:");
    let _ = writeln!(
        output,
        "- Ownership, borrowing, mutation, and lifetime boundaries:"
    );
    let _ = writeln!(output, "- Invariants and invalid states:");
    let _ = writeln!(output, "- Error strategy using `IlluError` / `Result`:");
    let _ = writeln!(output, "- Documentation verified:");
    let _ = writeln!(output, "- Impact and test plan:");
    let _ = writeln!(output, "- Performance or safety evidence, if claimed:\n");
}

fn render_playbook_excerpt(output: &mut String) {
    let _ = writeln!(output, "## Project Rust Playbook\n");
    let excerpt = crate::truncate_at(PLAYBOOK, 2_400);
    let _ = writeln!(output, "{excerpt}\n");
}

fn render_axioms(output: &mut String, heading: &str, query: &str) -> Result<(), crate::IlluError> {
    let _ = writeln!(output, "## {heading}\n");
    let rendered = super::axioms::handle_axioms(query)?;
    let excerpt = crate::truncate_at(&rendered, 5_000);
    let _ = writeln!(output, "{excerpt}\n");
    Ok(())
}

fn render_model_failures(
    output: &mut String,
    task: &str,
    symbols: &[String],
    std_items: &[String],
    dependencies: &[String],
) -> Result<(), crate::IlluError> {
    let cases = model_failure_cases()?;
    let matched = matching_model_failures(&cases, task, symbols, std_items, dependencies);
    let _ = writeln!(output, "## Model Failure Reminders\n");
    if matched.is_empty() {
        let _ = writeln!(
            output,
            "No corpus case matched the supplied trigger terms. Still apply the full playbook.\n"
        );
        return Ok(());
    }
    for case in matched {
        let _ = writeln!(output, "### {} — {}\n", case.id, case.title);
        let _ = writeln!(output, "- **Bad pattern:** {}", case.bad_pattern);
        let _ = writeln!(output, "- **Corrected pattern:** {}", case.good_pattern);
        let _ = writeln!(output, "- **Rule:** {}", case.rule);
        let _ = writeln!(output, "- **Source:** {}\n", case.source);
    }
    Ok(())
}

fn render_symbol_evidence(
    output: &mut String,
    db: &Database,
    symbols: &[String],
) -> Result<(), crate::IlluError> {
    let _ = writeln!(output, "## Symbol Evidence\n");
    if symbols.is_empty() {
        let _ = writeln!(
            output,
            "No symbols supplied. Add relevant local symbols before coding if the task touches indexed code.\n"
        );
        return Ok(());
    }

    for symbol in symbols {
        let _ = writeln!(output, "### `{symbol}`\n");
        let sections = ["docs", "source", "callers", "tested_by"];
        let context =
            super::context::handle_context(db, symbol, false, None, Some(&sections), None, true)?;
        let _ = writeln!(
            output,
            "#### Context\n\n{}\n",
            crate::truncate_at(&context, 4_000)
        );
        let impact = super::impact::handle_impact(db, symbol, Some(2), true, true)?;
        let _ = writeln!(
            output,
            "#### Impact Hints\n\n{}\n",
            crate::truncate_at(&impact, 3_000)
        );
        let tests = super::test_impact::handle_test_impact(db, symbol, Some(3))?;
        let _ = writeln!(
            output,
            "#### Test-Impact Hints\n\n{}\n",
            crate::truncate_at(&tests, 2_000)
        );
    }
    Ok(())
}

fn render_std_docs(output: &mut String, std_items: &[String]) -> Result<(), crate::IlluError> {
    let _ = writeln!(output, "## Standard Library Docs\n");
    if std_items.is_empty() {
        let _ = writeln!(
            output,
            "No standard-library items supplied. Add items such as `std::collections::HashMap::iter` when std behavior matters.\n"
        );
        return Ok(());
    }
    for item in std_items {
        let rendered = super::std_docs::handle_std_docs(item, None)?;
        let _ = writeln!(
            output,
            "### `{item}`\n\n{}\n",
            crate::truncate_at(&rendered, 3_500)
        );
    }
    Ok(())
}

fn render_dependency_docs(
    output: &mut String,
    db: &Database,
    dependencies: &[String],
) -> Result<(), crate::IlluError> {
    let _ = writeln!(output, "## Dependency Docs\n");
    if dependencies.is_empty() {
        let _ = writeln!(
            output,
            "No dependency docs requested. Add `crate` or `crate::topic` entries when external APIs matter.\n"
        );
        return Ok(());
    }
    for dependency in dependencies {
        let (dep, topic) = parse_dependency_request(dependency);
        let rendered = super::docs::handle_docs(db, dep, topic)?;
        let _ = writeln!(
            output,
            "### `{dependency}`\n\n{}\n",
            crate::truncate_at(&rendered, 3_500)
        );
    }
    Ok(())
}

fn render_diff_context(
    output: &mut String,
    db: &Database,
    repo_path: &Path,
    git_ref: Option<&str>,
) -> Result<(), crate::IlluError> {
    let _ = writeln!(output, "## Current Diff Context\n");
    if git_ref.is_none() {
        let _ = writeln!(
            output,
            "No git_ref supplied. Use `quality_gate` after editing with the exact plan/docs/tests evidence.\n"
        );
        return Ok(());
    }
    let rendered = super::diff_impact::handle_diff_impact(db, repo_path, git_ref, false, true)?;
    let _ = writeln!(output, "{}\n", crate::truncate_at(&rendered, 4_000));
    Ok(())
}

fn parse_dependency_request(input: &str) -> (&str, Option<&str>) {
    if let Some((dep, topic)) = input.split_once("::") {
        return (dep, Some(topic));
    }
    if let Some((dep, topic)) = input.split_once(':') {
        return (dep, Some(topic));
    }
    (input, None)
}

fn model_failure_cases() -> Result<Vec<ModelFailureCase>, crate::IlluError> {
    serde_json::from_str(MODEL_FAILURES_JSON).map_err(Into::into)
}

fn matching_model_failures<'a>(
    cases: &'a [ModelFailureCase],
    task: &str,
    symbols: &[String],
    std_items: &[String],
    dependencies: &[String],
) -> Vec<&'a ModelFailureCase> {
    let haystack = evidence_haystack(task, symbols, std_items, dependencies);
    cases
        .iter()
        .filter(|case| {
            case.trigger_terms
                .iter()
                .any(|term| haystack.contains(&term.to_ascii_lowercase()))
        })
        .take(5)
        .collect()
}

fn evidence_haystack(
    task: &str,
    symbols: &[String],
    std_items: &[String],
    dependencies: &[String],
) -> String {
    let mut haystack = task.to_ascii_lowercase();
    for item in symbols.iter().chain(std_items).chain(dependencies) {
        haystack.push(' ');
        haystack.push_str(&item.to_ascii_lowercase());
    }
    haystack
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::indexer::parser::{Symbol, SymbolKind, Visibility};
    use crate::indexer::store::store_symbols;
    use std::collections::HashSet;

    #[test]
    fn model_failure_cases_have_unique_ids_and_required_fields() {
        let cases = model_failure_cases().unwrap();
        let mut ids = HashSet::new();
        for case in &cases {
            assert!(!case.id.trim().is_empty());
            assert!(!case.title.trim().is_empty());
            assert!(!case.trigger_terms.is_empty());
            assert!(!case.bad_pattern.trim().is_empty());
            assert!(!case.good_pattern.trim().is_empty());
            assert!(!case.rule.trim().is_empty());
            assert!(ids.insert(case.id.clone()));
        }
    }

    #[test]
    fn matching_cases_use_trigger_terms() {
        let cases = model_failure_cases().unwrap();
        let matched = matching_model_failures(
            &cases,
            "optimize HashMap order deterministically without benchmark",
            &[],
            &[String::from("std::collections::HashMap")],
            &[],
        );
        let ids: Vec<&str> = matched.iter().map(|case| case.id.as_str()).collect();
        assert!(ids.contains(&"unordered-map-assumption"));
        assert!(ids.contains(&"performance-without-evidence"));
    }

    #[test]
    fn preflight_task_only_returns_template_and_axioms() {
        let db = Database::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let result = handle_rust_preflight(
            &db,
            dir.path(),
            "add a safe parser helper",
            &[],
            &[],
            &[],
            None,
        )
        .unwrap();
        assert!(result.contains("Required Design Plan Template"));
        assert!(result.contains("Baseline axioms"));
        assert!(result.contains("Task axioms"));
        assert!(result.contains("No symbols supplied"));
    }

    #[test]
    fn preflight_includes_symbol_and_dependency_docs() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash").unwrap();
        store_symbols(
            &db,
            file_id,
            &[Symbol {
                name: "helper".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 3,
                signature: "pub fn helper()".into(),
                doc_comment: Some("Does useful work.".into()),
                body: Some("pub fn helper() {}".into()),
                details: None,
                attributes: None,
                impl_type: None,
            }],
        )
        .unwrap();
        let dep_id = db.insert_dependency("serde", "1.0.0", true, None).unwrap();
        db.store_doc(dep_id, "docs.rs", "Serde serializes data")
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let result = handle_rust_preflight(
            &db,
            dir.path(),
            "serde helper docs",
            &[String::from("helper")],
            &[],
            &[String::from("serde")],
            None,
        )
        .unwrap();
        assert!(result.contains("Does useful work"));
        assert!(result.contains("Serde serializes data"));
    }
}
