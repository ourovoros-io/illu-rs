use crate::db::Database;
use crate::registry::Registry;
use std::fmt::Write;
use std::path::Path;

pub fn handle_cross_impact(
    registry: &Registry,
    primary_path: &Path,
    symbol_name: &str,
    filter: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let other_repos = registry.other_repos_for(primary_path);
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

        let mut refs = Vec::new();

        if let Some(f) = filter {
            // Textual search matching the namespace filter in body or signature
            let body_matches = db.search_symbols_by_body(f)?;
            let sig_matches = db.search_symbols_by_signature(f)?;

            // Combine and deduplicate
            let mut unique_matches = std::collections::HashMap::new();
            for s in body_matches.into_iter().chain(sig_matches) {
                if s.name == name && s.impl_type.as_deref() == impl_type {
                    unique_matches.insert(
                        (s.file_path.clone(), s.line_start),
                        crate::db::CrossRef {
                            name: s.name.clone(),
                            impl_type: s.impl_type.clone(),
                            file_path: s.file_path,
                            line_start: s.line_start,
                        },
                    );
                }
            }
            refs.extend(unique_matches.into_values());
        } else {
            // Structural graph query
            refs = db.find_cross_refs(name, impl_type)?;
        }

        // Sort by file path and line start for consistent output
        refs.sort_by(|a, b| {
            a.file_path
                .cmp(&b.file_path)
                .then_with(|| a.line_start.cmp(&b.line_start))
        });

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
