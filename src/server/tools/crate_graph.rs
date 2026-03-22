use crate::db::Database;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

pub fn handle_crate_graph(db: &Database) -> Result<String, Box<dyn std::error::Error>> {
    let crate_count = db.crate_count()?;
    if crate_count <= 1 {
        return Ok("Single-crate project — no crate dependency graph.".to_string());
    }

    let crates = db.all_crates()?;
    let deps = db.all_crate_deps()?;

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

    let all_names: BTreeSet<&str> =
        crates.iter().map(|c| c.name.as_str()).collect();
    let sources: BTreeSet<&str> = deps.iter().map(|(s, _)| s.as_str()).collect();
    let targets: BTreeSet<&str> = deps.iter().map(|(_, t)| t.as_str()).collect();

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

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::db::Database;

    #[test]
    fn test_crate_graph_single_crate_returns_early() {
        let db = Database::open_in_memory().unwrap();
        db.insert_crate("my_crate", "crates/my_crate").unwrap();

        let result = handle_crate_graph(&db).unwrap();
        assert_eq!(result, "Single-crate project — no crate dependency graph.");
    }

    #[test]
    fn test_crate_graph_empty_returns_early() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_crate_graph(&db).unwrap();
        assert_eq!(result, "Single-crate project — no crate dependency graph.");
    }

    #[test]
    fn test_crate_graph_shows_deps() {
        let db = Database::open_in_memory().unwrap();
        let app_id = db.insert_crate("app", "crates/app").unwrap();
        let core_id = db.insert_crate("core", "crates/core").unwrap();
        let util_id = db.insert_crate("util", "crates/util").unwrap();
        db.insert_crate_dep(app_id, core_id).unwrap();
        db.insert_crate_dep(core_id, util_id).unwrap();

        let result = handle_crate_graph(&db).unwrap();

        assert!(result.contains("## Crate Dependency Graph"));
        assert!(result.contains("**app**"));
        assert!(result.contains("**core**"));
        assert!(result.contains("**util**"));
        assert!(result.contains("**app** → core"));
        assert!(result.contains("**core** → util"));
    }

    #[test]
    fn test_crate_graph_identifies_roots_and_leaves() {
        let db = Database::open_in_memory().unwrap();
        // app -> core -> util
        // app is root (nothing depends on it)
        // util is leaf (no deps)
        // core is neither
        let app_id = db.insert_crate("app", "crates/app").unwrap();
        let core_id = db.insert_crate("core", "crates/core").unwrap();
        let util_id = db.insert_crate("util", "crates/util").unwrap();
        db.insert_crate_dep(app_id, core_id).unwrap();
        db.insert_crate_dep(core_id, util_id).unwrap();

        let result = handle_crate_graph(&db).unwrap();

        assert!(
            result.contains("Root crates"),
            "should identify root crates"
        );
        assert!(result.contains("**app**"), "app should be a root");
        assert!(
            result.contains("Leaf crates"),
            "should identify leaf crates"
        );
        assert!(result.contains("**util**"), "util should be a leaf");
    }

    #[test]
    fn test_crate_graph_no_deps_between_crates() {
        let db = Database::open_in_memory().unwrap();
        db.insert_crate("alpha", "crates/alpha").unwrap();
        db.insert_crate("beta", "crates/beta").unwrap();

        let result = handle_crate_graph(&db).unwrap();

        assert!(result.contains("## Crate Dependency Graph"));
        assert!(result.contains("No inter-crate dependencies."));
    }
}
