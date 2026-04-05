use crate::db::Database;
use crate::git::git_common_dir;
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

    // Identify the primary by `git_common_dir` so the active marker
    // stays correct across worktrees of the same repo. Fall back to
    // path comparison if git lookup fails (e.g. non-git invocation).
    let primary_common_dir = match git_common_dir(primary_path) {
        Ok(cd) => Some(cd),
        Err(e) => {
            tracing::debug!(
                primary_path = %primary_path.display(),
                error = %e,
                "git_common_dir failed; falling back to path-based primary detection"
            );
            None
        }
    };

    let mut out = String::from("## Registered Repos\n\n");
    out.push_str("| Repo | Path | Status | Symbols |\n");
    out.push_str("|------|------|--------|---------|\n");

    for repo in &registry.repos {
        let is_primary = primary_common_dir.as_ref().map_or_else(
            || repo.path == primary_path,
            |cd| &repo.git_common_dir == cd,
        );
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
