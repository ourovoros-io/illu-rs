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
    db.clear_code_index()?;

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
struct DirtyFile {
    relative_path: String,
    source: String,
    hash: String,
    crate_id: Option<crate::db::CrateId>,
}

pub fn refresh_index(
    db: &Database,
    config: &IndexConfig,
) -> Result<usize, Box<dyn std::error::Error>> {
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

    crate::status::set("refreshing ▸ scanning files");

    // Try git-based detection first, fall back to full walk
    let candidate_files = git_changed_rs_files(&config.repo_path, &existing);
    let dirty_files = collect_dirty_files(&config.repo_path, &candidate_files, &existing);

    // Check for deleted files (only in full-walk mode, git handles this via status)
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

    rebuild_refs_for_files(db, &dirty_files)?;

    let stale = db.delete_stale_refs()?;
    if stale > 0 {
        tracing::info!(deleted = stale, "Cleaned up stale symbol refs");
    }

    Ok(count)
}

fn rebuild_refs_for_files(
    db: &Database,
    dirty_files: &[DirtyFile],
) -> Result<(), Box<dyn std::error::Error>> {
    let known_symbols = db.get_all_symbol_names()?;
    if known_symbols.is_empty() {
        return Ok(());
    }

    let all_crates = db.get_all_crates()?;
    let crate_map: std::collections::HashMap<String, String> = all_crates
        .iter()
        .map(|c| (c.name.replace('-', "_"), c.path.clone()))
        .collect();

    let symbol_map = db.build_symbol_id_map()?;

    db.begin_transaction()?;
    for df in dirty_files {
        let refs = parser::extract_refs(&df.source, &df.relative_path, &known_symbols, &crate_map)
            .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
        db.store_symbol_refs_fast(&refs, &symbol_map)?;

        // Re-extract and store trait impls for this file
        let file_id: Option<crate::db::FileId> = db
            .conn
            .query_row(
                "SELECT id FROM files WHERE path = ?1",
                rusqlite::params![df.relative_path],
                |row| row.get(0),
            )
            .ok();
        if let Some(fid) = file_id {
            db.conn.execute(
                "DELETE FROM trait_impls WHERE file_id = ?1",
                rusqlite::params![fid],
            )?;
            let (_symbols, trait_impls) = parser::parse_rust_source(&df.source, &df.relative_path)
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            store::store_trait_impls(db, fid, &trait_impls)?;
        }
    }
    db.commit()?;
    Ok(())
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

    // Index integration tests, benchmarks, and examples
    for extra in &["tests", "benches", "examples"] {
        let dir = config.repo_path.join(extra);
        if dir.is_dir() {
            index_crate_sources(db, config, &dir, crate_id)?;
        }
    }

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

        let parsed: toml::Value = toml::from_str(&member_toml)?;

        let pkg_name = parsed
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(toml::Value::as_str)
            .map_or_else(|| member.clone(), String::from);
        let crate_id = db.insert_crate(&pkg_name, member)?;
        crate_ids.insert(pkg_name.clone(), crate_id);

        // Resolve external deps for this member
        let member_deps = workspace::resolve_member_deps(&parsed, &ws_info.workspace_deps);
        for dep in &member_deps {
            if !all_direct
                .iter()
                .any(|d: &dependencies::DirectDep| d.name == dep.name)
            {
                all_direct.push(dep.clone());
            }
        }

        // Collect inter-crate path deps (recorded after all crates exist)
        let pds = workspace::extract_path_deps(&parsed);
        let mut dep_names = Vec::new();
        for pd in pds {
            let target_toml_path = member_dir.join(&pd.path).join("Cargo.toml");
            let resolved_name = std::fs::read_to_string(&target_toml_path)
                .ok()
                .and_then(|content| extract_package_name(&content))
                .unwrap_or(pd.name);
            dep_names.push(resolved_name);
        }
        if !dep_names.is_empty() {
            path_deps_by_crate.push((pkg_name, dep_names));
        }

        // Index source files
        let src_dir = member_dir.join("src");
        index_crate_sources(db, config, &src_dir, crate_id)?;

        // Index integration tests, benchmarks, and examples
        for extra in &["tests", "benches", "examples"] {
            let dir = member_dir.join(extra);
            if dir.is_dir() {
                index_crate_sources(db, config, &dir, crate_id)?;
            }
        }
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
        .filter_entry(|e| {
            if !e.file_type().is_dir() || e.depth() == 0 {
                return true;
            }
            let name = e.file_name().to_string_lossy();
            name != "target" && !name.starts_with('.')
        })
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

    let all_crates = db.get_all_crates()?;
    let crate_map: std::collections::HashMap<String, String> = all_crates
        .iter()
        .map(|c| (c.name.replace('-', "_"), c.path.clone()))
        .collect();

    let symbol_map = db.build_symbol_id_map()?;

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
        let refs = parser::extract_refs(&source, relative, &known_symbols, &crate_map)
            .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
        ref_count += db.store_symbol_refs_fast(&refs, &symbol_map)?;
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

const TOOL_SECTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "Search & Navigate",
        &[
            (
                "query",
                "Search symbols, docs, or files. Filters: kind, attribute, signature, path.",
            ),
            (
                "context",
                "Full symbol context: source, callers, callees, trait impls. Supports `Type::method`, `sections` filter, `exclude_tests`.",
            ),
            ("batch_context", "Context for multiple symbols in one call."),
            ("symbols_at", "Find symbols at a file:line location."),
            ("overview", "Public symbols under a path, grouped by file."),
            ("tree", "File/module hierarchy."),
        ],
    ),
    (
        "Impact Analysis",
        &[
            (
                "impact",
                "Transitive dependents of a symbol (configurable depth).",
            ),
            ("diff_impact", "Batch impact for all symbols in a git diff."),
            ("test_impact", "Which tests break when changing a symbol."),
            ("crate_impact", "Which workspace crates are affected."),
        ],
    ),
    (
        "Call Graph",
        &[
            ("callpath", "Shortest or all paths between two symbols."),
            (
                "neighborhood",
                "Callers/callees within N hops (list or tree format).",
            ),
            (
                "references",
                "Unified view: call sites, type usage, trait impls.",
            ),
            (
                "type_usage",
                "Where a type appears in signatures and struct fields.",
            ),
            ("file_graph", "File-level dependency graph."),
            (
                "graph_export",
                "DOT/Graphviz export of call or file graphs.",
            ),
        ],
    ),
    (
        "Discovery & Audit",
        &[
            ("unused", "Symbols with no incoming references."),
            ("orphaned", "Symbols with no callers AND no test coverage."),
            (
                "boundary",
                "Public API vs internal-only classification for a module.",
            ),
            (
                "similar",
                "Functions with matching signatures and call patterns.",
            ),
            (
                "rename_plan",
                "All locations to update before renaming a symbol.",
            ),
            (
                "doc_coverage",
                "Undocumented symbols with coverage percentage.",
            ),
            (
                "hotspots",
                "Most-referenced, most-complex, and largest functions.",
            ),
            (
                "stats",
                "File/symbol counts, test coverage, top references.",
            ),
        ],
    ),
    (
        "Dependencies & Git",
        &[
            (
                "docs",
                "Version-pinned dependency documentation, filterable by topic.",
            ),
            ("implements", "Trait/type implementation relationships."),
            ("crate_graph", "Workspace inter-crate dependency graph."),
            ("blame", "Git blame on a symbol's line range."),
            (
                "history",
                "Git commit history for a symbol, with optional diffs.",
            ),
            ("freshness", "Index staleness check."),
            ("health", "Index quality diagnosis."),
        ],
    ),
];

fn write_tool_listing(out: &mut String) {
    use std::fmt::Write;

    let total: usize = TOOL_SECTIONS.iter().map(|(_, tools)| tools.len()).sum();
    let _ = writeln!(out, "## Tools ({total} available)\n");
    for (section, tools) in TOOL_SECTIONS {
        let _ = writeln!(out, "### {section}\n");
        for (name, desc) in *tools {
            let _ = writeln!(out, "- **{name}** — {desc}");
        }
        let _ = writeln!(out);
    }
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
    write_tool_listing(&mut out);
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

/// Use `git status` to find changed/new/deleted `.rs` files.
/// Returns a list of relative paths to check. If git fails, returns all
/// indexed files plus walks for new ones (full scan fallback).
fn git_changed_rs_files(
    repo_path: &std::path::Path,
    existing: &std::collections::HashMap<String, (String, Option<crate::db::CrateId>)>,
) -> Vec<String> {
    let output = std::process::Command::new("git")
        .args([
            "status",
            "--porcelain=v1",
            "--untracked-files=normal",
            "--no-renames",
        ])
        .current_dir(repo_path)
        .output();

    let Ok(output) = output else {
        tracing::debug!("git status failed, falling back to full scan");
        return full_scan_rs_files(repo_path);
    };
    if !output.status.success() {
        tracing::debug!("git status returned non-zero, falling back to full scan");
        return full_scan_rs_files(repo_path);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut changed: Vec<String> = Vec::new();

    for line in stdout.lines() {
        if line.len() < 4 {
            continue;
        }
        let path = line[3..].trim();
        if std::path::Path::new(path)
            .extension()
            .is_some_and(|ext| ext == "rs")
        {
            changed.push(path.to_string());
        }
    }

    // Also check for new .rs files not yet tracked by git but present on disk
    // and not yet in our index (e.g., files in .gitignore that we still want)
    // For now, the git status output covers new untracked files ("?? path").

    // Also include files that are in our index but might have been modified
    // outside of git tracking (rare but possible)
    for path in existing.keys() {
        if !changed.contains(path) {
            let full = repo_path.join(path);
            if !full.exists() {
                changed.push(path.clone());
            }
        }
    }

    tracing::debug!(count = changed.len(), "git detected changed .rs files");
    changed
}

/// Fallback: walk the repo for all `.rs` files.
fn full_scan_rs_files(repo_path: &std::path::Path) -> Vec<String> {
    let mut files = Vec::new();
    let walker = walkdir::WalkDir::new(repo_path)
        .into_iter()
        .filter_entry(|e| {
            if !e.file_type().is_dir() || e.depth() == 0 {
                return true;
            }
            let name = e.file_name().to_string_lossy();
            name != "target" && !name.starts_with('.')
        });
    for result in walker {
        let Ok(entry) = result else { continue };
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "rs") {
            let relative = path
                .strip_prefix(repo_path)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();
            files.push(relative);
        }
    }
    files
}

/// Read candidate files and determine which ones actually changed
/// by comparing content hashes.
fn collect_dirty_files(
    repo_path: &std::path::Path,
    candidates: &[String],
    existing: &std::collections::HashMap<String, (String, Option<crate::db::CrateId>)>,
) -> Vec<DirtyFile> {
    let mut dirty = Vec::new();
    for relative in candidates {
        let full_path = repo_path.join(relative);
        let Ok(source) = std::fs::read_to_string(&full_path) else {
            continue; // File deleted or unreadable
        };
        let hash = content_hash(&source);

        let needs_update = match existing.get(relative.as_str()) {
            Some((old_hash, _)) => *old_hash != hash,
            None => true,
        };

        if needs_update {
            let crate_id = existing.get(relative.as_str()).and_then(|(_, cid)| *cid);
            dirty.push(DirtyFile {
                relative_path: relative.clone(),
                source,
                hash,
                crate_id,
            });
        }
    }
    dirty
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
        assert!(skill.contains("31 available"));
        assert!(skill.contains("query"));
        assert!(skill.contains("context"));
        assert!(skill.contains("impact"));
        assert!(skill.contains("diff_impact"));
        assert!(skill.contains("test_impact"));
        assert!(skill.contains("neighborhood"));
        assert!(skill.contains("boundary"));
        assert!(skill.contains("orphaned"));
        assert!(skill.contains("blame"));
        assert!(skill.contains("overview"));
        assert!(skill.contains("docs"));
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
    fn test_crate_map_normalizes_hyphens() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();

        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(
            repo.join("Cargo.toml"),
            "[package]\nname = \"my-crate\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(repo.join("src/lib.rs"), "pub mod status;\n").unwrap();
        std::fs::write(repo.join("src/status.rs"), "pub fn reset_state() {}\n").unwrap();
        std::fs::write(
            repo.join("src/main.rs"),
            "use my_crate::status::reset_state;\nfn main() { reset_state(); }\n",
        )
        .unwrap();

        let db = Database::open_in_memory().unwrap();
        let config = IndexConfig {
            repo_path: repo.to_path_buf(),
        };
        index_repo(&db, &config).unwrap();

        // The ref from main→reset_state should exist with high confidence
        let callees = db.get_callees("main", "src/main.rs", false).unwrap();
        let has_ref = callees.iter().any(|c| c.name == "reset_state");
        assert!(
            has_ref,
            "main should have 'reset_state' as a callee (via my_crate:: import), got: {:?}",
            callees.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
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

    #[test]
    fn test_qualified_noisy_bypass_and_seen_dedup() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();

        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(
            repo.join("Cargo.toml"),
            "[package]\nname = \"my-app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();

        // Two structs each with a `new` (noisy) and `clear` (noisy)
        std::fs::write(
            repo.join("src/lib.rs"),
            r"pub struct Foo;
impl Foo {
    pub fn new() -> Self { Self }
    pub fn clear(&self) {}
}
pub struct Bar;
impl Bar {
    pub fn new() -> Self { Self }
    pub fn clear(&self) {}
}
pub fn run() {
    let f = Foo::new();
    f.clear();
    let b = Bar::new();
    b.clear();
}
",
        )
        .unwrap();

        std::fs::write(repo.join("src/main.rs"), "fn main() {}\n").unwrap();

        let db = Database::open_in_memory().unwrap();
        let config = IndexConfig {
            repo_path: repo.to_path_buf(),
        };
        index_repo(&db, &config).unwrap();

        let callees = db.get_callees("run", "src/lib.rs", false).unwrap();
        let callee_qualified: Vec<String> = callees
            .iter()
            .map(|c| match &c.impl_type {
                Some(it) => format!("{it}::{}", c.name),
                None => c.name.clone(),
            })
            .collect();

        // Bug 3 fix: `new` and `clear` are noisy names, but
        // Foo::new() and Bar::new() are qualified calls that should
        // bypass the noisy filter.
        assert!(
            callee_qualified.contains(&"Foo::new".to_string()),
            "run callees should include Foo::new, got: {callee_qualified:?}"
        );
        assert!(
            callee_qualified.contains(&"Bar::new".to_string()),
            "run callees should include Bar::new, got: {callee_qualified:?}"
        );

        // Bug 3 fix (also noisy): clear is noisy but qualified
        assert!(
            callee_qualified.contains(&"Foo::clear".to_string()),
            "run callees should include Foo::clear, \
             got: {callee_qualified:?}"
        );
        assert!(
            callee_qualified.contains(&"Bar::clear".to_string()),
            "run callees should include Bar::clear, \
             got: {callee_qualified:?}"
        );

        // Bug 4 fix: seen dedup includes target_context, so both
        // Foo::new AND Bar::new are captured (not just the first one)
        let new_count = callee_qualified
            .iter()
            .filter(|q| q.ends_with("::new"))
            .count();
        assert_eq!(
            new_count, 2,
            "Both Foo::new and Bar::new should be captured, \
             got: {callee_qualified:?}"
        );

        // Verify Foo::new and Bar::new are NOT reported as unused
        let unused = db.get_unreferenced_symbols(None, true).unwrap();
        let unused_names: Vec<String> = unused
            .iter()
            .map(|s| match &s.impl_type {
                Some(it) => format!("{it}::{}", s.name),
                None => s.name.clone(),
            })
            .collect();
        assert!(
            !unused_names.contains(&"Foo::new".to_string()),
            "Foo::new should NOT be unused, unused: {unused_names:?}"
        );
        assert!(
            !unused_names.contains(&"Bar::new".to_string()),
            "Bar::new should NOT be unused, unused: {unused_names:?}"
        );
    }

    #[test]
    fn test_index_tests_directory() {
        let dir = tempfile::TempDir::new().unwrap();
        let src_dir = dir.path().join("src");
        let tests_dir = dir.path().join("tests");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::create_dir_all(&tests_dir).unwrap();

        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(src_dir.join("lib.rs"), "pub fn compute() -> i32 { 42 }\n").unwrap();
        std::fs::write(
            tests_dir.join("integration.rs"),
            "#[test]\nfn test_compute() { assert_eq!(42, 42); }\n",
        )
        .unwrap();

        let db = Database::open_in_memory().unwrap();
        let config = IndexConfig {
            repo_path: dir.path().to_path_buf(),
        };
        index_repo(&db, &config).unwrap();

        // Integration test should be indexed
        let syms = db.search_symbols("test_compute").unwrap();
        assert_eq!(syms.len(), 1, "test_compute from tests/ should be indexed");
        assert!(
            syms[0].file_path.starts_with("tests/"),
            "should be in tests/ dir: {}",
            syms[0].file_path
        );

        // Source symbols still indexed
        let src_syms = db.search_symbols("compute").unwrap();
        assert!(!src_syms.is_empty(), "src/ symbols should still be indexed");
    }

    #[test]
    fn test_variable_method_resolution() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(
            repo.join("Cargo.toml"),
            "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(
            repo.join("src/lib.rs"),
            r"
pub struct Db;
impl Db {
    pub fn open() -> Self { Self }
    pub fn query(&self) -> i32 { 42 }
}
pub fn run() -> i32 {
    let db = Db::open();
    db.query()
}
",
        )
        .unwrap();
        std::fs::write(repo.join("src/main.rs"), "fn main() {}\n").unwrap();

        let db = Database::open_in_memory().unwrap();
        let config = IndexConfig {
            repo_path: repo.to_path_buf(),
        };
        index_repo(&db, &config).unwrap();

        let callees = db.get_callees("run", "src/lib.rs", false).unwrap();
        let callee_names: Vec<&str> = callees.iter().map(|c| c.name.as_str()).collect();

        assert!(
            callee_names.contains(&"open"),
            "run should call Db::open, got: {callee_names:?}"
        );
        assert!(
            callee_names.contains(&"query"),
            "run should call db.query() via local type inference, got: {callee_names:?}"
        );
    }

    #[test]
    fn test_variable_method_cross_file() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(
            repo.join("Cargo.toml"),
            "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(repo.join("src/lib.rs"), "pub mod db;\n").unwrap();
        std::fs::write(
            repo.join("src/db.rs"),
            r"
pub struct Conn;
impl Conn {
    pub fn open() -> Self { Self }
    pub fn execute(&self) -> i32 { 42 }
}
",
        )
        .unwrap();
        std::fs::write(
            repo.join("src/main.rs"),
            r"
use t::db::Conn;
fn run() -> i32 {
    let c = Conn::open();
    c.execute()
}
fn main() { run(); }
",
        )
        .unwrap();

        let db = Database::open_in_memory().unwrap();
        let config = IndexConfig {
            repo_path: repo.to_path_buf(),
        };
        index_repo(&db, &config).unwrap();

        let callees = db.get_callees("run", "src/main.rs", false).unwrap();
        let callee_names: Vec<&str> = callees.iter().map(|c| c.name.as_str()).collect();

        assert!(
            callee_names.contains(&"open"),
            "cross-file: run should call Conn::open, got: {callee_names:?}"
        );
        assert!(
            callee_names.contains(&"execute"),
            "cross-file: run should call c.execute() via local type inference, got: {callee_names:?}"
        );
    }

    #[test]
    fn test_is_test_precise_matching() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();

        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(
            repo.join("Cargo.toml"),
            "[package]\nname = \"test-app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();

        std::fs::write(
            repo.join("src/lib.rs"),
            r"
pub fn test_impact() {}

#[test]
fn test_real() {}

pub fn not_a_test() {}
",
        )
        .unwrap();
        std::fs::write(repo.join("src/main.rs"), "fn main() {}\n").unwrap();

        let db = Database::open_in_memory().unwrap();
        let config = IndexConfig {
            repo_path: repo.to_path_buf(),
        };
        index_repo(&db, &config).unwrap();

        let tests: Vec<String> = db
            .conn
            .prepare("SELECT name FROM symbols WHERE is_test = 1")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert!(
            tests.contains(&"test_real".to_string()),
            "test_real should be is_test=1"
        );
        assert!(
            !tests.contains(&"test_impact".to_string()),
            "test_impact (no #[test] attr) should NOT be is_test=1, \
             got: {tests:?}"
        );
    }
}
