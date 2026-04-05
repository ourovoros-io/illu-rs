use crate::db::Database;
use crate::git::git_common_dir;
use crate::registry::Registry;
use std::fmt::Write;
use std::path::Path;

pub fn handle_cross_impact(
    registry: &Registry,
    primary_path: &Path,
    symbol_name: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let primary_common_dir = git_common_dir(primary_path).ok();
    let other_repos = registry.other_repos(primary_path, primary_common_dir.as_deref());
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

        let refs = db.find_cross_refs(name, impl_type)?;

        if !refs.is_empty() {
            found_any = true;
            let _ = write!(
                out,
                "### {} ({}) \u{2014} {} reference(s)\n\n",
                repo.name,
                repo.path.display(),
                refs.len()
            );
            for r in &refs {
                let qualified = match &r.impl_type {
                    Some(it) => format!("{it}::{}", r.name),
                    None => r.name.clone(),
                };
                let _ = writeln!(out, "- `{qualified}` ({}:{})", r.file_path, r.line_start);
            }
            out.push('\n');
        }
    }

    if !found_any {
        out.push_str("No references to this symbol found in other repos.\n");
    }
    Ok(out)
}
