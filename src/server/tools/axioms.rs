use serde::Deserialize;
use std::cmp::Reverse;
use std::fmt::Write;
use std::sync::OnceLock;

/// On-disk JSON shape. Fields we never query (`id`) are not deserialised.
/// serde silently skips unknown keys, so dropping `id` from the struct
/// keeps the JSON authoritative without forcing a dead field.
#[derive(Deserialize, Debug)]
struct RawAxiom {
    category: String,
    triggers: Vec<String>,
    rule_summary: String,
    prompt_injection: String,
    anti_pattern: String,
    good_pattern: String,
}

/// In-memory axiom with pre-lowercased fields. Scoring touches every
/// axiom on every query; lowering once at load time trades ~a few KB
/// of steady-state memory for avoiding 31 × (n triggers + 2) string
/// allocations per call.
#[derive(Debug)]
#[non_exhaustive]
pub struct Axiom {
    pub category: String,
    pub triggers: Vec<String>,
    pub rule_summary: String,
    pub prompt_injection: String,
    pub anti_pattern: String,
    pub good_pattern: String,
    category_lower: String,
    triggers_lower: Vec<String>,
    rule_summary_lower: String,
}

impl From<RawAxiom> for Axiom {
    fn from(raw: RawAxiom) -> Self {
        let category_lower = raw.category.to_lowercase();
        let triggers_lower = raw.triggers.iter().map(|t| t.to_lowercase()).collect();
        let rule_summary_lower = raw.rule_summary.to_lowercase();
        Self {
            category: raw.category,
            triggers: raw.triggers,
            rule_summary: raw.rule_summary,
            prompt_injection: raw.prompt_injection,
            anti_pattern: raw.anti_pattern,
            good_pattern: raw.good_pattern,
            category_lower,
            triggers_lower,
            rule_summary_lower,
        }
    }
}

// Bake the JSON into the binary; the path is relative to this file.
const AXIOMS_JSON: &str = include_str!("../../../assets/axioms.json");

/// Parse once, cache forever. Parse failure is returned to the caller on
/// first call; later calls will retry until one succeeds.
fn axioms() -> Result<&'static [Axiom], crate::IlluError> {
    static AXIOMS: OnceLock<Vec<Axiom>> = OnceLock::new();
    if let Some(cached) = AXIOMS.get() {
        return Ok(cached);
    }
    let raw: Vec<RawAxiom> = serde_json::from_str(AXIOMS_JSON)?;
    let parsed: Vec<Axiom> = raw.into_iter().map(Axiom::from).collect();
    // Lost-race set is fine — the winner's Vec is equivalent to ours.
    let _ = AXIOMS.set(parsed);
    AXIOMS
        .get()
        .map(Vec::as_slice)
        .ok_or_else(|| "axioms cache not initialised after set".into())
}

pub fn handle_axioms(query: &str) -> Result<String, crate::IlluError> {
    let axioms = axioms()?;

    let query_lower = query.to_lowercase();
    let query_terms: Vec<&str> = query_lower.split_whitespace().collect();

    let mut scored: Vec<(&Axiom, usize)> = axioms
        .iter()
        .map(|axiom| {
            let mut score = 0;
            for term in &query_terms {
                if axiom.category_lower.contains(term) {
                    score += 5;
                }
                for trigger in &axiom.triggers_lower {
                    if trigger.contains(term) {
                        score += 10;
                    }
                }
                if axiom.rule_summary_lower.contains(term) {
                    score += 2;
                }
            }
            // Exact-match boost: user typed a category or trigger name
            // verbatim. `.contains` on the full query against single-word
            // strings was unreachable for multi-word queries — this
            // gives both single- and multi-word queries the same shot.
            if axiom.category_lower == query_lower {
                score += 20;
            }
            for trigger in &axiom.triggers_lower {
                if trigger == &query_lower {
                    score += 30;
                }
            }
            (axiom, score)
        })
        .filter(|(_, score)| *score > 0)
        .collect();

    scored.sort_by_key(|&(_, score)| Reverse(score));

    // The top-N is sized to the number of design-category axioms in
    // `assets/axioms.json` (Design Workflow, Data Modeling, Documentation,
    // Comments, Idiomatic Rust, Verification Sources, Performance Discipline)
    // so the baseline quality query returns every design rule alongside
    // whichever language-mechanics rules also score. Bump in lockstep when a
    // new design category is added.
    let top: Vec<&Axiom> = scored.into_iter().take(7).map(|(a, _)| a).collect();

    let mut output = String::new();
    let _ = writeln!(output, "## Rust Axioms matching '{query}'\n");

    if top.is_empty() {
        let _ = writeln!(
            output,
            "No matching Rust Axioms found in the database. Please refine your query."
        );
        return Ok(output);
    }

    for axiom in top {
        let _ = writeln!(output, "### [{}] {}", axiom.category, axiom.rule_summary);
        let _ = writeln!(output, "> **{}**\n", axiom.prompt_injection);

        if !axiom.good_pattern.is_empty() {
            let _ = writeln!(
                output,
                "#### Good Pattern:\n```rust\n{}\n```",
                axiom.good_pattern
            );
        }
        if !axiom.anti_pattern.is_empty() {
            let _ = writeln!(
                output,
                "#### Anti-Pattern:\n```rust\n{}\n```",
                axiom.anti_pattern
            );
        }
        let _ = writeln!(output, "---");
    }

    Ok(output)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn test_handle_axioms() {
        let query = "thread shared state unwrap";
        let result = handle_axioms(query).unwrap();
        assert!(result.contains("unwrap"));
    }

    #[test]
    fn test_exact_match_boost_single_word() {
        // An exact trigger match should outrank a mere substring hit.
        let result = handle_axioms("ownership").unwrap();
        assert!(result.contains("Ownership"));
    }

    #[test]
    fn test_quality_query_returns_design_axioms() {
        let result = handle_axioms(crate::agents::instruction_md::RUST_QUALITY_QUERY).unwrap();
        assert!(result.contains("Design Workflow"));
        assert!(result.contains("Data Modeling"));
        assert!(result.contains("Documentation"));
        assert!(result.contains("Comments"));
        assert!(result.contains("Idiomatic Rust"));
        assert!(result.contains("Verification Sources"));
        assert!(result.contains("Performance Discipline"));
    }
}
