use crate::db::Database;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write;

pub fn handle_callpath(
    db: &Database,
    from: &str,
    to: &str,
    max_depth: Option<i64>,
    all_paths: bool,
    max_paths: Option<i64>,
    exclude_tests: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let max_depth = usize::try_from(max_depth.unwrap_or(10).max(1)).unwrap_or(10);

    let from_syms = super::resolve_symbol(db, from)?;
    if from_syms.is_empty() {
        return Ok(super::symbol_not_found(db, from));
    }
    let to_syms = super::resolve_symbol(db, to)?;
    if to_syms.is_empty() {
        return Ok(super::symbol_not_found(db, to));
    }

    let to_name = super::base_name(to);

    if all_paths {
        let max_paths = usize::try_from(max_paths.unwrap_or(5).max(1)).unwrap_or(5);
        let cfg = DfsConfig {
            max_depth,
            max_paths,
            exclude_tests,
        };
        handle_all_paths(db, from, to, &from_syms, to_name, &cfg)
    } else {
        handle_shortest_path(db, from, to, &from_syms, to_name, max_depth, exclude_tests)
    }
}

/// BFS node: `(symbol_name, file_path)` to disambiguate symbols with the same name.
type BfsNode = (String, String);

fn handle_shortest_path(
    db: &Database,
    from: &str,
    to: &str,
    from_syms: &[crate::db::StoredSymbol],
    to_name: &str,
    max_depth: usize,
    exclude_tests: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut visited: HashSet<BfsNode> = HashSet::new();
    let mut parent: HashMap<BfsNode, BfsNode> = HashMap::new();
    let mut queue: VecDeque<(BfsNode, usize)> = VecDeque::new();

    // Seed BFS with all definitions of the source symbol
    for sym in from_syms {
        let node = (sym.name.clone(), sym.file_path.clone());
        visited.insert(node.clone());
        queue.push_back((node, 0));
    }

    let mut found_node: Option<BfsNode> = None;
    while let Some((current, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }

        // Use file-specific callee lookup to avoid ambiguity
        let callees = db.callees(&current.0, &current.1, exclude_tests)?;
        for callee in callees {
            let node = (callee.name.clone(), callee.file_path.clone());
            if visited.contains(&node) {
                continue;
            }
            visited.insert(node.clone());
            parent.insert(node.clone(), current.clone());

            if callee.name == to_name {
                found_node = Some(node);
                break;
            }
            queue.push_back((node, depth + 1));
        }
        if found_node.is_some() {
            break;
        }
    }

    let mut output = String::new();
    let _ = writeln!(output, "## Call Path: {from} → {to}\n");

    let Some(target_node) = found_node else {
        let _ = writeln!(
            output,
            "No call path found from `{from}` to `{to}` \
             within depth {max_depth}."
        );
        return Ok(output);
    };

    let mut path = vec![target_node.clone()];
    let mut current = target_node;
    while let Some(prev) = parent.get(&current) {
        path.push(prev.clone());
        current = prev.clone();
    }
    path.reverse();

    let _ = writeln!(output, "**Path ({} hops):**\n", path.len() - 1);
    let names: Vec<&str> = path.iter().map(|(name, _)| name.as_str()).collect();
    let _ = writeln!(output, "`{}`", names.join(" → "));

    let _ = writeln!(output, "\n**Locations:**\n");
    for (name, file) in &path {
        let syms = super::resolve_symbol(db, name)?;
        let sym = syms
            .iter()
            .find(|s| s.file_path == *file)
            .or_else(|| syms.first());
        if let Some(s) = sym {
            let _ = writeln!(
                output,
                "- **{name}** ({}:{}-{})",
                s.file_path, s.line_start, s.line_end
            );
        }
    }

    Ok(output)
}

struct DfsConfig {
    max_depth: usize,
    max_paths: usize,
    exclude_tests: bool,
}

fn find_all_paths(
    db: &Database,
    target: &str,
    cfg: &DfsConfig,
    current_path: &mut Vec<BfsNode>,
    visited: &mut HashSet<BfsNode>,
    results: &mut Vec<Vec<String>>,
) -> Result<(), Box<dyn std::error::Error>> {
    if results.len() >= cfg.max_paths || current_path.len() > cfg.max_depth {
        return Ok(());
    }

    let current = current_path
        .last()
        .ok_or("current_path must not be empty")?
        .clone();

    let callees = db.callees(&current.0, &current.1, cfg.exclude_tests)?;
    for callee in callees {
        if results.len() >= cfg.max_paths {
            break;
        }
        let node = (callee.name.clone(), callee.file_path.clone());
        if callee.name == target {
            let mut path: Vec<String> = current_path.iter().map(|(name, _)| name.clone()).collect();
            path.push(callee.name);
            results.push(path);
            continue;
        }
        if visited.contains(&node) {
            continue;
        }
        visited.insert(node.clone());
        current_path.push(node.clone());
        find_all_paths(db, target, cfg, current_path, visited, results)?;
        current_path.pop();
        visited.remove(&node);
    }

    Ok(())
}

fn handle_all_paths(
    db: &Database,
    from: &str,
    to: &str,
    from_syms: &[crate::db::StoredSymbol],
    to_name: &str,
    cfg: &DfsConfig,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut results: Vec<Vec<String>> = Vec::new();

    for sym in from_syms {
        let start_node = (sym.name.clone(), sym.file_path.clone());
        let mut current_path = vec![start_node.clone()];
        let mut visited: HashSet<BfsNode> = HashSet::new();
        visited.insert(start_node);
        find_all_paths(
            db,
            to_name,
            cfg,
            &mut current_path,
            &mut visited,
            &mut results,
        )?;
    }

    let mut output = String::new();
    let _ = writeln!(output, "## All Call Paths: {from} → {to}\n");

    if results.is_empty() {
        let _ = writeln!(
            output,
            "No call paths found from `{from}` to `{to}` \
             within depth {}.",
            cfg.max_depth
        );
        return Ok(output);
    }

    results.sort_by_key(Vec::len);

    let _ = writeln!(output, "**{} paths found:**\n", results.len());
    for (i, path) in results.iter().enumerate() {
        let _ = writeln!(
            output,
            "{}. `{}` ({} hops)",
            i + 1,
            path.join(" → "),
            path.len() - 1,
        );
    }

    Ok(output)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::db::SymbolId;
    use rusqlite::params;

    fn insert_symbol(db: &Database, file_id: crate::db::FileId, name: &str) -> SymbolId {
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, ?2, 'function', 'public', \
                         1, 10, ?3)",
                params![file_id, name, format!("fn {name}()")],
            )
            .unwrap();
        let id = SymbolId(db.conn.last_insert_rowid());
        db.conn
            .execute(
                "INSERT INTO symbols_fts (rowid, name, signature, doc_comment) \
                 VALUES (?1, ?2, ?3, '')",
                params![id, name, format!("fn {name}()")],
            )
            .unwrap();
        id
    }

    fn setup_diamond() -> Database {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();

        let a_id = insert_symbol(&db, file_id, "a");
        let b_id = insert_symbol(&db, file_id, "b");
        let c_id = insert_symbol(&db, file_id, "c");
        let d_id = insert_symbol(&db, file_id, "d");

        // Diamond: a -> b -> d, a -> c -> d
        db.insert_symbol_ref(a_id, b_id, "call", "high", None)
            .unwrap();
        db.insert_symbol_ref(a_id, c_id, "call", "high", None)
            .unwrap();
        db.insert_symbol_ref(b_id, d_id, "call", "high", None)
            .unwrap();
        db.insert_symbol_ref(c_id, d_id, "call", "high", None)
            .unwrap();

        db
    }

    #[test]
    fn test_callpath_shortest() {
        let db = setup_diamond();
        let result = handle_callpath(&db, "a", "d", None, false, None, false).unwrap();
        assert!(result.contains("## Call Path:"), "header missing: {result}");
        assert!(result.contains("hops"), "hop count missing: {result}");
    }

    #[test]
    fn test_callpath_all_paths() {
        let db = setup_diamond();
        let result = handle_callpath(&db, "a", "d", None, true, None, false).unwrap();
        assert!(
            result.contains("2 paths found"),
            "should find two paths: {result}"
        );
        assert!(result.contains("## All Call Paths:"), "header: {result}");
    }

    #[test]
    fn test_callpath_all_paths_none_found() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        insert_symbol(&db, file_id, "x");
        insert_symbol(&db, file_id, "y");
        // No edges between x and y
        let result = handle_callpath(&db, "x", "y", None, true, None, false).unwrap();
        assert!(
            result.contains("No call paths found"),
            "should report no paths: {result}"
        );
    }

    #[test]
    fn test_callpath_all_paths_max_paths() {
        let db = setup_diamond();
        let result = handle_callpath(&db, "a", "d", None, true, Some(1), false).unwrap();
        assert!(
            result.contains("1 paths found"),
            "should respect max_paths=1: {result}"
        );
    }

    #[test]
    fn test_callpath_not_found() {
        let db = Database::open_in_memory().unwrap();
        let result =
            handle_callpath(&db, "nonexistent", "other", None, false, None, false).unwrap();
        assert!(
            result.contains("No symbol found"),
            "missing symbol: {result}"
        );
    }
}
