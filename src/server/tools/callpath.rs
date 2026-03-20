use crate::db::Database;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write;

/// Extract the base symbol name (without `Type::` prefix).
fn base_name(name: &str) -> &str {
    name.split_once("::").map_or(name, |(_, m)| m)
}

pub fn handle_callpath(
    db: &Database,
    from: &str,
    to: &str,
    max_depth: Option<i64>,
    all_paths: bool,
    max_paths: Option<i64>,
) -> Result<String, Box<dyn std::error::Error>> {
    let max_depth = usize::try_from(max_depth.unwrap_or(10).max(1))
        .unwrap_or(10);

    let from_syms = super::resolve_symbol(db, from)?;
    if from_syms.is_empty() {
        return Ok(format!("Source symbol '{from}' not found."));
    }
    let to_syms = super::resolve_symbol(db, to)?;
    if to_syms.is_empty() {
        return Ok(format!("Target symbol '{to}' not found."));
    }

    let from_name = base_name(from);
    let to_name = base_name(to);

    if all_paths {
        let max_paths = usize::try_from(max_paths.unwrap_or(5).max(1))
            .unwrap_or(5);
        handle_all_paths(db, from, to, from_name, to_name, max_depth, max_paths)
    } else {
        handle_shortest_path(db, from, to, from_name, to_name, max_depth)
    }
}

fn handle_shortest_path(
    db: &Database,
    from: &str,
    to: &str,
    from_name: &str,
    to_name: &str,
    max_depth: usize,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut visited: HashSet<String> = HashSet::new();
    let mut parent: HashMap<String, String> = HashMap::new();
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();

    visited.insert(from_name.to_string());
    queue.push_back((from_name.to_string(), 0));

    let mut found = false;
    while let Some((current, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }

        let callees = db.get_callees_by_name(&current)?;
        for (callee_name, _file) in callees {
            if visited.contains(&callee_name) {
                continue;
            }
            visited.insert(callee_name.clone());
            parent.insert(callee_name.clone(), current.clone());

            if callee_name == to_name {
                found = true;
                break;
            }
            queue.push_back((callee_name, depth + 1));
        }
        if found {
            break;
        }
    }

    let mut output = String::new();
    let _ = writeln!(output, "## Call Path: {from} → {to}\n");

    if !found {
        let _ = writeln!(
            output,
            "No call path found from `{from}` to `{to}` \
             within depth {max_depth}."
        );
        return Ok(output);
    }

    let mut path = vec![to_name.to_string()];
    let mut current = to_name.to_string();
    while let Some(prev) = parent.get(&current) {
        path.push(prev.clone());
        current = prev.clone();
    }
    path.reverse();

    let _ = writeln!(output, "**Path ({} hops):**\n", path.len() - 1);
    let _ = writeln!(output, "`{}`", path.join(" → "));

    let _ = writeln!(output, "\n**Locations:**\n");
    for name in &path {
        let syms = super::resolve_symbol(db, name)?;
        if let Some(sym) = syms.first() {
            let _ = writeln!(
                output,
                "- **{name}** ({}:{}-{})",
                sym.file_path, sym.line_start, sym.line_end
            );
        }
    }

    Ok(output)
}

fn find_all_paths(
    db: &Database,
    target: &str,
    max_depth: usize,
    max_paths: usize,
    current_path: &mut Vec<String>,
    visited: &mut HashSet<String>,
    results: &mut Vec<Vec<String>>,
) -> Result<(), Box<dyn std::error::Error>> {
    if results.len() >= max_paths || current_path.len() > max_depth {
        return Ok(());
    }

    let current = current_path
        .last()
        .ok_or("current_path must not be empty")?
        .clone();

    let callees = db.get_callees_by_name(&current)?;
    for (callee_name, _file) in callees {
        if results.len() >= max_paths {
            break;
        }
        if callee_name == target {
            let mut path = current_path.clone();
            path.push(callee_name);
            results.push(path);
            continue;
        }
        if visited.contains(&callee_name) {
            continue;
        }
        visited.insert(callee_name.clone());
        current_path.push(callee_name.clone());
        find_all_paths(
            db, target, max_depth, max_paths,
            current_path, visited, results,
        )?;
        current_path.pop();
        visited.remove(&callee_name);
    }

    Ok(())
}

fn handle_all_paths(
    db: &Database,
    from: &str,
    to: &str,
    from_name: &str,
    to_name: &str,
    max_depth: usize,
    max_paths: usize,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut current_path = vec![from_name.to_string()];
    let mut visited: HashSet<String> = HashSet::new();
    visited.insert(from_name.to_string());
    let mut results: Vec<Vec<String>> = Vec::new();

    find_all_paths(
        db, to_name, max_depth, max_paths,
        &mut current_path, &mut visited, &mut results,
    )?;

    let mut output = String::new();
    let _ = writeln!(output, "## All Call Paths: {from} → {to}\n");

    if results.is_empty() {
        let _ = writeln!(
            output,
            "No call paths found from `{from}` to `{to}` \
             within depth {max_depth}."
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
        db.insert_symbol_ref(a_id, b_id, "call").unwrap();
        db.insert_symbol_ref(a_id, c_id, "call").unwrap();
        db.insert_symbol_ref(b_id, d_id, "call").unwrap();
        db.insert_symbol_ref(c_id, d_id, "call").unwrap();

        db
    }

    #[test]
    fn test_callpath_shortest() {
        let db = setup_diamond();
        let result = handle_callpath(&db, "a", "d", None, false, None).unwrap();
        assert!(result.contains("## Call Path:"), "header missing: {result}");
        assert!(result.contains("hops"), "hop count missing: {result}");
    }

    #[test]
    fn test_callpath_all_paths() {
        let db = setup_diamond();
        let result = handle_callpath(&db, "a", "d", None, true, None).unwrap();
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
        let result = handle_callpath(&db, "x", "y", None, true, None).unwrap();
        assert!(
            result.contains("No call paths found"),
            "should report no paths: {result}"
        );
    }

    #[test]
    fn test_callpath_all_paths_max_paths() {
        let db = setup_diamond();
        let result = handle_callpath(&db, "a", "d", None, true, Some(1)).unwrap();
        assert!(
            result.contains("1 paths found"),
            "should respect max_paths=1: {result}"
        );
    }

    #[test]
    fn test_callpath_not_found() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_callpath(&db, "nonexistent", "other", None, false, None).unwrap();
        assert!(result.contains("not found"), "missing symbol: {result}");
    }
}
