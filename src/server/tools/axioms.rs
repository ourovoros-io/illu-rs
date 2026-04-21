use serde::Deserialize;
use std::cmp::Reverse;
use std::fmt::Write;
use std::sync::OnceLock;

#[derive(Deserialize, Debug)]
struct Axiom {
    #[expect(dead_code, reason = "round-trip field from the JSON schema")]
    id: String,
    category: String,
    triggers: Vec<String>,
    rule_summary: String,
    prompt_injection: String,
    anti_pattern: String,
    good_pattern: String,
}

// Bake the JSON into the binary; the path is relative to this file.
const AXIOMS_JSON: &str = include_str!("../../../assets/axioms.json");

/// Parse once, cache forever. Parse failure is returned to the caller on
/// first call; later calls will retry until one succeeds.
fn axioms() -> Result<&'static [Axiom], Box<dyn std::error::Error>> {
    static AXIOMS: OnceLock<Vec<Axiom>> = OnceLock::new();
    if let Some(cached) = AXIOMS.get() {
        return Ok(cached);
    }
    let parsed: Vec<Axiom> = serde_json::from_str(AXIOMS_JSON)?;
    // Lost-race set is fine — the winner's Vec is equivalent to ours.
    let _ = AXIOMS.set(parsed);
    AXIOMS
        .get()
        .map(Vec::as_slice)
        .ok_or_else(|| "axioms cache not initialised after set".into())
}

pub fn handle_axioms(query: &str) -> Result<String, Box<dyn std::error::Error>> {
    let axioms = axioms()?;

    let query_lower = query.to_lowercase();
    let query_terms: Vec<&str> = query_lower.split_whitespace().collect();

    let mut scored: Vec<(&Axiom, usize)> = axioms
        .iter()
        .map(|axiom| {
            let mut score = 0;
            for term in &query_terms {
                if axiom.category.to_lowercase().contains(term) {
                    score += 5;
                }
                for trigger in &axiom.triggers {
                    if trigger.to_lowercase().contains(term) {
                        score += 10;
                    }
                }
                if axiom.rule_summary.to_lowercase().contains(term) {
                    score += 2;
                }
            }
            // Exact-match boost: user typed a category or trigger name
            // verbatim. `.contains` on the full query against single-word
            // strings was unreachable for multi-word queries — this
            // gives both single- and multi-word queries the same shot.
            if axiom.category.eq_ignore_ascii_case(&query_lower) {
                score += 20;
            }
            for trigger in &axiom.triggers {
                if trigger.eq_ignore_ascii_case(&query_lower) {
                    score += 30;
                }
            }
            (axiom, score)
        })
        .filter(|(_, score)| *score > 0)
        .collect();

    scored.sort_by_key(|&(_, score)| Reverse(score));

    let top: Vec<&Axiom> = scored.into_iter().take(3).map(|(a, _)| a).collect();

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
}
