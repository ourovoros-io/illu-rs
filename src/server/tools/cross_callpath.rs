use crate::db::Database;
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
    let other_repos = registry.other_repos(primary_path);
    if other_repos.is_empty() {
        return Ok("No other repos registered.".into());
    }

    let mut out = format!("## Cross-Repo Callpath: `{from}` \u{2192} `{to}`\n\n");

    let from_exists: i64 = primary_db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE name = ?1",
            [from],
            |r| r.get(0),
        )
        .unwrap_or(0);

    if from_exists == 0 {
        return Ok(format!("`{from}` not found in the current repo."));
    }

    let repos_to_search: Vec<_> = if let Some(name) = target_repo {
        other_repos.into_iter().filter(|r| r.name == name).collect()
    } else {
        other_repos
    };

    let callees = get_callees_of(primary_db, from)?;
    let mut found = false;

    for repo in &repos_to_search {
        let db_path = repo.path.join(".illu/index.db");
        let Ok(db) = Database::open_readonly(&db_path) else {
            continue;
        };

        let to_exists: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM symbols WHERE name = ?1", [to], |r| {
                r.get(0)
            })
            .unwrap_or(0);

        if to_exists == 0 {
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

fn get_callees_of(db: &Database, from: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut stmt = db.conn.prepare(
        "SELECT DISTINCT t.name
         FROM symbol_refs sr
         JOIN symbols s ON sr.source_symbol_id = s.id
         JOIN symbols t ON sr.target_symbol_id = t.id
         WHERE s.name = ?1 AND sr.kind = 'call' LIMIT 100",
    )?;
    let callees: Vec<String> = stmt
        .query_map([from], |row| row.get(0))?
        .filter_map(std::result::Result::ok)
        .collect();
    Ok(callees)
}

fn find_bridges(db: &Database, callees: &[String]) -> Vec<String> {
    let mut bridges = Vec::new();
    for callee in callees {
        let exists: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM symbols WHERE name = ?1",
                [callee.as_str()],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if exists > 0 {
            bridges.push(callee.clone());
        }
    }
    bridges
}
