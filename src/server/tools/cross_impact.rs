use crate::db::Database;
use crate::registry::Registry;
use std::fmt::Write;
use std::path::Path;

pub fn handle_cross_impact(
    registry: &Registry,
    primary_path: &Path,
    symbol_name: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let other_repos = registry.other_repos(primary_path);
    if other_repos.is_empty() {
        return Ok("No other repos registered.".into());
    }

    let (name, impl_type) = if let Some(idx) = symbol_name.find("::") {
        (&symbol_name[idx + 2..], Some(&symbol_name[..idx]))
    } else {
        (symbol_name, None)
    };

    let mut out = format!("## Cross-Repo Impact: `{symbol_name}`\n\n");
    let mut found_any = false;

    for repo in &other_repos {
        let db_path = repo.path.join(".illu/index.db");
        let Ok(db) = Database::open_readonly(&db_path) else {
            continue;
        };

        let refs = query_refs(&db, name, impl_type)?;

        if !refs.is_empty() {
            found_any = true;
            let _ = write!(
                out,
                "### {} ({}) \u{2014} {} reference(s)\n\n",
                repo.name,
                repo.path.display(),
                refs.len()
            );
            for (sname, simpl, file, line) in &refs {
                let qualified = match simpl {
                    Some(it) => format!("{it}::{sname}"),
                    None => sname.clone(),
                };
                let _ = writeln!(out, "- `{qualified}` ({file}:{line})");
            }
            out.push('\n');
        }
    }

    if !found_any {
        out.push_str("No references to this symbol found in other repos.\n");
    }
    Ok(out)
}

type RefRow = (String, Option<String>, String, i64);

fn query_refs(
    db: &Database,
    name: &str,
    impl_type: Option<&str>,
) -> Result<Vec<RefRow>, Box<dyn std::error::Error>> {
    let sql = if impl_type.is_some() {
        "SELECT DISTINCT s.name, s.impl_type, f.path, s.line_start
         FROM symbol_refs sr
         JOIN symbols s ON sr.source_symbol_id = s.id
         JOIN symbols t ON sr.target_symbol_id = t.id
         JOIN files f ON s.file_id = f.id
         WHERE t.name = ?1 AND t.impl_type = ?2
         ORDER BY f.path, s.line_start LIMIT 50"
    } else {
        "SELECT DISTINCT s.name, s.impl_type, f.path, s.line_start
         FROM symbol_refs sr
         JOIN symbols s ON sr.source_symbol_id = s.id
         JOIN symbols t ON sr.target_symbol_id = t.id
         JOIN files f ON s.file_id = f.id
         WHERE t.name = ?1
         ORDER BY f.path, s.line_start LIMIT 50"
    };

    let mut stmt = db.conn.prepare(sql)?;
    let rows = if let Some(it) = impl_type {
        stmt.query_map(rusqlite::params![name, it], map_row)?
            .filter_map(std::result::Result::ok)
            .collect()
    } else {
        stmt.query_map(rusqlite::params![name], map_row)?
            .filter_map(std::result::Result::ok)
            .collect()
    };
    Ok(rows)
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RefRow> {
    Ok((
        row.get::<_, String>(0)?,
        row.get::<_, Option<String>>(1)?,
        row.get::<_, String>(2)?,
        row.get::<_, i64>(3)?,
    ))
}
