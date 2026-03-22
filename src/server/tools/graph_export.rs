use crate::db::{Database, StoredSymbol};
use std::collections::HashSet;
use std::fmt::Write;

pub fn handle_graph_export(
    db: &Database,
    symbol_name: Option<&str>,
    path: Option<&str>,
    depth: Option<i64>,
    direction: Option<super::Direction>,
    format: Option<super::ExportFormat>,
) -> Result<String, Box<dyn std::error::Error>> {
    use super::{Direction, ExportFormat};

    let fmt = format.unwrap_or(ExportFormat::Dot);

    if let Some(sym) = symbol_name {
        let symbols = super::resolve_symbol(db, sym)?;
        if symbols.is_empty() {
            return Ok(super::symbol_not_found(db, sym));
        }
        let max_depth = depth.unwrap_or(2);
        let dir = direction.unwrap_or(Direction::Both);
        let edges = collect_symbol_edges(db, &symbols[0], max_depth, dir)?;
        return Ok(render_edges(&edges, fmt, "call_graph"));
    }

    if let Some(p) = path {
        let edges = collect_file_edges(db, p)?;
        return Ok(render_edges(&edges, fmt, "file_deps"));
    }

    Ok(
        "Provide either `symbol_name` for a call graph or `path` for a file dependency graph."
            .into(),
    )
}

fn collect_symbol_edges(
    db: &Database,
    symbol: &StoredSymbol,
    max_depth: i64,
    direction: super::Direction,
) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
    use super::Direction;
    let include_down = direction == Direction::Both || direction == Direction::Down;
    let include_up = direction == Direction::Both || direction == Direction::Up;

    let mut edges = Vec::new();
    let mut seen = HashSet::new();
    seen.insert((symbol.name.clone(), symbol.file_path.clone()));

    if include_down {
        let mut queue = vec![(symbol.name.clone(), symbol.file_path.clone(), 0i64)];
        while let Some((name, file, d)) = queue.pop() {
            if d >= max_depth {
                continue;
            }
            let callees = db.get_callees(&name, &file, false)?;
            for c in &callees {
                edges.push((name.clone(), c.name.clone()));
                if seen.insert((c.name.clone(), c.file_path.clone())) {
                    queue.push((c.name.clone(), c.file_path.clone(), d + 1));
                }
            }
        }
    }

    if include_up {
        let mut queue = vec![(symbol.name.clone(), 0i64)];
        let mut seen_up = HashSet::new();
        seen_up.insert(symbol.name.clone());
        while let Some((name, d)) = queue.pop() {
            if d >= max_depth {
                continue;
            }
            let callers = db.get_callers_by_name(&name, Some("high"), false)?;
            for (caller, _path) in &callers {
                edges.push((caller.clone(), name.clone()));
                if seen_up.insert(caller.clone()) {
                    queue.push((caller.clone(), d + 1));
                }
            }
        }
    }

    Ok(edges)
}

fn collect_file_edges(
    db: &Database,
    path_prefix: &str,
) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
    let deps = db.get_file_dependencies(path_prefix, Some("high"))?;
    let edges: Vec<(String, String)> = deps
        .into_iter()
        .map(|(src, tgt)| {
            let short_src = src.strip_prefix(path_prefix).unwrap_or(&src).to_owned();
            let short_tgt = tgt.strip_prefix(path_prefix).unwrap_or(&tgt).to_owned();
            (short_src, short_tgt)
        })
        .collect();
    Ok(edges)
}

fn render_edges(
    edges: &[(String, String)],
    format: super::ExportFormat,
    graph_name: &str,
) -> String {
    use super::ExportFormat;
    let mut nodes = HashSet::new();
    for (src, tgt) in edges {
        nodes.insert(src.as_str());
        nodes.insert(tgt.as_str());
    }
    let node_count = nodes.len();
    let edge_count = edges.len();

    match format {
        ExportFormat::Edges => render_edges_format(edges, node_count, edge_count),
        ExportFormat::Summary => render_summary(edges, node_count, edge_count),
        ExportFormat::Dot => render_dot(edges, graph_name, node_count, edge_count),
    }
}

fn render_dot(
    edges: &[(String, String)],
    graph_name: &str,
    node_count: usize,
    edge_count: usize,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "digraph {graph_name} {{");
    let _ = writeln!(out, "  rankdir=LR;");
    let _ = writeln!(out, "  node [shape=box, fontname=\"monospace\"];");
    for (src, tgt) in edges {
        let _ = writeln!(out, "  \"{src}\" -> \"{tgt}\";");
    }
    let _ = writeln!(out, "}}");
    let _ = writeln!(out, "\n// {node_count} nodes, {edge_count} edges");
    out
}

fn render_edges_format(edges: &[(String, String)], node_count: usize, edge_count: usize) -> String {
    let mut out = String::new();
    for (src, tgt) in edges {
        let _ = writeln!(out, "{src} -> {tgt}");
    }
    let _ = writeln!(out, "// {node_count} nodes, {edge_count} edges");
    out
}

fn render_summary(edges: &[(String, String)], node_count: usize, edge_count: usize) -> String {
    let mut sources = HashSet::new();
    let mut targets = HashSet::new();
    for (src, tgt) in edges {
        sources.insert(src.as_str());
        targets.insert(tgt.as_str());
    }

    let all_nodes: HashSet<&str> = sources.union(&targets).copied().collect();
    let mut roots: Vec<&str> = all_nodes
        .iter()
        .filter(|n| !targets.contains(**n))
        .copied()
        .collect();
    roots.sort_unstable();
    let mut leaves: Vec<&str> = all_nodes
        .iter()
        .filter(|n| !sources.contains(**n))
        .copied()
        .collect();
    leaves.sort_unstable();

    let mut out = String::new();
    let _ = writeln!(out, "## Graph Summary\n");
    let _ = writeln!(out, "- **Nodes:** {node_count}");
    let _ = writeln!(out, "- **Edges:** {edge_count}");
    let _ = writeln!(out, "- **Roots (no incoming):** {}", format_list(&roots));
    let _ = writeln!(out, "- **Leaves (no outgoing):** {}", format_list(&leaves));
    out
}

fn format_list(items: &[&str]) -> String {
    const MAX_SHOW: usize = 5;
    if items.is_empty() {
        return "(none)".to_owned();
    }
    if items.len() <= MAX_SHOW {
        return items.join(", ");
    }
    let shown: Vec<&str> = items[..MAX_SHOW].to_vec();
    format!("{} (+{} more)", shown.join(", "), items.len() - MAX_SHOW)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::super::ExportFormat;
    use super::*;

    #[test]
    fn test_graph_export_needs_params() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_graph_export(&db, None, None, None, None, None).unwrap();
        assert!(result.contains("Provide either"));
    }

    #[test]
    fn test_graph_export_symbol_not_found() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_graph_export(&db, Some("nonexistent"), None, None, None, None).unwrap();
        assert!(result.contains("No symbol found"));
    }

    #[test]
    fn test_graph_export_file_empty() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_graph_export(&db, None, Some("src/"), None, None, None).unwrap();
        assert!(result.contains("digraph"));
        assert!(result.contains("0 nodes"));
    }

    #[test]
    fn test_graph_export_edges_format() {
        use crate::indexer::parser::{Symbol, SymbolKind, Visibility};
        use crate::indexer::store::store_symbols;
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "h1").unwrap();
        store_symbols(
            &db,
            file_id,
            &[
                Symbol {
                    name: "foo".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 1,
                    line_end: 3,
                    signature: "fn foo()".into(),
                    doc_comment: None,
                    body: Some("fn foo() { bar(); }".into()),
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "bar".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/lib.rs".into(),
                    line_start: 5,
                    line_end: 7,
                    signature: "fn bar()".into(),
                    doc_comment: None,
                    body: Some("fn bar() {}".into()),
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();
        let foo_id = db.get_symbol_id("foo", "src/lib.rs").unwrap().unwrap();
        let bar_id = db.get_symbol_id("bar", "src/lib.rs").unwrap().unwrap();
        db.insert_symbol_ref(foo_id, bar_id, "call", "high", Some(2))
            .unwrap();

        let result = handle_graph_export(
            &db,
            Some("foo"),
            None,
            Some(2),
            None,
            Some(ExportFormat::Edges),
        )
        .unwrap();
        assert!(
            result.contains("foo -> bar"),
            "Edges format should show 'foo -> bar', got:\n{result}"
        );
        assert!(
            !result.contains("digraph"),
            "Edges format should NOT contain DOT syntax, got:\n{result}"
        );
    }

    #[test]
    fn test_graph_export_summary_format() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_graph_export(
            &db,
            None,
            Some("src/"),
            None,
            None,
            Some(ExportFormat::Summary),
        )
        .unwrap();
        assert!(
            result.contains("Nodes") && result.contains("Edges"),
            "Summary should mention Nodes and Edges, got:\n{result}"
        );
    }
}
