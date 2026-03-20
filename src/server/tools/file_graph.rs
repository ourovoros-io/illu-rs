use crate::db::Database;
use std::collections::BTreeMap;
use std::fmt::Write;

pub fn handle_file_graph(db: &Database, path: &str) -> Result<String, Box<dyn std::error::Error>> {
    let edges = db.get_file_dependencies(path)?;

    let mut output = String::new();
    let _ = writeln!(output, "## File Dependency Graph: {path}\n");

    if edges.is_empty() {
        let _ = writeln!(output, "No file dependencies found under '{path}'.");
        return Ok(output);
    }

    let mut graph: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (source, target) in &edges {
        graph.entry(source).or_default().push(target);
    }

    for (source, targets) in &graph {
        let _ = writeln!(output, "### {source}\n");
        for target in targets {
            let _ = writeln!(output, "- \u{2192} {target}");
        }
        let _ = writeln!(output);
    }

    let file_count = graph.len();
    let edge_count = edges.len();
    let _ = writeln!(
        output,
        "---\n**{file_count} files, {edge_count} dependency edges**"
    );

    Ok(output)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::db::SymbolId;
    use rusqlite::params;

    fn setup_two_file_db() -> Database {
        let db = Database::open_in_memory().unwrap();
        let f1 = db.insert_file("src/server/mod.rs", "h1").unwrap();
        let f2 = db.insert_file("src/db.rs", "h2").unwrap();

        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, 'handle', 'function', 'public', 1, 5, 'fn handle()')",
                params![f1],
            )
            .unwrap();
        let src_id = SymbolId(db.conn.last_insert_rowid());

        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, 'query', 'function', 'public', 1, 5, 'fn query()')",
                params![f2],
            )
            .unwrap();
        let tgt_id = SymbolId(db.conn.last_insert_rowid());

        db.insert_symbol_ref(src_id, tgt_id, "call").unwrap();
        db
    }

    #[test]
    fn test_file_graph_shows_edges() {
        let db = setup_two_file_db();
        let result = handle_file_graph(&db, "src/").unwrap();

        assert!(result.contains("## File Dependency Graph: src/"));
        assert!(result.contains("src/server/mod.rs"));
        assert!(result.contains("\u{2192} src/db.rs"));
        assert!(result.contains("1 files, 1 dependency edges"));
    }

    #[test]
    fn test_file_graph_empty() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_file_graph(&db, "src/").unwrap();

        assert!(result.contains("No file dependencies found under 'src/'"));
    }

    #[test]
    fn test_file_graph_excludes_self_refs() {
        let db = Database::open_in_memory().unwrap();
        let f1 = db.insert_file("src/lib.rs", "h1").unwrap();

        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, 'foo', 'function', 'public', 1, 5, 'fn foo()')",
                params![f1],
            )
            .unwrap();
        let s1 = SymbolId(db.conn.last_insert_rowid());

        db.conn
            .execute(
                "INSERT INTO symbols \
                 (file_id, name, kind, visibility, \
                  line_start, line_end, signature) \
                 VALUES (?1, 'bar', 'function', 'public', 6, 10, 'fn bar()')",
                params![f1],
            )
            .unwrap();
        let s2 = SymbolId(db.conn.last_insert_rowid());

        db.insert_symbol_ref(s1, s2, "call").unwrap();

        let result = handle_file_graph(&db, "src/").unwrap();
        assert!(result.contains("No file dependencies found"));
    }
}
