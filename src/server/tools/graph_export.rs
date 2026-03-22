use crate::db::Database;
use std::fmt::Write;

pub fn handle_graph_export(
    db: &Database,
    symbol_name: Option<&str>,
    path: Option<&str>,
    depth: Option<i64>,
    direction: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(sym) = symbol_name {
        return export_symbol_graph(db, sym, depth.unwrap_or(2), direction.unwrap_or("both"));
    }
    if let Some(p) = path {
        return export_file_graph(db, p);
    }
    Ok(
        "Provide either `symbol_name` for a call graph or `path` for a file dependency graph."
            .into(),
    )
}

fn export_symbol_graph(
    db: &Database,
    symbol_name: &str,
    max_depth: i64,
    direction: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let symbols = super::resolve_symbol(db, symbol_name)?;
    if symbols.is_empty() {
        return Ok(super::symbol_not_found(db, symbol_name));
    }

    let base_name = &symbols[0].name;
    let base_file = &symbols[0].file_path;
    let mut output = String::new();
    let _ = writeln!(output, "digraph call_graph {{");
    let _ = writeln!(output, "  rankdir=LR;");
    let _ = writeln!(output, "  node [shape=box, fontname=\"monospace\"];");

    let include_down = direction == "both" || direction == "down";
    let include_up = direction == "both" || direction == "up";

    let mut seen = std::collections::HashSet::new();
    seen.insert((base_name.clone(), base_file.clone()));

    if include_down {
        let mut queue = vec![(base_name.clone(), base_file.clone(), 0i64)];
        while let Some((name, file, depth)) = queue.pop() {
            if depth >= max_depth {
                continue;
            }
            let callees = db.get_callees(&name, &file, false)?;
            for c in &callees {
                let _ = writeln!(output, "  \"{name}\" -> \"{}\";", c.name);
                if seen.insert((c.name.clone(), c.file_path.clone())) {
                    queue.push((c.name.clone(), c.file_path.clone(), depth + 1));
                }
            }
        }
    }

    if include_up {
        let mut queue = vec![(base_name.clone(), 0i64)];
        let mut seen_up = std::collections::HashSet::new();
        seen_up.insert(base_name.clone());
        while let Some((name, depth)) = queue.pop() {
            if depth >= max_depth {
                continue;
            }
            let callers = db.get_callers_by_name(&name, Some("high"), false)?;
            for (caller, _path) in &callers {
                let _ = writeln!(output, "  \"{caller}\" -> \"{name}\";");
                if seen_up.insert(caller.clone()) {
                    queue.push((caller.clone(), depth + 1));
                }
            }
        }
    }

    let _ = writeln!(output, "}}");
    Ok(output)
}

fn export_file_graph(
    db: &Database,
    path_prefix: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let deps = db.get_file_dependencies(path_prefix, Some("high"))?;

    let mut output = String::new();
    let _ = writeln!(output, "digraph file_deps {{");
    let _ = writeln!(output, "  rankdir=TB;");
    let _ = writeln!(output, "  node [shape=box, fontname=\"monospace\"];");

    for (source, target) in &deps {
        let short_src = source.strip_prefix(path_prefix).unwrap_or(source);
        let short_tgt = target.strip_prefix(path_prefix).unwrap_or(target);
        let _ = writeln!(output, "  \"{short_src}\" -> \"{short_tgt}\";");
    }

    let _ = writeln!(output, "}}");

    let file_count = deps
        .iter()
        .flat_map(|(s, t)| [s.as_str(), t.as_str()])
        .collect::<std::collections::HashSet<_>>()
        .len();
    let _ = writeln!(
        output,
        "\n// {file_count} files, {} dependency edges",
        deps.len()
    );

    Ok(output)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn test_graph_export_needs_params() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_graph_export(&db, None, None, None, None).unwrap();
        assert!(result.contains("Provide either"));
    }

    #[test]
    fn test_graph_export_symbol_not_found() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_graph_export(&db, Some("nonexistent"), None, None, None).unwrap();
        assert!(result.contains("No symbol found"));
    }

    #[test]
    fn test_graph_export_file_empty() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_graph_export(&db, None, Some("src/"), None, None).unwrap();
        assert!(result.contains("digraph"));
        assert!(result.contains("0 files"));
    }
}
