use crate::db::Database;
use crate::git::git_common_dir;
use crate::registry::Registry;
use std::fmt::Write;
use std::path::Path;

pub fn handle_cross_callpath(
    primary_db: &Database,
    registry: &Registry,
    primary_path: &Path,
    from: &str,
    to: &str,
    target_repo: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let primary_common_dir = git_common_dir(primary_path).ok();
    let other_repos = registry.other_repos(primary_path, primary_common_dir.as_deref());
    if other_repos.is_empty() {
        return Ok("No other repos registered.".into());
    }

    let mut out = format!("## Cross-Repo Callpath: `{from}` \u{2192} `{to}`\n\n");

    if !primary_db.symbol_exists(from)? {
        return Ok(format!("`{from}` not found in the current repo."));
    }

    let repos_to_search: Vec<_> = if let Some(name) = target_repo {
        other_repos.into_iter().filter(|r| r.name == name).collect()
    } else {
        other_repos
    };

    let callees = primary_db.get_direct_callees(from)?;
    let mut found = false;

    for repo in &repos_to_search {
        let db_path = repo.path.join(".illu/index.db");
        let Ok(db) = Database::open_readonly(&db_path) else {
            continue;
        };

        let Ok(to_exists) = db.symbol_exists(to) else {
            continue;
        };
        if !to_exists {
            continue;
        }

        found = true;
        let _ = write!(out, "### Via {} ({})\n\n", repo.name, repo.path.display());

        let bridges = find_bridges(&db, &callees);

        if bridges.is_empty() {
            let _ = write!(
                out,
                "- `{from}` (this repo) \u{2192} ? \u{2192} \
                 `{to}` ({}) \u{2014} no shared symbols found\n\n",
                repo.name
            );
        } else {
            for bridge in &bridges {
                let _ = writeln!(
                    out,
                    "- `{from}` (this repo) \u{2192} \
                     `{bridge}` \u{2192} `{to}` ({})",
                    repo.name
                );
            }
            out.push('\n');
        }
    }

    if !found {
        let _ = writeln!(out, "`{to}` not found in any other registered repo.");
    }
    Ok(out)
}

fn find_bridges(db: &Database, callees: &[String]) -> Vec<String> {
    let mut bridges = Vec::new();
    for callee in callees {
        let exists = db.symbol_exists(callee).unwrap_or(false);
        if exists {
            bridges.push(callee.clone());
        }
    }
    bridges
}
