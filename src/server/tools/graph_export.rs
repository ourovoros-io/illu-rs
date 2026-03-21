use crate::db::Database;
use std::fmt::Write;

pub fn handle_graph_export(
    db: &Database,
    symbol_name: Option<&str>,
    path: Option<&str>,
    depth: Option<i64>,
) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(sym) = symbol_name {
        return export_symbol_graph(db, sym, depth.unwrap_or(2));
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
) -> Result<String, Box<dyn std::error::Error>> {
    let symbols = super::resolve_symbol(db, symbol_name)?;
    if symbols.is_empty() {
        return Ok(format!("Symbol '{symbol_name}' not found."));
    }

    let base_name = &symbols[0].name;
    let mut output = String::new();
    let _ = writeln!(output, "digraph call_graph {{");
    let _ = writeln!(output, "  rankdir=LR;");
    let _ = writeln!(output, "  node [shape=box, fontname=\"monospace\"];");

    let mut seen = std::collections::HashSet::new();
    let mut queue = vec![(base_name.clone(), 0i64)];
    seen.insert(base_name.clone());

    while let Some((name, depth)) = queue.pop() {
        if depth >= max_depth {
            continue;
        }
        let callees = db.get_callees_by_name(&name, Some("high"))?;
        for (callee, _path) in &callees {
            let _ = writeln!(output, "  \"{name}\" -> \"{callee}\";");
            if seen.insert(callee.clone()) {
                queue.push((callee.clone(), depth + 1));
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
        let result = handle_graph_export(&db, None, None, None).unwrap();
        assert!(result.contains("Provide either"));
    }

    #[test]
    fn test_graph_export_symbol_not_found() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_graph_export(&db, Some("nonexistent"), None, None).unwrap();
        assert!(result.contains("not found"));
    }

    #[test]
    fn test_graph_export_file_empty() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_graph_export(&db, None, Some("src/"), None).unwrap();
        assert!(result.contains("digraph"));
        assert!(result.contains("0 files"));
    }
}
