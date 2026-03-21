use crate::db::Database;
use crate::registry::Registry;
use crate::server::tools::query::handle_query;
use std::fmt::Write;
use std::path::Path;

pub struct CrossQueryOpts<'a> {
    pub query: &'a str,
    pub scope: Option<&'a str>,
    pub kind: Option<&'a str>,
    pub attribute: Option<&'a str>,
    pub signature: Option<&'a str>,
    pub path: Option<&'a str>,
    pub limit: Option<i64>,
}

pub fn handle_cross_query(
    registry: &Registry,
    primary_path: &Path,
    opts: &CrossQueryOpts<'_>,
) -> Result<String, Box<dyn std::error::Error>> {
    let other_repos = registry.other_repos(primary_path);
    if other_repos.is_empty() {
        return Ok("No other repos registered. \
             Start illu in other repos to enable cross-repo queries."
            .into());
    }

    let mut out = String::from("## Cross-Repo Query Results\n\n");
    let mut found_any = false;

    for repo in &other_repos {
        let db_path = repo.path.join(".illu/index.db");
        let Ok(db) = Database::open_readonly(&db_path) else {
            continue;
        };
        let Ok(result) = handle_query(
            &db,
            opts.query,
            opts.scope,
            opts.kind,
            opts.attribute,
            opts.signature,
            opts.path,
            opts.limit,
        ) else {
            continue;
        };

        if !result.contains("No results") && !result.is_empty() {
            found_any = true;
            let _ = write!(out, "### {} ({})\n\n", repo.name, repo.path.display());
            out.push_str(&result);
            out.push_str("\n\n");
        }
    }

    if !found_any {
        out.push_str("No matches found in other repos.\n");
    }
    Ok(out)
}
