use crate::db::Database;
use std::collections::{BTreeMap, VecDeque};
use std::fmt::Write;

pub fn handle_neighborhood(
    db: &Database,
    symbol_name: &str,
    depth: Option<i64>,
) -> Result<String, Box<dyn std::error::Error>> {
    let max_depth = usize::try_from(depth.unwrap_or(2).max(1)).unwrap_or(2);

    let syms = super::resolve_symbol(db, symbol_name)?;
    if syms.is_empty() {
        return Ok(format!("Symbol '{symbol_name}' not found."));
    }

    let base = symbol_name
        .split_once("::")
        .map_or(symbol_name, |(_, m)| m);

    // BFS outward (callees)
    let mut outward: BTreeMap<String, usize> = BTreeMap::new();
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();
    outward.insert(base.to_string(), 0);
    queue.push_back((base.to_string(), 0));
    while let Some((current, d)) = queue.pop_front() {
        if d >= max_depth {
            continue;
        }
        for (callee, _) in db.get_callees_by_name(&current)? {
            if !outward.contains_key(&callee) {
                outward.insert(callee.clone(), d + 1);
                queue.push_back((callee, d + 1));
            }
        }
    }

    // BFS inward (callers)
    let mut inward: BTreeMap<String, usize> = BTreeMap::new();
    inward.insert(base.to_string(), 0);
    queue.push_back((base.to_string(), 0));
    while let Some((current, d)) = queue.pop_front() {
        if d >= max_depth {
            continue;
        }
        for (caller, _) in db.get_callers_by_name(&current)? {
            if !inward.contains_key(&caller) {
                inward.insert(caller.clone(), d + 1);
                queue.push_back((caller, d + 1));
            }
        }
    }

    // Format output
    let mut output = String::new();
    let _ = writeln!(
        output,
        "## Neighborhood: {symbol_name} (depth {max_depth})\n"
    );

    let callers: Vec<_> = inward
        .iter()
        .filter(|(n, _)| n.as_str() != base)
        .collect();
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

    let callees: Vec<_> = outward
        .iter()
        .filter(|(n, _)| n.as_str() != base)
        .collect();
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

    fn setup_db_with_chain() -> Database {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();

        let alpha_id = insert_symbol(&db, file_id, "alpha");
        let center_id = insert_symbol(&db, file_id, "center");
        let beta_id = insert_symbol(&db, file_id, "beta");

        // alpha -> center
        db.insert_symbol_ref(alpha_id, center_id, "call").unwrap();
        // center -> beta
        db.insert_symbol_ref(center_id, beta_id, "call").unwrap();

        db
    }

    #[test]
    fn test_neighborhood_bidirectional() {
        let db = setup_db_with_chain();
        let result = handle_neighborhood(&db, "center", None).unwrap();

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
        let result = handle_neighborhood(&db, "nonexistent", None).unwrap();
        assert!(result.contains("not found"));
    }

    #[test]
    fn test_neighborhood_isolated() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();
        insert_symbol(&db, file_id, "lonely");

        let result = handle_neighborhood(&db, "lonely", None).unwrap();
        assert!(result.contains("### Center: lonely"));
        assert!(result.contains("No connections found"));
    }
}
