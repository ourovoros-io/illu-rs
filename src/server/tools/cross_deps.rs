use crate::registry::Registry;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

pub fn handle_cross_deps(registry: &Registry) -> Result<String, Box<dyn std::error::Error>> {
    if registry.repos.len() < 2 {
        return Ok("Need at least 2 registered repos for cross-dep analysis.".into());
    }

    let mut out = String::from("## Cross-Repo Dependencies\n\n");
    let mut repo_deps: HashMap<String, HashSet<String>> = HashMap::new();
    let mut path_deps: Vec<(String, String, String)> = Vec::new();

    for repo in &registry.repos {
        let (deps, paths) = collect_repo_deps(repo);
        repo_deps.insert(repo.name.clone(), deps);
        path_deps.extend(paths);
    }

    render_path_deps(&mut out, registry, &path_deps);
    render_shared_deps(&mut out, &repo_deps);

    Ok(out)
}

fn collect_repo_deps(
    repo: &crate::registry::RepoEntry,
) -> (HashSet<String>, Vec<(String, String, String)>) {
    let mut deps = HashSet::new();
    let mut path_deps = Vec::new();

    let cargo_toml = repo.path.join("Cargo.toml");
    let Ok(content) = std::fs::read_to_string(&cargo_toml) else {
        return (deps, path_deps);
    };
    let Ok(parsed) = content.parse::<toml::Table>() else {
        return (deps, path_deps);
    };

    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        let Some(table) = parsed.get(section).and_then(|v| v.as_table()) else {
            continue;
        };
        for (name, value) in table {
            deps.insert(name.clone());
            let path_val = match value {
                toml::Value::Table(t) => t.get("path").and_then(|v| v.as_str()),
                _ => None,
            };
            if let Some(p) = path_val {
                let abs = repo.path.join(p);
                if let Ok(canonical) = abs.canonicalize() {
                    path_deps.push((
                        repo.name.clone(),
                        name.clone(),
                        canonical.to_string_lossy().into_owned(),
                    ));
                }
            }
        }
    }
    (deps, path_deps)
}

fn render_path_deps(out: &mut String, registry: &Registry, path_deps: &[(String, String, String)]) {
    let registered_paths: HashSet<String> = registry
        .repos
        .iter()
        .filter_map(|r| r.path.canonicalize().ok())
        .map(|p| p.to_string_lossy().into_owned())
        .collect();

    let cross: Vec<_> = path_deps
        .iter()
        .filter(|(_, _, to)| registered_paths.contains(to))
        .collect();

    if cross.is_empty() {
        return;
    }

    out.push_str("### Path Dependencies (direct source links)\n\n");
    for (from, name, to) in &cross {
        let to_name = registry
            .repos
            .iter()
            .find(|r| {
                r.path
                    .canonicalize()
                    .ok()
                    .is_some_and(|p| p.to_string_lossy() == *to)
            })
            .map_or("?", |r| r.name.as_str());
        let _ = writeln!(out, "- **{from}** \u{2192} `{name}` \u{2192} **{to_name}**");
    }
    out.push('\n');
}

fn render_shared_deps(out: &mut String, repo_deps: &HashMap<String, HashSet<String>>) {
    if repo_deps.len() < 2 {
        return;
    }

    let all_dep_names: HashSet<&String> = repo_deps.values().flatten().collect();
    let mut shared: Vec<(String, Vec<String>)> = Vec::new();
    for dep in &all_dep_names {
        let users: Vec<String> = repo_deps
            .iter()
            .filter(|(_, deps)| deps.contains(*dep))
            .map(|(name, _)| name.clone())
            .collect();
        if users.len() >= 2 {
            shared.push(((*dep).clone(), users));
        }
    }

    if shared.is_empty() {
        return;
    }

    shared.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
    out.push_str("### Shared Dependencies\n\n");
    out.push_str("| Crate | Used By |\n|-------|---------|\n");
    for (dep, users) in shared.iter().take(30) {
        let _ = writeln!(out, "| {} | {} |", dep, users.join(", "));
    }
    out.push('\n');
}
