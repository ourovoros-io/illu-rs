pub mod cargo_doc;
pub mod dependencies;
pub mod docs;
pub mod parser;
pub mod store;
pub mod workspace;

use crate::db::Database;
use std::path::PathBuf;

#[derive(Clone)]
pub struct IndexConfig {
    pub repo_path: PathBuf,
}

pub fn index_repo(db: &Database, config: &IndexConfig) -> Result<(), Box<dyn std::error::Error>> {
    db.clear_index()?;

    let cargo_toml = std::fs::read_to_string(config.repo_path.join("Cargo.toml"))?;
    let ws_info = workspace::parse_workspace_toml(&cargo_toml)?;

    if ws_info.is_workspace {
        tracing::info!(
            members = ws_info.members.len(),
            "Phase 1/4: indexing workspace"
        );
        crate::status::set("indexing ▸ parsing workspace");
        index_workspace(db, config, &ws_info)?;
    } else {
        tracing::info!("Phase 1/4: indexing single crate");
        crate::status::set("indexing ▸ parsing crate");
        index_single_crate(db, config, &cargo_toml)?;
    }

    tracing::info!("Phase 2/4: extracting symbol references");
    crate::status::set("indexing ▸ extracting refs");
    extract_all_symbol_refs(db, config)?;
    tracing::info!("Phase 3/4: generating skill file");
    crate::status::set("indexing ▸ writing skill file");
    generate_skill_file(db, config)?;
    tracing::info!("Phase 4/4: updating metadata");
    update_metadata(db, config)?;

    let file_count = db.file_count()?;
    tracing::info!(files = file_count, "Indexing complete");
    crate::status::set(crate::status::READY);

    Ok(())
}

/// Incrementally re-index only files whose content has changed.
/// If the DB is empty, does a full index first.
/// Returns the number of files that were re-indexed.
pub fn refresh_index(
    db: &Database,
    config: &IndexConfig,
) -> Result<usize, Box<dyn std::error::Error>> {
    struct DirtyFile {
        relative_path: String,
        source: String,
        hash: String,
        crate_id: Option<crate::db::CrateId>,
    }

    let file_count = db.file_count()?;
    if file_count == 0 {
        tracing::info!("Empty index — running full index");
        index_repo(db, config)?;
        let new_count = db.file_count()?;
        return Ok(usize::try_from(new_count).unwrap_or(0));
    }

    let existing: std::collections::HashMap<String, (String, Option<crate::db::CrateId>)> = db
        .get_all_files_with_hashes()?
        .into_iter()
        .map(|f| (f.path, (f.content_hash, f.crate_id)))
        .collect();

    let mut dirty_files: Vec<DirtyFile> = Vec::new();

    // Walk all .rs files in the repo
    for result in walkdir::WalkDir::new(&config.repo_path) {
        let entry = match result {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Skipping directory entry: {e}");
                continue;
            }
        };
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
            let crate_id = existing.get(&relative).and_then(|(_, cid)| *cid);
            dirty_files.push(DirtyFile {
                relative_path: relative,
                source,
                hash,
                crate_id,
            });
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
    tracing::info!(files = count, "Re-indexing changed files");
    crate::status::set(&format!("refreshing ▸ {count} files"));

    for (i, df) in dirty_files.iter().enumerate() {
        crate::status::set(&format!("refreshing ▸ [{}/{}]", i + 1, count));
        tracing::debug!("[{}/{}] Re-indexing {}", i + 1, count, df.relative_path);
        db.delete_file_data(&df.relative_path)?;
        let file_id = if let Some(cid) = df.crate_id {
            db.insert_file_with_crate(&df.relative_path, &df.hash, cid)?
        } else {
            db.insert_file(&df.relative_path, &df.hash)?
        };
        let (symbols, trait_impls) = parser::parse_rust_source(&df.source, &df.relative_path)
            .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
        store::store_symbols(db, file_id, &symbols)?;
        store::store_trait_impls(db, file_id, &trait_impls)?;
    }

    // Rebuild refs for dirty files (need full symbol set)
    let known_symbols = db.get_all_symbol_names()?;
    if !known_symbols.is_empty() {
        db.begin_transaction()?;
        for df in &dirty_files {
            let refs = parser::extract_refs(&df.source, &df.relative_path, &known_symbols)
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            db.store_symbol_refs(&refs)?;
        }
        db.commit()?;
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
    let crate_id = db.insert_crate(&pkg_name, ".")?;

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
    let mut crate_ids: std::collections::HashMap<String, crate::db::CrateId> =
        std::collections::HashMap::new();

    // Collect path deps per crate to record after all crates are registered
    let mut path_deps_by_crate: Vec<(String, Vec<String>)> = Vec::new();

    let total_members = ws_info.members.len();
    for (i, member) in ws_info.members.iter().enumerate() {
        let member_dir = config.repo_path.join(member);
        let member_toml_path = member_dir.join("Cargo.toml");
        let Ok(member_toml) = std::fs::read_to_string(&member_toml_path) else {
            tracing::warn!("Skipping member {member}: no Cargo.toml");
            continue;
        };
        tracing::info!("[{}/{}] Indexing crate: {member}", i + 1, total_members);
        crate::status::set(&format!(
            "indexing ▸ crate [{}/{}] {member}",
            i + 1,
            total_members
        ));

        let pkg_name = extract_package_name(&member_toml).unwrap_or_else(|| member.clone());
        let crate_id = db.insert_crate(&pkg_name, member)?;
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

        // Collect inter-crate path deps (recorded after all crates exist)
        let pds = workspace::extract_path_deps(&member_toml)?;
        let dep_names: Vec<String> = pds.into_iter().map(|pd| pd.name).collect();
        if !dep_names.is_empty() {
            path_deps_by_crate.push((pkg_name, dep_names));
        }

        // Index source files
        let src_dir = member_dir.join("src");
        index_crate_sources(db, config, &src_dir, crate_id)?;
    }

    // Store resolved external deps
    let resolved = dependencies::resolve_dependencies(&all_direct, &locked);
    store::store_dependencies(db, &resolved)?;

    // Record inter-crate dependencies (all crates registered now)
    for (pkg_name, dep_names) in &path_deps_by_crate {
        let Some(&source_id) = crate_ids.get(pkg_name.as_str()) else {
            continue;
        };
        for dep_name in dep_names {
            if let Some(&target_id) = crate_ids.get(dep_name.as_str()) {
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
    crate_id: crate::db::CrateId,
) -> Result<(), Box<dyn std::error::Error>> {
    // Collect files first so we can report progress
    let rs_files: Vec<_> = walkdir::WalkDir::new(src_dir)
        .into_iter()
        .filter_map(|r| match r {
            Ok(e) if e.path().extension().is_some_and(|ext| ext == "rs") => Some(e.into_path()),
            Err(e) => {
                tracing::warn!("Skipping directory entry: {e}");
                None
            }
            _ => None,
        })
        .collect();

    let total = rs_files.len();
    for (i, path) in rs_files.iter().enumerate() {
        crate::status::set(&format!("indexing ▸ parsing [{}/{}]", i + 1, total));
        let source = std::fs::read_to_string(path)?;
        let relative = path
            .strip_prefix(&config.repo_path)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        tracing::debug!("[{}/{}] Parsing {relative}", i + 1, total);
        let hash = content_hash(&source);
        let file_id = db.insert_file_with_crate(&relative, &hash, crate_id)?;
        let (symbols, trait_impls) = parser::parse_rust_source(&source, &relative)
            .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
        store::store_symbols(db, file_id, &symbols)?;
        store::store_trait_impls(db, file_id, &trait_impls)?;
    }
    tracing::info!(files = total, "Parsed source files");
    Ok(())
}

fn extract_all_symbol_refs(
    db: &Database,
    config: &IndexConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let known_symbols = db.get_all_symbol_names()?;
    if known_symbols.is_empty() {
        tracing::info!("No symbols found, skipping ref extraction");
        return Ok(());
    }

    let files = db.get_all_file_paths()?;
    let total = files.len();
    tracing::info!(
        files = total,
        symbols = known_symbols.len(),
        "Extracting symbol references"
    );
    let mut ref_count: u64 = 0;

    db.begin_transaction()?;
    for (i, relative) in files.iter().enumerate() {
        crate::status::set(&format!("indexing ▸ refs [{}/{}]", i + 1, total));
        if total > 20 && (i + 1) % 20 == 0 {
            tracing::info!("[{}/{}] Extracting refs...", i + 1, total);
        }
        let full_path = config.repo_path.join(relative);
        let source = match std::fs::read_to_string(&full_path) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Cannot read {}: {e}", full_path.display());
                continue;
            }
        };
        let refs = parser::extract_refs(&source, relative, &known_symbols)
            .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
        ref_count += db.store_symbol_refs(&refs)?;
    }
    db.commit()?;

    tracing::info!(refs = ref_count, "Symbol reference extraction complete");
    Ok(())
}

fn generate_skill_file(
    db: &Database,
    config: &IndexConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let direct_deps = db.get_direct_dependencies()?;
    let dep_names: Vec<&str> = direct_deps.iter().map(|d| d.name.as_str()).collect();
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
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e.into()),
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
pub(crate) fn generate_claude_skill(direct_dep_names: &[&str]) -> String {
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
         doc comments, definition, source body, struct fields, \
         trait implementations, and callees."
    );
    let _ = writeln!(
        out,
        "- **impact** — Analyze the impact of changing a \
         symbol by finding all transitive dependents."
    );
    let _ = writeln!(
        out,
        "- **docs** — Get documentation for a dependency, \
         optionally filtered by topic."
    );
    let _ = writeln!(
        out,
        "- **overview** — Get a structural overview of all \
         public symbols under a file path prefix.\n"
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

/// Stable FNV-1a hash — deterministic across Rust versions,
/// unlike `DefaultHasher` whose algorithm may change.
fn content_hash(content: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in content.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:x}")
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
        };
        index_repo(&db, &config).unwrap();

        let symbols = db.search_symbols("hello").unwrap();
        assert_eq!(symbols.len(), 1);
    }

    #[test]
    fn test_generate_skill_content() {
        let skill = generate_claude_skill(&["serde", "tokio"]);
        assert!(skill.contains("serde"));
        assert!(skill.contains("tokio"));
        assert!(skill.contains("docs"));
        assert!(skill.contains("context"));
        assert!(skill.contains("query"));
        assert!(skill.contains("impact"));
        assert!(skill.contains("overview"));
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

    #[test]
    fn test_refresh_index_no_changes() {
        let dir = tempfile::TempDir::new().unwrap();
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(src_dir.join("lib.rs"), "pub fn hello() {}\n").unwrap();

        let db = Database::open_in_memory().unwrap();
        let config = IndexConfig {
            repo_path: dir.path().to_path_buf(),
        };
        index_repo(&db, &config).unwrap();

        // No changes — refresh should return 0
        let refreshed = refresh_index(&db, &config).unwrap();
        assert_eq!(refreshed, 0);
    }

    #[test]
    fn test_refresh_index_detects_changed_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(src_dir.join("lib.rs"), "pub fn hello() {}\n").unwrap();

        let db = Database::open_in_memory().unwrap();
        let config = IndexConfig {
            repo_path: dir.path().to_path_buf(),
        };
        index_repo(&db, &config).unwrap();

        // Modify the file
        std::fs::write(
            src_dir.join("lib.rs"),
            "pub fn hello() {}\npub fn world() {}\n",
        )
        .unwrap();

        let refreshed = refresh_index(&db, &config).unwrap();
        assert_eq!(refreshed, 1);

        // New symbol should be indexed
        let syms = db.search_symbols("world").unwrap();
        assert_eq!(syms.len(), 1);
    }

    #[test]
    fn test_refresh_index_detects_deleted_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(src_dir.join("lib.rs"), "pub fn keep() {}\n").unwrap();
        std::fs::write(src_dir.join("extra.rs"), "pub fn gone() {}\n").unwrap();

        let db = Database::open_in_memory().unwrap();
        let config = IndexConfig {
            repo_path: dir.path().to_path_buf(),
        };
        index_repo(&db, &config).unwrap();

        // Verify extra.rs was indexed
        let syms = db.search_symbols("gone").unwrap();
        assert_eq!(syms.len(), 1);

        // Delete extra.rs
        std::fs::remove_file(src_dir.join("extra.rs")).unwrap();

        let _ = refresh_index(&db, &config).unwrap();

        // Deleted symbol should be gone
        let syms = db.search_symbols("gone").unwrap();
        assert!(syms.is_empty());

        // Kept symbol still present
        let syms = db.search_symbols("keep").unwrap();
        assert_eq!(syms.len(), 1);
    }

    #[test]
    fn test_refresh_index_on_empty_db() {
        let dir = tempfile::TempDir::new().unwrap();
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(src_dir.join("lib.rs"), "pub fn fresh() {}\n").unwrap();

        let db = Database::open_in_memory().unwrap();
        let config = IndexConfig {
            repo_path: dir.path().to_path_buf(),
        };

        // refresh on empty DB should do a full index
        let count = refresh_index(&db, &config).unwrap();
        assert!(count > 0);

        let syms = db.search_symbols("fresh").unwrap();
        assert_eq!(syms.len(), 1);
    }
}
