use crate::db::Database;
use std::collections::BTreeMap;
use std::fmt::Write;

pub fn handle_crate_graph(db: &Database) -> Result<String, Box<dyn std::error::Error>> {
    let crate_count = db.get_crate_count()?;
    if crate_count <= 1 {
        return Ok("Single-crate project — no crate dependency graph.".to_string());
    }

    let crates = db.get_all_crates()?;
    let deps = db.get_all_crate_deps()?;

    let mut output = String::new();
    let _ = writeln!(output, "## Crate Dependency Graph\n");
    let _ = writeln!(output, "**{} crates:**\n", crates.len());

    for c in &crates {
        let _ = writeln!(output, "- **{}** (`{}`)", c.name, c.path);
    }

    let mut adj: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (source, target) in &deps {
        adj.entry(source.clone()).or_default().push(target.clone());
    }

    if adj.is_empty() {
        let _ = writeln!(output, "\nNo inter-crate dependencies.");
        return Ok(output);
    }

    let _ = writeln!(output, "\n### Dependencies\n");
    for (source, targets) in &adj {
        let targets_str = targets.join(", ");
        let _ = writeln!(output, "- **{source}** → {targets_str}");
    }

    let all_names: std::collections::BTreeSet<&str> =
        crates.iter().map(|c| c.name.as_str()).collect();
    let sources: std::collections::BTreeSet<&str> = deps.iter().map(|(s, _)| s.as_str()).collect();
    let targets: std::collections::BTreeSet<&str> = deps.iter().map(|(_, t)| t.as_str()).collect();

    let leaves: Vec<&&str> = all_names
        .iter()
        .filter(|n| !sources.contains(**n))
        .collect();
    let roots: Vec<&&str> = all_names
        .iter()
        .filter(|n| !targets.contains(**n))
        .collect();

    if !roots.is_empty() {
        let _ = writeln!(
            output,
            "\n**Root crates** (not depended on): {}",
            roots
                .iter()
                .map(|n| format!("**{n}**"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if !leaves.is_empty() {
        let _ = writeln!(
            output,
            "**Leaf crates** (no deps): {}",
            leaves
                .iter()
                .map(|n| format!("**{n}**"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    Ok(output)
}
