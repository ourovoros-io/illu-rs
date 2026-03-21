use crate::db::Database;
use crate::registry::Registry;
use std::fmt::Write;
use std::path::Path;

pub fn handle_repos(
    registry: &Registry,
    primary_path: &Path,
) -> Result<String, Box<dyn std::error::Error>> {
    if registry.repos.is_empty() {
        return Ok("No repos registered. Start illu in a repo to auto-register it.".into());
    }

    let mut out = String::from("## Registered Repos\n\n");
    out.push_str("| Repo | Path | Status | Symbols |\n");
    out.push_str("|------|------|--------|---------|\n");

    for repo in &registry.repos {
        let is_primary = repo.path == primary_path;
        let db_path = repo.path.join(".illu/index.db");

        let (status, symbols) = if !repo.path.exists() {
            ("missing".to_string(), "\u{2014}".to_string())
        } else if !db_path.exists() {
            ("no index".to_string(), "\u{2014}".to_string())
        } else if let Ok(db) = Database::open_readonly(&db_path) {
            let count: i64 = db
                .conn
                .query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))
                .unwrap_or(0);
            let status = if is_primary { "active" } else { "indexed" };
            (status.to_string(), count.to_string())
        } else {
            ("error".to_string(), "\u{2014}".to_string())
        };

        let marker = if is_primary { " *" } else { "" };
        let _ = writeln!(
            out,
            "| {}{marker} | {} | {status} | {symbols} |",
            repo.name,
            repo.path.display(),
        );
    }

    out.push_str("\n\\* = active (current session)\n");
    Ok(out)
}
