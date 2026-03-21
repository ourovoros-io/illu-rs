use crate::db::Database;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fmt::Write;

pub fn handle_neighborhood(
    db: &Database,
    symbol_name: &str,
    depth: Option<i64>,
    direction: Option<&str>,
    format: Option<&str>,
    exclude_tests: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let max_depth = usize::try_from(depth.unwrap_or(2).max(1)).unwrap_or(2);
    let dir = direction.unwrap_or("both");
    let fmt = format.unwrap_or("list");

    let syms = super::resolve_symbol(db, symbol_name)?;
    if syms.is_empty() {
        return Ok(format!(
            "Symbol '{symbol_name}' not found.\n\
            Try `Type::method` syntax for methods \
            (e.g. `Database::new`), or use `query` to search."
        ));
    }

    let base = symbol_name.split_once("::").map_or(symbol_name, |(_, m)| m);

    if fmt == "tree" {
        if dir == "both" {
            let mut output = String::new();
            let _ = writeln!(output, "## Neighborhood: {symbol_name}\n");
            let _ = writeln!(output, "### Callers (upstream)\n");
            let up = render_tree_output(db, symbol_name, base, "up", max_depth, exclude_tests)?;
            // Skip the header from render_tree_output
            if let Some(body) = up.split_once("\n\n") {
                output.push_str(body.1);
            }
            let _ = writeln!(output, "### Callees (downstream)\n");
            let down = render_tree_output(db, symbol_name, base, "down", max_depth, exclude_tests)?;
            if let Some(body) = down.split_once("\n\n") {
                output.push_str(body.1);
            }
            return Ok(output);
        }
        return render_tree_output(db, symbol_name, base, dir, max_depth, exclude_tests);
    }

    let include_down = dir == "both" || dir == "down";
    let include_up = dir == "both" || dir == "up";

    let outward = if include_down {
        bfs_collect(db, base, max_depth, Direction::Down, exclude_tests)?
    } else {
        BTreeMap::new()
    };

    let inward = if include_up {
        bfs_collect(db, base, max_depth, Direction::Up, exclude_tests)?
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

type BfsEntry = (usize, String);

fn bfs_collect(
    db: &Database,
    base: &str,
    max_depth: usize,
    direction: Direction,
    exclude_tests: bool,
) -> Result<BTreeMap<String, BfsEntry>, Box<dyn std::error::Error>> {
    let mut visited: BTreeMap<String, BfsEntry> = BTreeMap::new();
    // Track (name, file) for file-qualified BFS to avoid name collisions
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    let mut queue: VecDeque<(String, String, usize)> = VecDeque::new();

    // Seed with all definitions of the base symbol
    let syms = super::resolve_symbol(db, base)?;
    visited.insert(base.to_string(), (0, String::new()));
    for sym in &syms {
        seen.insert((sym.name.clone(), sym.file_path.clone()));
        queue.push_back((sym.name.clone(), sym.file_path.clone(), 0));
    }

    while let Some((current_name, current_file, d)) = queue.pop_front() {
        if d >= max_depth {
            continue;
        }
        let neighbors: Vec<(String, String)> = match direction {
            Direction::Down => {
                let callees = db.get_callees(&current_name, &current_file, exclude_tests)?;
                callees
                    .into_iter()
                    .filter(|c| c.kind != "const" && c.kind != "static")
                    .map(|c| (c.name, c.file_path))
                    .collect()
            }
            Direction::Up => db.get_callers_by_name(&current_name, Some("high"), exclude_tests)?,
        };
        for (neighbor, file_path) in neighbors {
            if super::NOISY_CALLEES.contains(&neighbor.as_str()) {
                continue;
            }
            let key = (neighbor.clone(), file_path.clone());
            if !seen.insert(key) {
                continue;
            }
            if !visited.contains_key(&neighbor) {
                visited.insert(neighbor.clone(), (d + 1, file_path.clone()));
            }
            queue.push_back((neighbor, file_path, d + 1));
        }
    }
    Ok(visited)
}

fn format_list_output(
    symbol_name: &str,
    base: &str,
    syms: &[crate::db::StoredSymbol],
    inward: &BTreeMap<String, BfsEntry>,
    outward: &BTreeMap<String, BfsEntry>,
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
        for (name, (d, file)) in &callers {
            let _ = writeln!(output, "- **{name}** ({file}) — depth {d}");
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
        for (name, (d, file)) in &callees {
            let _ = writeln!(output, "- **{name}** ({file}) — depth {d}");
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
    exclude_tests: bool,
}

impl<'a> TreeRenderer<'a> {
    fn new(db: &'a Database, max_depth: usize, use_callers: bool, exclude_tests: bool) -> Self {
        Self {
            db,
            output: String::new(),
            max_depth,
            visited: HashSet::new(),
            use_callers,
            exclude_tests,
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
            self.db
                .get_callers_by_name(name, Some("high"), self.exclude_tests)?
        } else {
            self.db
                .get_callees_by_name(name, Some("high"), self.exclude_tests)?
        };

        let child_prefix = if is_root {
            String::new()
        } else if is_last {
            format!("{prefix}    ")
        } else {
            format!("{prefix}│   ")
        };

        let children: Vec<_> = children
            .into_iter()
            .filter(|(name, _)| !super::NOISY_CALLEES.contains(&name.as_str()))
            .collect();
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
    exclude_tests: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let label = if dir == "down" {
        "Call Tree"
    } else {
        "Caller Tree"
    };
    let mut renderer = TreeRenderer::new(db, max_depth, dir == "up", exclude_tests);
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
        db.insert_symbol_ref(alpha_id, center_id, "call", "high", None)
            .unwrap();
        // center -> beta
        db.insert_symbol_ref(center_id, beta_id, "call", "high", None)
            .unwrap();

        db
    }

    #[test]
    fn test_neighborhood_bidirectional() {
        let db = setup_db_with_chain();
        let result = handle_neighborhood(&db, "center", None, None, None, false).unwrap();

        assert!(result.contains("## Neighborhood: center"));
        assert!(result.contains("### Callers (upstream)"));
        assert!(result.contains("**alpha** (src/lib.rs)"));
        assert!(result.contains("### Callees (downstream)"));
        assert!(result.contains("**beta** (src/lib.rs)"));
        assert!(result.contains("### Center: center"));
    }

    #[test]
    fn test_neighborhood_not_found() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_neighborhood(&db, "nonexistent", None, None, None, false).unwrap();
        assert!(result.contains("not found"));
    }

    #[test]
    fn test_neighborhood_isolated() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        insert_symbol(&db, file_id, "lonely");

        let result = handle_neighborhood(&db, "lonely", None, None, None, false).unwrap();
        assert!(result.contains("### Center: lonely"));
        assert!(result.contains("No connections found"));
    }

    #[test]
    fn test_neighborhood_direction_down() {
        let db = setup_db_with_chain();
        let result = handle_neighborhood(&db, "center", None, Some("down"), None, false).unwrap();

        assert!(result.contains("### Callees (downstream)"));
        assert!(result.contains("**beta** (src/lib.rs)"));
        assert!(!result.contains("### Callers (upstream)"));
        assert!(!result.contains("**alpha**"));
    }

    #[test]
    fn test_neighborhood_direction_up() {
        let db = setup_db_with_chain();
        let result = handle_neighborhood(&db, "center", None, Some("up"), None, false).unwrap();

        assert!(result.contains("### Callers (upstream)"));
        assert!(result.contains("**alpha** (src/lib.rs)"));
        assert!(!result.contains("### Callees (downstream)"));
        assert!(!result.contains("**beta**"));
    }

    #[test]
    fn test_neighborhood_tree_format() {
        let db = setup_db_with_chain();
        let result =
            handle_neighborhood(&db, "center", Some(2), Some("down"), Some("tree"), false).unwrap();

        assert!(result.contains("## Call Tree: center"));
        assert!(result.contains("**center**"));
        assert!(result.contains("└── beta"));
    }

    #[test]
    fn test_neighborhood_tree_format_up() {
        let db = setup_db_with_chain();
        let result =
            handle_neighborhood(&db, "center", Some(2), Some("up"), Some("tree"), false).unwrap();

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
        db.insert_symbol_ref(root_id, child_a_id, "call", "high", None)
            .unwrap();
        db.insert_symbol_ref(root_id, child_b_id, "call", "high", None)
            .unwrap();
        db.insert_symbol_ref(child_a_id, grandchild_id, "call", "high", None)
            .unwrap();

        let result =
            handle_neighborhood(&db, "root", Some(2), Some("down"), Some("tree"), false).unwrap();

        assert!(result.contains("**root**"));
        assert!(result.contains("├── "));
        assert!(result.contains("└── "));
        assert!(result.contains("grandchild"));
    }

    fn insert_test_symbol(db: &Database, file_id: crate::db::FileId, name: &str) -> SymbolId {
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature, is_test) \
                 VALUES (?1, ?2, 'function', 'public', \
                         1, 10, ?3, 1)",
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

    #[test]
    fn test_neighborhood_exclude_tests() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();

        let prod_caller = insert_symbol(&db, file_id, "prod_caller");
        let target = insert_symbol(&db, file_id, "target");
        let test_caller = insert_test_symbol(&db, file_id, "test_caller");

        // prod_caller -> target, test_caller -> target
        db.insert_symbol_ref(prod_caller, target, "call", "high", None)
            .unwrap();
        db.insert_symbol_ref(test_caller, target, "call", "high", None)
            .unwrap();

        // Without exclude_tests: both callers visible
        let result = handle_neighborhood(&db, "target", None, Some("up"), None, false).unwrap();
        assert!(result.contains("prod_caller"), "prod caller present");
        assert!(result.contains("test_caller"), "test caller present");

        // With exclude_tests: only production caller visible
        let result = handle_neighborhood(&db, "target", None, Some("up"), None, true).unwrap();
        assert!(result.contains("prod_caller"), "prod caller still present");
        assert!(
            !result.contains("test_caller"),
            "test caller should be excluded"
        );
    }

    fn insert_symbol_with_kind(
        db: &Database,
        file_id: crate::db::FileId,
        name: &str,
        kind: &str,
        line_start: i64,
        line_end: i64,
    ) -> SymbolId {
        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, ?2, ?3, 'public', ?4, ?5, ?6)",
                params![
                    file_id,
                    name,
                    kind,
                    line_start,
                    line_end,
                    format!("fn {name}()")
                ],
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

    #[test]
    fn test_neighborhood_excludes_constants() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/a.rs", "hash1").unwrap();

        let caller_id = insert_symbol_with_kind(&db, file_id, "caller_fn", "function", 1, 5);
        let const_id = insert_symbol_with_kind(&db, file_id, "MY_CONST", "const", 7, 7);
        let fn_id = insert_symbol_with_kind(&db, file_id, "real_fn", "function", 9, 12);

        db.insert_symbol_ref(caller_id, const_id, "call", "high", None)
            .unwrap();
        db.insert_symbol_ref(caller_id, fn_id, "call", "high", None)
            .unwrap();

        let result =
            handle_neighborhood(&db, "caller_fn", Some(1), Some("down"), None, false).unwrap();
        assert!(result.contains("real_fn"), "should show function callees");
        assert!(
            !result.contains("MY_CONST"),
            "should NOT show constant callees in call graph"
        );
    }
}
