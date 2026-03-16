pub mod dependencies;
pub mod docs;
pub mod parser;
pub mod store;
pub mod workspace;

use crate::db::Database;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

#[derive(Clone)]
pub struct IndexConfig {
    pub repo_path: PathBuf,
    pub skip_doc_fetch: bool,
}

pub fn index_repo(db: &Database, config: &IndexConfig) -> Result<(), Box<dyn std::error::Error>> {
    db.clear_index()?;

    let cargo_toml = std::fs::read_to_string(config.repo_path.join("Cargo.toml"))?;
    let ws_info = workspace::parse_workspace_toml(&cargo_toml)?;

    if ws_info.is_workspace {
        index_workspace(db, config, &ws_info)?;
    } else {
        index_single_crate(db, config, &cargo_toml)?;
    }

    extract_all_symbol_refs(db, config)?;
    generate_skill_file(db, config)?;
    update_metadata(db, config)?;

    Ok(())
}

/// Incrementally re-index only files whose content has changed.
/// If the DB is empty, does a full index first.
/// Returns the number of files that were re-indexed.
pub fn refresh_index(
    db: &Database,
    config: &IndexConfig,
) -> Result<usize, Box<dyn std::error::Error>> {
    let file_count: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))?;
    if file_count == 0 {
        tracing::info!("Empty index — running full index");
        index_repo(db, config)?;
        let new_count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))?;
        return Ok(usize::try_from(new_count).unwrap_or(0));
    }

    let existing: std::collections::HashMap<String, (String, Option<i64>)> = db
        .get_all_files_with_hashes()?
        .into_iter()
        .map(|(path, hash, crate_id)| (path, (hash, crate_id)))
        .collect();

    let mut dirty_files: Vec<(String, String, Option<i64>)> = Vec::new();

    // Walk all .rs files in the repo
    for entry in walkdir::WalkDir::new(&config.repo_path)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "rs") {
            continue;
        }
        // Skip hidden dirs and target/
        let relative = path
            .strip_prefix(&config.repo_path)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        if relative.starts_with("target/") || relative.starts_with('.') {
            continue;
        }

        let source = std::fs::read_to_string(path)?;
        let hash = content_hash(&source);

        let needs_update = match existing.get(&relative) {
            Some((old_hash, _)) => *old_hash != hash,
            None => true,
        };

        if needs_update {
            let crate_id = existing
                .get(&relative)
                .and_then(|(_, cid)| *cid);
            dirty_files.push((relative, source, crate_id));
        }
    }

    // Check for deleted files
    for path in existing.keys() {
        let full = config.repo_path.join(path);
        if !full.exists() {
            db.delete_file_data(path)?;
        }
    }

    if dirty_files.is_empty() {
        return Ok(0);
    }

    let count = dirty_files.len();
    tracing::info!("{count} file(s) changed, re-indexing");

    for (relative, source, crate_id) in &dirty_files {
        db.delete_file_data(relative)?;
        let hash = content_hash(source);
        let file_id = if let Some(cid) = crate_id {
            db.insert_file_with_crate(relative, &hash, *cid)?
        } else {
            db.insert_file(relative, &hash)?
        };
        let symbols = parser::parse_rust_source(source, relative)
            .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
        store::store_symbols(db, file_id, &symbols)?;
    }

    // Rebuild refs for dirty files (need full symbol set)
    let known_symbols = db.get_all_symbol_names()?;
    if !known_symbols.is_empty() {
        for (relative, source, _) in &dirty_files {
            let refs = parser::extract_refs(source, relative, &known_symbols)
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            for r in &refs {
                let source_id = db.get_symbol_id(&r.source_name, &r.source_file)?;
                let target_id = db.get_symbol_id_by_name(&r.target_name)?;
                if let (Some(sid), Some(tid)) = (source_id, target_id) {
                    db.insert_symbol_ref(sid, tid, &r.kind.to_string())?;
                }
            }
        }
    }

    Ok(count)
}

fn index_single_crate(
    db: &Database,
    config: &IndexConfig,
    cargo_toml: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let direct = dependencies::parse_cargo_toml(cargo_toml)?;
    let locked = parse_cargo_lock(&config.repo_path)?;
    let resolved = dependencies::resolve_dependencies(&direct, &locked);
    store::store_dependencies(db, &resolved)?;

    let pkg_name = extract_package_name(cargo_toml).unwrap_or_else(|| "root".to_string());
    let crate_id = db.insert_crate(&pkg_name, ".", false)?;

    index_crate_sources(db, config, &config.repo_path.join("src"), crate_id)?;
    Ok(())
}

fn index_workspace(
    db: &Database,
    config: &IndexConfig,
    ws_info: &workspace::WorkspaceInfo,
) -> Result<(), Box<dyn std::error::Error>> {
    let locked = parse_cargo_lock(&config.repo_path)?;

    // Collect all external deps across members
    let mut all_direct = Vec::new();

    // Register each member crate
    let mut crate_ids: std::collections::HashMap<String, i64> = std::collections::HashMap::new();

    for member in &ws_info.members {
        let member_dir = config.repo_path.join(member);
        let member_toml_path = member_dir.join("Cargo.toml");
        let Ok(member_toml) = std::fs::read_to_string(&member_toml_path) else {
            tracing::warn!("Skipping member {member}: no Cargo.toml");
            continue;
        };

        let pkg_name = extract_package_name(&member_toml).unwrap_or_else(|| member.clone());
        let crate_id = db.insert_crate(&pkg_name, member, false)?;
        crate_ids.insert(pkg_name.clone(), crate_id);

        // Resolve external deps for this member
        let member_deps = workspace::resolve_member_deps(&member_toml, &ws_info.workspace_deps)?;
        for dep in &member_deps {
            if !all_direct
                .iter()
                .any(|d: &dependencies::DirectDep| d.name == dep.name)
            {
                all_direct.push(dep.clone());
            }
        }

        // Index source files
        let src_dir = member_dir.join("src");
        index_crate_sources(db, config, &src_dir, crate_id)?;
    }

    // Store resolved external deps
    let resolved = dependencies::resolve_dependencies(&all_direct, &locked);
    store::store_dependencies(db, &resolved)?;

    // Record inter-crate dependencies
    for member in &ws_info.members {
        let member_dir = config.repo_path.join(member);
        let member_toml_path = member_dir.join("Cargo.toml");
        let Ok(member_toml) = std::fs::read_to_string(&member_toml_path) else {
            continue;
        };

        let pkg_name = extract_package_name(&member_toml).unwrap_or_else(|| member.clone());
        let Some(&source_id) = crate_ids.get(&pkg_name) else {
            continue;
        };

        let path_deps = workspace::extract_path_deps(&member_toml)?;
        for pd in &path_deps {
            if let Some(&target_id) = crate_ids.get(&pd.name) {
                db.insert_crate_dep(source_id, target_id)?;
            }
        }
    }

    Ok(())
}

fn index_crate_sources(
    db: &Database,
    config: &IndexConfig,
    src_dir: &std::path::Path,
    crate_id: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    if !src_dir.exists() {
        return Ok(());
    }
    for entry in walkdir::WalkDir::new(src_dir)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "rs") {
            let source = std::fs::read_to_string(path)?;
            let relative = path
                .strip_prefix(&config.repo_path)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();
            let hash = content_hash(&source);
            let file_id = db.insert_file_with_crate(&relative, &hash, crate_id)?;
            let symbols = parser::parse_rust_source(&source, &relative)
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            store::store_symbols(db, file_id, &symbols)?;
        }
    }
    Ok(())
}

fn extract_all_symbol_refs(
    db: &Database,
    config: &IndexConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let known_symbols = db.get_all_symbol_names()?;
    if known_symbols.is_empty() {
        return Ok(());
    }

    let files = db.get_all_file_paths()?;
    let mut ref_count: u64 = 0;

    for relative in &files {
        let full_path = config.repo_path.join(relative);
        let Ok(source) = std::fs::read_to_string(&full_path) else {
            continue;
        };
        let refs = parser::extract_refs(&source, relative, &known_symbols)
            .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
        for r in &refs {
            let source_id = db.get_symbol_id(&r.source_name, &r.source_file)?;
            let target_id = db.get_symbol_id_by_name(&r.target_name)?;
            if let (Some(sid), Some(tid)) = (source_id, target_id) {
                db.insert_symbol_ref(sid, tid, &r.kind.to_string())?;
                ref_count += 1;
            }
        }
    }

    tracing::info!("Stored {ref_count} symbol references");
    Ok(())
}

fn generate_skill_file(
    db: &Database,
    config: &IndexConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let direct_deps = db.get_direct_dependencies()?;
    let dep_names: Vec<String> = direct_deps.iter().map(|d| d.name.clone()).collect();
    let skill_content = generate_claude_skill(&dep_names);
    let skill_dir = config.repo_path.join(".claude").join("skills");
    std::fs::create_dir_all(&skill_dir)?;
    std::fs::write(skill_dir.join("illu-rs.md"), &skill_content)?;
    tracing::info!("Wrote Claude skill to .claude/skills/illu-rs.md");
    Ok(())
}

fn update_metadata(db: &Database, config: &IndexConfig) -> Result<(), Box<dyn std::error::Error>> {
    let commit_hash =
        get_current_commit_hash(&config.repo_path).unwrap_or_else(|_| "unknown".to_string());
    db.set_metadata(&config.repo_path.display().to_string(), &commit_hash)?;
    Ok(())
}

fn parse_cargo_lock(
    repo_path: &std::path::Path,
) -> Result<Vec<dependencies::LockedDep>, Box<dyn std::error::Error>> {
    match std::fs::read_to_string(repo_path.join("Cargo.lock")) {
        Ok(lock) => Ok(dependencies::parse_cargo_lock(&lock)?),
        Err(_) => Ok(vec![]),
    }
}

fn extract_package_name(cargo_toml: &str) -> Option<String> {
    let parsed: toml::Value = toml::from_str(cargo_toml).ok()?;
    parsed
        .get("package")?
        .get("name")?
        .as_str()
        .map(String::from)
}

/// Generate a Claude skill markdown file listing available
/// MCP tools and the project's direct dependencies.
#[must_use]
pub fn generate_claude_skill(direct_dep_names: &[String]) -> String {
    use std::fmt::Write;

    let mut out = String::new();
    let _ = writeln!(out, "# illu-rs Code Intelligence\n");
    let _ = writeln!(
        out,
        "This project is indexed by illu-rs. \
         Use the following MCP tools to explore the codebase \
         and its dependencies.\n"
    );
    let _ = writeln!(out, "## Tools\n");
    let _ = writeln!(
        out,
        "- **query** — Search symbols, docs, or files. \
         Pass `scope` (symbols/docs/files/all)."
    );
    let _ = writeln!(
        out,
        "- **context** — Get full context for a symbol: \
         definition, signature, file location, related docs."
    );
    let _ = writeln!(
        out,
        "- **impact** — Analyze the impact of changing a \
         symbol by finding all transitive dependents."
    );
    let _ = writeln!(
        out,
        "- **docs** — Get documentation for a dependency, \
         optionally filtered by topic.\n"
    );
    let _ = writeln!(out, "## Direct Dependencies\n");

    if direct_dep_names.is_empty() {
        let _ = writeln!(out, "No direct dependencies found.");
    } else {
        for dep in direct_dep_names {
            let _ = writeln!(out, "- {dep}");
        }
    }
    out
}

fn content_hash(content: &str) -> String {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

fn get_current_commit_hash(
    repo_path: &std::path::Path,
) -> Result<String, Box<dyn std::error::Error>> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_path)
        .output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err("git rev-parse HEAD failed".into())
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn test_index_pipeline_offline() {
        let dir = tempfile::TempDir::new().unwrap();
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();

        std::fs::write(
            dir.path().join("Cargo.toml"),
            r#"
[package]
name = "test"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();

        std::fs::write(
            dir.path().join("Cargo.lock"),
            r#"
[[package]]
name = "test"
version = "0.1.0"
"#,
        )
        .unwrap();

        std::fs::write(
            src_dir.join("main.rs"),
            r#"
pub fn hello() -> &'static str { "hello" }
"#,
        )
        .unwrap();

        let db = Database::open_in_memory().unwrap();
        let config = IndexConfig {
            repo_path: dir.path().to_path_buf(),
            skip_doc_fetch: true,
        };
        index_repo(&db, &config).unwrap();

        let symbols = db.search_symbols("hello").unwrap();
        assert_eq!(symbols.len(), 1);
    }

    #[test]
    fn test_generate_skill_content() {
        let deps = vec!["serde".to_string(), "tokio".to_string()];
        let skill = generate_claude_skill(&deps);
        assert!(skill.contains("serde"));
        assert!(skill.contains("tokio"));
        assert!(skill.contains("docs"));
        assert!(skill.contains("context"));
        assert!(skill.contains("query"));
        assert!(skill.contains("impact"));
    }

    #[test]
    fn test_generate_skill_no_deps() {
        let skill = generate_claude_skill(&[]);
        assert!(skill.contains("No direct dependencies"));
    }

    #[test]
    fn test_index_no_src_dir() {
        let dir = tempfile::TempDir::new().unwrap();

        std::fs::write(
            dir.path().join("Cargo.toml"),
            r#"
[package]
name = "test"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();

        let db = Database::open_in_memory().unwrap();
        let config = IndexConfig {
            repo_path: dir.path().to_path_buf(),
            skip_doc_fetch: true,
        };
        index_repo(&db, &config).unwrap();
        let symbols = db.search_symbols("anything").unwrap();
        assert_eq!(symbols.len(), 0);
    }

    #[test]
    fn test_index_workspace() {
        let dir = tempfile::TempDir::new().unwrap();

        // Workspace root Cargo.toml
        std::fs::write(
            dir.path().join("Cargo.toml"),
            r#"
[workspace]
members = ["shared", "app"]

[workspace.dependencies]
serde = "1.0"
"#,
        )
        .unwrap();

        std::fs::write(
            dir.path().join("Cargo.lock"),
            r#"
[[package]]
name = "serde"
version = "1.0.210"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "shared"
version = "0.1.0"

[[package]]
name = "app"
version = "0.1.0"
"#,
        )
        .unwrap();

        // shared crate
        let shared_dir = dir.path().join("shared");
        std::fs::create_dir_all(shared_dir.join("src")).unwrap();
        std::fs::write(
            shared_dir.join("Cargo.toml"),
            r#"
[package]
name = "shared"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();
        std::fs::write(
            shared_dir.join("src").join("lib.rs"),
            "pub struct SharedType { pub value: i32 }\n",
        )
        .unwrap();

        // app crate depending on shared
        let app_dir = dir.path().join("app");
        std::fs::create_dir_all(app_dir.join("src")).unwrap();
        std::fs::write(
            app_dir.join("Cargo.toml"),
            r#"
[package]
name = "app"
version = "0.1.0"
edition = "2021"

[dependencies]
shared = { path = "../shared" }
serde = { workspace = true }
"#,
        )
        .unwrap();
        std::fs::write(
            app_dir.join("src").join("main.rs"),
            r"
pub fn use_shared() -> SharedType {
    SharedType { value: 42 }
}
",
        )
        .unwrap();

        let db = Database::open_in_memory().unwrap();
        let config = IndexConfig {
            repo_path: dir.path().to_path_buf(),
            skip_doc_fetch: true,
        };
        index_repo(&db, &config).unwrap();

        // Both crates' symbols indexed
        let shared_syms = db.search_symbols("SharedType").unwrap();
        assert!(!shared_syms.is_empty(), "SharedType should be indexed");

        let app_syms = db.search_symbols("use_shared").unwrap();
        assert!(!app_syms.is_empty(), "use_shared should be indexed");

        // Inter-crate dependency tracked
        let shared_crate = db.get_crate_by_name("shared").unwrap().unwrap();
        let dependents = db.get_crate_dependents(shared_crate.id).unwrap();
        assert_eq!(dependents.len(), 1);
        assert_eq!(dependents[0].name, "app");

        // Workspace dep resolved
        let serde_dep = db.get_dependency_by_name("serde").unwrap();
        assert!(serde_dep.is_some());

        // Cross-crate symbol ref exists
        let refs_result = db.search_symbols("SharedType").unwrap();
        assert!(!refs_result.is_empty());
    }
}
