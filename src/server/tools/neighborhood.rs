use crate::db::Database;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fmt::Write;

pub fn handle_neighborhood(
    db: &Database,
    symbol_name: &str,
    depth: Option<i64>,
    direction: Option<&str>,
    format: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let max_depth = usize::try_from(depth.unwrap_or(2).max(1)).unwrap_or(2);
    let dir = direction.unwrap_or("both");
    let fmt = format.unwrap_or("list");

    let syms = super::resolve_symbol(db, symbol_name)?;
    if syms.is_empty() {
        return Ok(format!("Symbol '{symbol_name}' not found."));
    }

    let base = symbol_name.split_once("::").map_or(symbol_name, |(_, m)| m);

    // Tree format only works for single-direction; fall back to list for "both"
    if fmt == "tree" && dir != "both" {
        return render_tree_output(db, symbol_name, base, dir, max_depth);
    }

    let include_down = dir == "both" || dir == "down";
    let include_up = dir == "both" || dir == "up";

    let outward = if include_down {
        bfs_collect(db, base, max_depth, Direction::Down)?
    } else {
        BTreeMap::new()
    };

    let inward = if include_up {
        bfs_collect(db, base, max_depth, Direction::Up)?
    } else {
        BTreeMap::new()
    };

    Ok(format_list_output(
        symbol_name,
        base,
        &syms,
        &inward,
        &outward,
        max_depth,
    ))
}

#[derive(Clone, Copy)]
enum Direction {
    Up,
    Down,
}

fn bfs_collect(
    db: &Database,
    base: &str,
    max_depth: usize,
    direction: Direction,
) -> Result<BTreeMap<String, usize>, Box<dyn std::error::Error>> {
    let mut visited: BTreeMap<String, usize> = BTreeMap::new();
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();
    visited.insert(base.to_string(), 0);
    queue.push_back((base.to_string(), 0));
    while let Some((current, d)) = queue.pop_front() {
        if d >= max_depth {
            continue;
        }
        let neighbors = match direction {
            Direction::Down => db.get_callees_by_name(&current, Some("high"))?,
            Direction::Up => db.get_callers_by_name(&current, Some("high"))?,
        };
        for (neighbor, _) in neighbors {
            if !visited.contains_key(&neighbor) {
                visited.insert(neighbor.clone(), d + 1);
                queue.push_back((neighbor, d + 1));
            }
        }
    }
    Ok(visited)
}

fn format_list_output(
    symbol_name: &str,
    base: &str,
    syms: &[crate::db::StoredSymbol],
    inward: &BTreeMap<String, usize>,
    outward: &BTreeMap<String, usize>,
    max_depth: usize,
) -> String {
    let mut output = String::new();
    let _ = writeln!(
        output,
        "## Neighborhood: {symbol_name} (depth {max_depth})\n"
    );

    let callers: Vec<_> = inward.iter().filter(|(n, _)| n.as_str() != base).collect();
    if !callers.is_empty() {
        let _ = writeln!(output, "### Callers (upstream)\n");
        for (name, d) in &callers {
            let _ = writeln!(output, "- **{name}** (depth {d})");
        }
        let _ = writeln!(output);
    }

    let _ = writeln!(output, "### Center: {base}\n");
    if let Some(sym) = syms.first() {
        let _ = writeln!(
            output,
            "- {}:{}-{} — `{}`",
            sym.file_path, sym.line_start, sym.line_end, sym.signature
        );
    }
    let _ = writeln!(output);

    let callees: Vec<_> = outward.iter().filter(|(n, _)| n.as_str() != base).collect();
    if !callees.is_empty() {
        let _ = writeln!(output, "### Callees (downstream)\n");
        for (name, d) in &callees {
            let _ = writeln!(output, "- **{name}** (depth {d})");
        }
        let _ = writeln!(output);
    }

    if callers.is_empty() && callees.is_empty() {
        let _ = writeln!(output, "No connections found within depth {max_depth}.");
    }

    output
}

struct TreeRenderer<'a> {
    db: &'a Database,
    output: String,
    max_depth: usize,
    visited: HashSet<String>,
    use_callers: bool,
}

impl<'a> TreeRenderer<'a> {
    fn new(db: &'a Database, max_depth: usize, use_callers: bool) -> Self {
        Self {
            db,
            output: String::new(),
            max_depth,
            visited: HashSet::new(),
            use_callers,
        }
    }

    fn render(
        &mut self,
        name: &str,
        depth: usize,
        prefix: &str,
        is_last: bool,
        is_root: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if is_root {
            let _ = writeln!(self.output, "**{name}**");
        } else {
            let connector = if is_last { "└── " } else { "├── " };
            let _ = writeln!(self.output, "{prefix}{connector}{name}");
        }

        if depth >= self.max_depth || !self.visited.insert(name.to_string()) {
            return Ok(());
        }

        let children = if self.use_callers {
            self.db.get_callers_by_name(name, Some("high"))?
        } else {
            self.db.get_callees_by_name(name, Some("high"))?
        };

        let child_prefix = if is_root {
            String::new()
        } else if is_last {
            format!("{prefix}    ")
        } else {
            format!("{prefix}│   ")
        };

        for (i, (child, _)) in children.iter().enumerate() {
            let last = i == children.len() - 1;
            self.render(child, depth + 1, &child_prefix, last, false)?;
        }

        self.visited.remove(name);
        Ok(())
    }
}

fn render_tree_output(
    db: &Database,
    symbol_name: &str,
    base: &str,
    dir: &str,
    max_depth: usize,
) -> Result<String, Box<dyn std::error::Error>> {
    let label = if dir == "down" {
        "Call Tree"
    } else {
        "Caller Tree"
    };
    let mut renderer = TreeRenderer::new(db, max_depth, dir == "up");
    let _ = writeln!(
        renderer.output,
        "## {label}: {symbol_name} (depth {max_depth})\n"
    );
    renderer.render(base, 0, "", true, true)?;
    Ok(renderer.output)
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

    fn setup_db_with_chain() -> Database {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();

        let alpha_id = insert_symbol(&db, file_id, "alpha");
        let center_id = insert_symbol(&db, file_id, "center");
        let beta_id = insert_symbol(&db, file_id, "beta");

        // alpha -> center
        db.insert_symbol_ref(alpha_id, center_id, "call", "high")
            .unwrap();
        // center -> beta
        db.insert_symbol_ref(center_id, beta_id, "call", "high")
            .unwrap();

        db
    }

    #[test]
    fn test_neighborhood_bidirectional() {
        let db = setup_db_with_chain();
        let result = handle_neighborhood(&db, "center", None, None, None).unwrap();

        assert!(result.contains("## Neighborhood: center"));
        assert!(result.contains("### Callers (upstream)"));
        assert!(result.contains("**alpha**"));
        assert!(result.contains("### Callees (downstream)"));
        assert!(result.contains("**beta**"));
        assert!(result.contains("### Center: center"));
    }

    #[test]
    fn test_neighborhood_not_found() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_neighborhood(&db, "nonexistent", None, None, None).unwrap();
        assert!(result.contains("not found"));
    }

    #[test]
    fn test_neighborhood_isolated() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        insert_symbol(&db, file_id, "lonely");

        let result = handle_neighborhood(&db, "lonely", None, None, None).unwrap();
        assert!(result.contains("### Center: lonely"));
        assert!(result.contains("No connections found"));
    }

    #[test]
    fn test_neighborhood_direction_down() {
        let db = setup_db_with_chain();
        let result = handle_neighborhood(&db, "center", None, Some("down"), None).unwrap();

        assert!(result.contains("### Callees (downstream)"));
        assert!(result.contains("**beta**"));
        assert!(!result.contains("### Callers (upstream)"));
        assert!(!result.contains("**alpha**"));
    }

    #[test]
    fn test_neighborhood_direction_up() {
        let db = setup_db_with_chain();
        let result = handle_neighborhood(&db, "center", None, Some("up"), None).unwrap();

        assert!(result.contains("### Callers (upstream)"));
        assert!(result.contains("**alpha**"));
        assert!(!result.contains("### Callees (downstream)"));
        assert!(!result.contains("**beta**"));
    }

    #[test]
    fn test_neighborhood_tree_format() {
        let db = setup_db_with_chain();
        let result =
            handle_neighborhood(&db, "center", Some(2), Some("down"), Some("tree")).unwrap();

        assert!(result.contains("## Call Tree: center"));
        assert!(result.contains("**center**"));
        assert!(result.contains("└── beta"));
    }

    #[test]
    fn test_neighborhood_tree_format_up() {
        let db = setup_db_with_chain();
        let result = handle_neighborhood(&db, "center", Some(2), Some("up"), Some("tree")).unwrap();

        assert!(result.contains("## Caller Tree: center"));
        assert!(result.contains("**center**"));
        assert!(result.contains("└── alpha"));
    }

    #[test]
    fn test_neighborhood_tree_with_branching() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();

        let root_id = insert_symbol(&db, file_id, "root");
        let child_a_id = insert_symbol(&db, file_id, "child_a");
        let child_b_id = insert_symbol(&db, file_id, "child_b");
        let grandchild_id = insert_symbol(&db, file_id, "grandchild");

        // root -> child_a, root -> child_b, child_a -> grandchild
        db.insert_symbol_ref(root_id, child_a_id, "call", "high")
            .unwrap();
        db.insert_symbol_ref(root_id, child_b_id, "call", "high")
            .unwrap();
        db.insert_symbol_ref(child_a_id, grandchild_id, "call", "high")
            .unwrap();

        let result = handle_neighborhood(&db, "root", Some(2), Some("down"), Some("tree")).unwrap();

        assert!(result.contains("**root**"));
        assert!(result.contains("├── "));
        assert!(result.contains("└── "));
        assert!(result.contains("grandchild"));
    }
}
