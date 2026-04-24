pub mod cargo_doc;
pub mod dependencies;
pub mod docs;
pub mod parser;
pub mod py_imports;
pub mod py_parser;
pub mod store;
pub mod tauri_bridge;
pub mod ts_imports;
pub mod ts_parser;
pub mod workspace;

use crate::db::Database;
use std::path::PathBuf;

/// Directories to exclude from file scanning.
const EXCLUDED_DIRS: &[&str] = &[
    "target",
    "node_modules",
    "dist",
    ".next",
    "build",
    ".nuxt",
    ".output",
    "__pycache__",
    "venv",
    "env",
];

pub(crate) fn is_excluded_dir(name: &str) -> bool {
    EXCLUDED_DIRS.contains(&name) || name.starts_with('.') || name.ends_with(".egg-info")
}

fn is_source_file(ext: &std::ffi::OsStr) -> bool {
    matches!(
        ext.to_str(),
        Some("rs" | "ts" | "tsx" | "js" | "jsx" | "py")
    )
}

fn is_source_ts(ext: &std::ffi::OsStr) -> bool {
    matches!(ext.to_str(), Some("ts" | "tsx" | "js" | "jsx"))
}

fn is_ts_file(path: &str) -> bool {
    let p = std::path::Path::new(path);
    p.extension().is_some_and(is_source_ts)
}

fn is_py_file(path: &str) -> bool {
    let p = std::path::Path::new(path);
    p.extension().is_some_and(|ext| ext == "py")
}

/// Mark all symbols in a TS test file with a `"test"` attribute
/// so that `is_test_attribute` in `store.rs` sets `is_test = 1`.
fn mark_ts_test_symbols(symbols: &mut [parser::Symbol], path: &str) {
    if ts_parser::is_test_ts_file(path) {
        for sym in symbols.iter_mut() {
            sym.attributes = Some(
                sym.attributes
                    .as_ref()
                    .map_or_else(|| "test".to_string(), |a| format!("{a}, test")),
            );
        }
    }
}

/// Mark test symbols in Python files.
/// In test files (`test_*.py`, `*_test.py`, `tests/`): marks `test_*`
/// functions. In any file: marks `@pytest.mark.*` decorated functions.
fn mark_py_test_symbols(symbols: &mut [parser::Symbol], path: &str) {
    let is_test_file = py_parser::is_test_py_file(path);
    for sym in symbols.iter_mut() {
        let is_test = if is_test_file {
            sym.name.starts_with("test_")
        } else {
            sym.attributes
                .as_ref()
                .is_some_and(|a| a.contains("pytest.mark"))
        };
        if is_test {
            sym.attributes = Some(
                sym.attributes
                    .as_ref()
                    .map_or_else(|| "test".to_string(), |a| format!("{a}, test")),
            );
        }
    }
}

/// Check if a directory contains a Python project.
#[must_use]
pub fn has_python_project(repo_path: &std::path::Path) -> bool {
    repo_path.join("pyproject.toml").exists()
        || repo_path.join("setup.py").exists()
        || repo_path.join("setup.cfg").exists()
        || repo_path.join("requirements.txt").exists()
}

/// Extract the `"name"` field from a `package.json` content string.
fn package_name_from_json(content: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()?
        .get("name")?
        .as_str()
        .map(String::from)
}

/// Configuration for one indexing run.
///
/// `repo_path` must be the repository root used for every relative path written
/// to the database. Refresh, docs fetching, git diff tools, and source-body
/// recovery all assume the same root; constructing this with a subdirectory
/// produces an index that cannot be safely compared to git paths.
#[derive(Clone)]
pub struct IndexConfig {
    pub repo_path: PathBuf,
}

pub fn index_repo(db: &Database, config: &IndexConfig) -> Result<(), crate::IlluError> {
    db.clear_code_index()?;

    let has_cargo = config.repo_path.join("Cargo.toml").exists();
    let has_ts = config.repo_path.join("tsconfig.json").exists()
        || config.repo_path.join("package.json").exists();
    let has_python = has_python_project(&config.repo_path);

    // Phase 1: Parse sources
    if has_cargo {
        let cargo_toml = std::fs::read_to_string(config.repo_path.join("Cargo.toml"))?;
        let ws_info = workspace::parse_workspace_toml(&cargo_toml)?;
        if ws_info.is_workspace {
            tracing::info!(
                members = ws_info.members.len(),
                "Phase 1: indexing Rust workspace"
            );
            crate::status::set("indexing ▸ parsing workspace");
            index_workspace(db, config, &ws_info)?;
        } else {
            tracing::info!("Phase 1: indexing single Rust crate");
            crate::status::set("indexing ▸ parsing crate");
            index_single_crate(db, config, &cargo_toml)?;
        }
    }

    if has_ts {
        tracing::info!("Phase 1: indexing TypeScript sources");
        crate::status::set("indexing ▸ parsing TypeScript");
        index_typescript(db, config)?;
    }

    if has_python {
        tracing::info!("Phase 1: indexing Python sources");
        crate::status::set("indexing ▸ parsing Python");
        index_python(db, config)?;
    }

    if !has_cargo && !has_ts && !has_python {
        return Err(crate::IlluError::Workspace(format!(
            "No Cargo.toml, tsconfig.json, package.json, or Python project file found in {}",
            config.repo_path.display()
        )));
    }

    tracing::info!("Phase 2: extracting symbol references");
    crate::status::set("indexing ▸ extracting refs");
    extract_all_symbol_refs(db, config)?;

    // Phase 2b: resolve Tauri cross-language bridge
    if has_cargo && has_ts && tauri_bridge::is_tauri_project(&config.repo_path) {
        tracing::info!("Phase 2b: resolving Tauri bridge");
        crate::status::set("indexing ▸ Tauri bridge");
        let bridge_refs = tauri_bridge::resolve_tauri_bridge(db)?;
        if bridge_refs > 0 {
            tracing::info!(refs = bridge_refs, "Tauri bridge references resolved");
        }
    }

    tracing::info!("Phase 3: generating skill file");
    crate::status::set("indexing ▸ writing skill file");
    generate_skill_file(db, config)?;
    tracing::info!("Phase 4: updating metadata");
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

pub fn refresh_index(db: &Database, config: &IndexConfig) -> Result<usize, crate::IlluError> {
    let file_count = db.file_count()?;
    if file_count == 0 {
        tracing::info!("Empty index — running full index");
        index_repo(db, config)?;
        let new_count = db.file_count()?;
        return Ok(usize::try_from(new_count).unwrap_or(0));
    }

    // Version mismatch → full re-index to avoid stale data
    let stored_version = db
        .get_index_version(&config.repo_path.display().to_string())
        .unwrap_or(None);
    if stored_version.as_deref() != Some(INDEX_VERSION) {
        tracing::info!(
            stored = stored_version.as_deref().unwrap_or("none"),
            current = INDEX_VERSION,
            "Index version mismatch — running full re-index"
        );
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

    // Detect committed changes since last indexed commit
    let stored_hash = db.get_commit_hash(&config.repo_path.display().to_string())?;
    let current_head = get_current_commit_hash(&config.repo_path).ok();
    let head_changed = match (&stored_hash, &current_head) {
        (Some(old), Some(new)) => old != new,
        (None, Some(_)) => true,
        _ => false,
    };
    let committed_changes = if head_changed {
        if let Some(old) = &stored_hash {
            committed_changed_source_files(&config.repo_path, old)
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    // Try git-based detection first, fall back to full walk
    let mut candidate_files = git_changed_source_files(&config.repo_path, &existing);

    // Merge in committed changes not already in the candidate list
    if !committed_changes.is_empty() {
        let existing_set: std::collections::HashSet<&str> =
            candidate_files.iter().map(String::as_str).collect();
        let new: Vec<String> = committed_changes
            .into_iter()
            .filter(|p| !existing_set.contains(p.as_str()))
            .collect();
        candidate_files.extend(new);
        tracing::debug!("Merged committed changes into candidates");
    }

    let dirty_files = collect_dirty_files(&config.repo_path, &candidate_files, &existing);

    // Check for deleted files (only in full-walk mode, git handles this via status)
    for path in existing.keys() {
        let full = config.repo_path.join(path);
        if !full.exists() {
            db.delete_file_data(path)?;
        }
    }

    if dirty_files.is_empty() {
        // Freshness compares stored hash to HEAD; keep it current
        if head_changed {
            update_metadata(db, config)?;
        }
        crate::status::set(crate::status::READY);
        return Ok(0);
    }

    let count = dirty_files.len();
    tracing::info!(files = count, "Re-indexing changed files");
    crate::status::set(&format!("refreshing ▸ {count} files"));

    // Snapshot the symbol universe before reindexing so we can tell if
    // dirty-file edits introduced or removed cross-file targets. Without
    // this, refs in *non-dirty* files that mention a newly-added target
    // would silently never be created — they were unresolvable on their
    // original indexing pass and never revisited. Empirically this leaked
    // ~37% of symbol refs on incrementally-refreshed repos.
    let known_before = db.get_all_symbol_names()?;

    reindex_dirty_files(db, &dirty_files, &config.repo_path)?;

    let known_after = db.get_all_symbol_names()?;
    let universe_grew = known_after.iter().any(|name| !known_before.contains(name));
    let universe_shrank = known_before.iter().any(|name| !known_after.contains(name));
    if universe_grew || universe_shrank {
        let changed: std::collections::HashSet<&str> = known_after
            .symmetric_difference(&known_before)
            .map(String::as_str)
            .collect();
        tracing::info!(
            changed = changed.len(),
            "Symbol universe changed — rebuilding refs for files that mention the affected names"
        );
        rebuild_refs_for_universe_change(db, config, &dirty_files, &changed)?;
    }

    let stale = db.delete_stale_refs()?;
    if stale > 0 {
        tracing::info!(deleted = stale, "Cleaned up stale symbol refs");
    }

    // Update stored commit hash so freshness reports correctly
    update_metadata(db, config)?;

    crate::status::set(crate::status::READY);
    Ok(count)
}

/// Rebuild outgoing refs for any non-dirty file whose source text mentions
/// a symbol name that was added or removed by the current refresh.
///
/// Substring matching is intentionally coarse: a hit may be inside a
/// comment or string literal, in which case the re-extracted ref count for
/// that file is simply `0` after parsing — no false rows land in the DB.
/// False negatives, by contrast, would reintroduce the original leak, so
/// the match is keyed on the exact identifier text the parser would see.
fn rebuild_refs_for_universe_change(
    db: &Database,
    config: &IndexConfig,
    already_dirty: &[DirtyFile],
    changed_names: &std::collections::HashSet<&str>,
) -> Result<(), crate::IlluError> {
    if changed_names.is_empty() {
        return Ok(());
    }

    let dirty_paths: std::collections::HashSet<&str> = already_dirty
        .iter()
        .map(|d| d.relative_path.as_str())
        .collect();

    let all_paths = db.get_all_file_paths()?;
    let candidates: Vec<String> = all_paths
        .into_iter()
        .filter(|p| !dirty_paths.contains(p.as_str()))
        .collect();

    if candidates.is_empty() {
        return Ok(());
    }

    let known_symbols = db.get_all_symbol_names()?;
    let all_crates = db.get_all_crates()?;
    let crate_map: std::collections::HashMap<String, String> = all_crates
        .iter()
        .map(|c| (c.name.replace('-', "_"), c.path.clone()))
        .collect();
    let symbol_map = db.build_symbol_id_map()?;

    let mut touched = 0usize;
    db.begin_transaction()?;
    for relative in &candidates {
        let full = config.repo_path.join(relative);
        let source = match std::fs::read_to_string(&full) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(path = %relative, error = %e, "skipping during ref rebuild");
                continue;
            }
        };
        if !changed_names.iter().any(|name| source.contains(name)) {
            continue;
        }
        db.delete_refs_from_file(relative)?;
        let refs = if is_ts_file(relative) {
            ts_parser::extract_ts_refs(&source, relative, &known_symbols, &config.repo_path)
                .map_err(crate::IlluError::Indexing)?
        } else if is_py_file(relative) {
            py_parser::extract_py_refs(&source, relative, &known_symbols, &config.repo_path)
                .map_err(crate::IlluError::Indexing)?
        } else {
            parser::extract_refs(&source, relative, &known_symbols, &crate_map)
                .map_err(crate::IlluError::Indexing)?
        };
        db.store_symbol_refs_fast(&refs, &symbol_map)?;
        touched += 1;
    }
    db.commit()?;

    tracing::info!(
        files = touched,
        "Rebuilt refs for universe-change affected files"
    );
    Ok(())
}

fn reindex_dirty_files(
    db: &Database,
    dirty_files: &[DirtyFile],
    repo_path: &std::path::Path,
) -> Result<(), crate::IlluError> {
    let count = dirty_files.len();
    for (i, df) in dirty_files.iter().enumerate() {
        crate::status::set(&format!("refreshing ▸ [{}/{}]", i + 1, count));
        tracing::debug!("[{}/{}] Re-indexing {}", i + 1, count, df.relative_path);
        db.delete_file_data(&df.relative_path)?;
        let file_id = if let Some(cid) = df.crate_id {
            db.insert_file_with_crate(&df.relative_path, &df.hash, cid)?
        } else {
            db.insert_file(&df.relative_path, &df.hash)?
        };
        let (symbols, trait_impls) = if is_ts_file(&df.relative_path) {
            let (mut syms, ti) = ts_parser::parse_ts_source(&df.source, &df.relative_path)
                .map_err(crate::IlluError::Indexing)?;
            mark_ts_test_symbols(&mut syms, &df.relative_path);
            (syms, ti)
        } else if is_py_file(&df.relative_path) {
            let (mut syms, ti) = py_parser::parse_py_source(&df.source, &df.relative_path)
                .map_err(crate::IlluError::Indexing)?;
            mark_py_test_symbols(&mut syms, &df.relative_path);
            (syms, ti)
        } else {
            parser::parse_rust_source(&df.source, &df.relative_path)
                .map_err(crate::IlluError::Indexing)?
        };
        store::store_symbols(db, file_id, &symbols)?;
        store::store_trait_impls(db, file_id, &trait_impls)?;
    }

    rebuild_refs_for_files(db, dirty_files, repo_path)?;
    Ok(())
}

fn rebuild_refs_for_files(
    db: &Database,
    dirty_files: &[DirtyFile],
    repo_path: &std::path::Path,
) -> Result<(), crate::IlluError> {
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
        let refs = if is_ts_file(&df.relative_path) {
            ts_parser::extract_ts_refs(&df.source, &df.relative_path, &known_symbols, repo_path)
                .map_err(crate::IlluError::Indexing)?
        } else if is_py_file(&df.relative_path) {
            py_parser::extract_py_refs(&df.source, &df.relative_path, &known_symbols, repo_path)
                .map_err(crate::IlluError::Indexing)?
        } else {
            parser::extract_refs(&df.source, &df.relative_path, &known_symbols, &crate_map)
                .map_err(crate::IlluError::Indexing)?
        };
        db.store_symbol_refs_fast(&refs, &symbol_map)?;
    }
    db.commit()?;
    Ok(())
}

fn index_single_crate(
    db: &Database,
    config: &IndexConfig,
    cargo_toml: &str,
) -> Result<(), crate::IlluError> {
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
) -> Result<(), crate::IlluError> {
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

fn index_typescript(db: &Database, config: &IndexConfig) -> Result<(), crate::IlluError> {
    // Parse package.json for name and dependencies
    let pkg_json_path = config.repo_path.join("package.json");
    let (pkg_name, pkg_content) = if pkg_json_path.exists() {
        let content = std::fs::read_to_string(&pkg_json_path)?;
        let name = package_name_from_json(&content).unwrap_or_else(|| "ts-frontend".to_string());
        (name, Some(content))
    } else {
        ("ts-frontend".to_string(), None)
    };

    // Store npm dependencies
    if let Some(content) = &pkg_content
        && let Ok(deps) = dependencies::parse_package_json(content)
    {
        let resolved: Vec<_> = deps
            .iter()
            .map(dependencies::ResolvedDep::from_direct)
            .collect();
        store::store_dependencies(db, &resolved)?;
    }

    // Handle npm workspaces
    let workspace_patterns = ts_imports::parse_npm_workspaces(&config.repo_path);
    let workspace_members =
        ts_imports::resolve_workspace_members(&config.repo_path, &workspace_patterns);

    if workspace_members.is_empty() {
        // Single package
        let crate_id = db.insert_crate(&pkg_name, ".")?;
        index_ts_files(db, config, &config.repo_path, crate_id)?;
    } else {
        // npm workspace — register each member
        tracing::info!(members = workspace_members.len(), "Detected npm workspace");
        for member_path in &workspace_members {
            let member_pkg = member_path.join("package.json");
            let member_name = std::fs::read_to_string(&member_pkg)
                .ok()
                .and_then(|c| package_name_from_json(&c))
                .or_else(|| {
                    member_path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                })
                .unwrap_or_else(|| "unknown".to_string());
            let member_rel = member_path
                .strip_prefix(&config.repo_path)
                .unwrap_or(member_path)
                .to_string_lossy()
                .to_string();
            let crate_id = db.insert_crate(&member_name, &member_rel)?;
            index_ts_files(db, config, member_path, crate_id)?;
        }
    }

    Ok(())
}

fn index_ts_files(
    db: &Database,
    config: &IndexConfig,
    root: &std::path::Path,
    crate_id: crate::db::CrateId,
) -> Result<(), crate::IlluError> {
    // Walk for TS/TSX/JS/JSX files (excluding node_modules, etc.)
    let ts_files: Vec<_> = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| {
            if !e.file_type().is_dir() || e.depth() == 0 {
                return true;
            }
            let name = e.file_name().to_string_lossy();
            !is_excluded_dir(&name) && name != "src-tauri"
        })
        .filter_map(|r| match r {
            Ok(e) if e.path().extension().is_some_and(is_source_ts) => Some(e.into_path()),
            _ => None,
        })
        .collect();

    let total = ts_files.len();
    tracing::info!(files = total, "Found TypeScript files");

    for (i, path) in ts_files.iter().enumerate() {
        crate::status::set(&format!("indexing ▸ TS [{}/{}]", i + 1, total));
        let source = std::fs::read_to_string(path)?;
        let relative = path
            .strip_prefix(&config.repo_path)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        tracing::debug!("[{}/{}] Parsing TS {relative}", i + 1, total);
        let hash = content_hash(&source);
        let file_id = db.insert_file_with_crate(&relative, &hash, crate_id)?;
        let (mut symbols, trait_impls) =
            ts_parser::parse_ts_source(&source, &relative).map_err(crate::IlluError::Indexing)?;

        mark_ts_test_symbols(&mut symbols, &relative);

        store::store_symbols(db, file_id, &symbols)?;
        store::store_trait_impls(db, file_id, &trait_impls)?;
    }
    tracing::info!(files = total, "Parsed TypeScript files");
    Ok(())
}

fn index_python(db: &Database, config: &IndexConfig) -> Result<(), crate::IlluError> {
    let pyproject_path = config.repo_path.join("pyproject.toml");
    let pkg_name = if pyproject_path.exists() {
        let content = std::fs::read_to_string(&pyproject_path)?;
        py_imports::extract_project_name(&content).unwrap_or_else(|| "python-project".to_string())
    } else {
        "python-project".to_string()
    };

    // Store Python dependencies
    if let Ok(deps) = dependencies::parse_python_deps(&config.repo_path) {
        let resolved: Vec<_> = deps
            .iter()
            .map(dependencies::ResolvedDep::from_direct)
            .collect();
        store::store_dependencies(db, &resolved)?;
    }

    let crate_id = db.insert_crate(&pkg_name, ".")?;
    index_py_files(db, config, &config.repo_path, crate_id)?;
    Ok(())
}

fn index_py_files(
    db: &Database,
    config: &IndexConfig,
    root: &std::path::Path,
    crate_id: crate::db::CrateId,
) -> Result<(), crate::IlluError> {
    let py_files: Vec<_> = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| {
            if !e.file_type().is_dir() || e.depth() == 0 {
                return true;
            }
            let name = e.file_name().to_string_lossy();
            !is_excluded_dir(&name)
        })
        .filter_map(|r| match r {
            Ok(e) if e.path().extension().is_some_and(|ext| ext == "py") => Some(e.into_path()),
            _ => None,
        })
        .collect();

    let total = py_files.len();
    tracing::info!(files = total, "Found Python files");

    for (i, path) in py_files.iter().enumerate() {
        crate::status::set(&format!("indexing ▸ Py [{}/{}]", i + 1, total));
        let source = std::fs::read_to_string(path)?;
        let relative = path
            .strip_prefix(&config.repo_path)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        tracing::debug!("[{}/{}] Parsing Py {relative}", i + 1, total);
        let hash = content_hash(&source);
        let file_id = db.insert_file_with_crate(&relative, &hash, crate_id)?;
        let (mut symbols, trait_impls) = py_parser::parse_py_source(&source, &relative)
            .map_err(|e| crate::IlluError::Indexing(format!("{e}: {relative}")))?;

        mark_py_test_symbols(&mut symbols, &relative);

        store::store_symbols(db, file_id, &symbols)?;
        store::store_trait_impls(db, file_id, &trait_impls)?;
    }
    tracing::info!(files = total, "Parsed Python files");
    Ok(())
}

fn index_crate_sources(
    db: &Database,
    config: &IndexConfig,
    src_dir: &std::path::Path,
    crate_id: crate::db::CrateId,
) -> Result<(), crate::IlluError> {
    // Collect files first so we can report progress
    let rs_files: Vec<_> = walkdir::WalkDir::new(src_dir)
        .into_iter()
        .filter_entry(|e| {
            if !e.file_type().is_dir() || e.depth() == 0 {
                return true;
            }
            let name = e.file_name().to_string_lossy();
            !is_excluded_dir(&name)
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
        let (symbols, trait_impls) =
            parser::parse_rust_source(&source, &relative).map_err(crate::IlluError::Indexing)?;
        store::store_symbols(db, file_id, &symbols)?;
        store::store_trait_impls(db, file_id, &trait_impls)?;
    }
    tracing::info!(files = total, "Parsed source files");
    Ok(())
}

fn extract_all_symbol_refs(db: &Database, config: &IndexConfig) -> Result<(), crate::IlluError> {
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
        let refs = if is_ts_file(relative) {
            ts_parser::extract_ts_refs(&source, relative, &known_symbols, &config.repo_path)
                .map_err(crate::IlluError::Indexing)?
        } else if is_py_file(relative) {
            py_parser::extract_py_refs(&source, relative, &known_symbols, &config.repo_path)
                .map_err(crate::IlluError::Indexing)?
        } else {
            parser::extract_refs(&source, relative, &known_symbols, &crate_map)
                .map_err(crate::IlluError::Indexing)?
        };
        ref_count += db.store_symbol_refs_fast(&refs, &symbol_map)?;
    }
    db.commit()?;

    tracing::info!(refs = ref_count, "Symbol reference extraction complete");
    Ok(())
}

fn generate_skill_file(db: &Database, config: &IndexConfig) -> Result<(), crate::IlluError> {
    let direct_deps = db.get_direct_dependencies()?;
    let dep_names: Vec<&str> = direct_deps.iter().map(|d| d.name.as_str()).collect();
    let skill_content = generate_claude_skill(&dep_names);
    let skill_dir = config.repo_path.join(".claude").join("skills");
    std::fs::create_dir_all(&skill_dir)?;
    std::fs::write(skill_dir.join("illu-rs.md"), &skill_content)?;
    tracing::info!("Wrote Claude skill to .claude/skills/illu-rs.md");
    Ok(())
}

/// Key used to detect stale on-disk indexes. Combines the crate version
/// with a monotonic schema revision suffix (`+schema.N`) so we can force
/// a full re-index when ref-extraction or symbol-storage logic changes
/// in a way that invalidates existing rows — without having to bump the
/// crate version just for that.
///
/// Bump the suffix when: a new ref kind is emitted; call-site context
/// broadens; a bug fix should retroactively reindex. Suffix history:
///   - `schema.1` (PR #84): ref-extraction fix for `super::fn` call
///     sites; earlier indexes may have missed handler-file refs.
// schema.2: fix incremental refresh leaking cross-file refs when the symbol
// universe changes. Existing schema.1 indexes are structurally compatible
// but arithmetically stale — we bump so they rebuild once on upgrade and
// callers see the missing refs that `refresh_index` now preserves.
pub const INDEX_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "+schema.2");

/// True iff the stored index is out of date relative to the binary's
/// schema version or the repo's current HEAD. The canonical rule —
/// both the `freshness` MCP tool and the dashboard defer to this so
/// the two surfaces cannot disagree about what "stale" means.
#[must_use]
pub fn is_index_stale(
    stored_version: Option<&str>,
    indexed_commit: Option<&str>,
    current_head: Option<&str>,
) -> bool {
    stored_version != Some(INDEX_VERSION) || indexed_commit != current_head
}

fn update_metadata(db: &Database, config: &IndexConfig) -> Result<(), crate::IlluError> {
    let commit_hash =
        get_current_commit_hash(&config.repo_path).unwrap_or_else(|_| "unknown".to_string());
    db.set_metadata(
        &config.repo_path.display().to_string(),
        &commit_hash,
        INDEX_VERSION,
    )?;
    Ok(())
}

fn parse_cargo_lock(
    repo_path: &std::path::Path,
) -> Result<Vec<dependencies::LockedDep>, crate::IlluError> {
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
                "Search symbols, docs, files, bodies, or string literals. Filters: kind, attribute, signature, path.",
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
        "Rust Quality",
        &[
            (
                "axioms",
                "Rust rules, safety constraints, and best-practice guidance.",
            ),
            (
                "rust_preflight",
                "Required evidence packet before Rust design/code: axioms, symbol context, impact hints, std/dependency docs, and model-failure reminders.",
            ),
            (
                "std_docs",
                "Local standard-library rustdoc lookup for items and methods.",
            ),
            (
                "quality_gate",
                "PASS/WARN/BLOCKED check for Rust diff evidence before final answer or commit.",
            ),
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
                "Export call or file graphs as DOT, compact edge list, or summary.",
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
    (
        "Cross-Repo",
        &[
            (
                "repos",
                "Dashboard of all registered repos with status and symbol counts.",
            ),
            ("cross_query", "Search symbols across all registered repos."),
            (
                "cross_impact",
                "Find references to a symbol in other repos.",
            ),
            (
                "cross_deps",
                "Inter-repo dependency relationships via Cargo.toml.",
            ),
            (
                "cross_callpath",
                "Find call chains spanning repo boundaries.",
            ),
        ],
    ),
    (
        "rust-analyzer (compiler-accurate, positions use file:line:col)",
        &[
            (
                "ra_definition",
                "Go to definition — resolves through macros, trait impls, generics.",
            ),
            (
                "ra_hover",
                "Type information and documentation at a position.",
            ),
            (
                "ra_diagnostics",
                "Compilation errors and warnings, optionally filtered by file.",
            ),
            (
                "ra_call_hierarchy",
                "Callers and/or callees at a position (direction: in/out/both).",
            ),
            (
                "ra_type_hierarchy",
                "Supertypes (traits) and subtypes for a type.",
            ),
            (
                "ra_rename",
                "Preview rename impact: affected files and reference counts.",
            ),
            (
                "ra_safe_rename",
                "Apply a rename with compilation error checking.",
            ),
            (
                "ra_code_actions",
                "Available quick fixes and refactors at a position.",
            ),
            (
                "ra_expand_macro",
                "Expand macro at a position, showing generated code.",
            ),
            (
                "ra_ssr",
                "Structural search and replace (e.g. `foo($a) ==>> bar($a)`).",
            ),
            (
                "ra_context",
                "Full compiler-accurate context: definition, hover, callers, callees, impls, tests.",
            ),
            (
                "ra_syntax_tree",
                "Show syntax tree for a file (debugging/parse structure).",
            ),
            (
                "ra_related_tests",
                "Find tests related to a symbol — more accurate than text matching.",
            ),
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
    use crate::agents::instruction_md::RUST_QUALITY_QUERY;
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

    let _ = writeln!(out, "## Rust Design Discipline\n");
    let _ = writeln!(
        out,
        "Before writing, modifying, or recommending Rust code, do these in order:\n\n\
         1. Run `rust_preflight` first to gather axioms, local symbol evidence, \
         impact hints, std/dependency docs, and model-failure reminders.\n\
         2. Plan first after preflight — name the data flow, invariants, failure cases, and \
         the concrete types (structs / enums / newtypes / collections) you will use.\n\
         3. Choose data structures deliberately; prefer representations that make \
         invalid states unrepresentable.\n\
         4. Read the docs before assuming any non-trivial API's behavior. \
         Standard-library items require `std_docs`; dependencies use `docs`; \
         local types use `context`.\n\
         5. Query `axioms` twice if preflight did not already supply both: once \
         with `{RUST_QUALITY_QUERY}` and once with the concrete task context.\n\
         6. Write idiomatic Rust per The Rust Book, Rust for Rustaceans, and \
         illu axioms — ownership/borrowing, enums, iterators, explicit errors.\n\
         7. Comments must explain invariants, safety, ownership rationale, or \
         why the design exists — never narrate syntax.\n\n\
         Before final answer or commit for a Rust diff, run `quality_gate` with \
         the plan, docs verified, impact checked, and tests run. `BLOCKED` means \
         the work is not ready.\n\n\
         Full rules: see the `Rust Design Discipline` section of CLAUDE.md or \
         GEMINI.md in the repo.\n"
    );

    let _ = writeln!(out, "## Direct Dependencies\n");

    if direct_dep_names.is_empty() {
        let _ = writeln!(out, "No direct dependencies found.");
    } else {
        let mut unique: Vec<&str> = direct_dep_names.to_vec();
        unique.sort_unstable();
        unique.dedup();
        for dep in &unique {
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

/// Find source files changed in commits since `since_hash` up to HEAD.
fn committed_changed_source_files(repo_path: &std::path::Path, since_hash: &str) -> Vec<String> {
    let output = std::process::Command::new("git")
        .args(["diff", "--name-only", &format!("{since_hash}..HEAD")])
        .current_dir(repo_path)
        .output();
    let Ok(output) = output else {
        tracing::debug!("git diff --name-only failed to execute");
        return Vec::new();
    };
    if !output.status.success() {
        tracing::debug!("git diff --name-only returned non-zero");
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| {
            std::path::Path::new(l)
                .extension()
                .is_some_and(is_source_file)
        })
        .map(String::from)
        .collect()
}

/// Use `git status` to find changed/new/deleted source files.
/// Returns a list of relative paths to check. If git fails, returns all
/// indexed files plus walks for new ones (full scan fallback).
fn git_changed_source_files(
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
        return full_scan_source_files(repo_path);
    };
    if !output.status.success() {
        tracing::debug!("git status returned non-zero, falling back to full scan");
        return full_scan_source_files(repo_path);
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
            .is_some_and(is_source_file)
        {
            changed.push(path.to_string());
        }
    }

    // Also check for new .rs files not yet tracked by git but present on disk
    // and not yet in our index (e.g., files in .gitignore that we still want)
    // For now, the git status output covers new untracked files ("?? path").

    // Also include files that are in our index but might have been modified
    // outside of git tracking (rare but possible)
    let changed_set: std::collections::HashSet<String> = changed.iter().cloned().collect();
    for path in existing.keys() {
        if !changed_set.contains(path) {
            let full = repo_path.join(path);
            if !full.exists() {
                changed.push(path.clone());
            }
        }
    }

    tracing::debug!(count = changed.len(), "git detected changed source files");
    changed
}

/// Fallback: walk the repo for all source files.
fn full_scan_source_files(repo_path: &std::path::Path) -> Vec<String> {
    let mut files = Vec::new();
    let walker = walkdir::WalkDir::new(repo_path)
        .into_iter()
        .filter_entry(|e| {
            if !e.file_type().is_dir() || e.depth() == 0 {
                return true;
            }
            let name = e.file_name().to_string_lossy();
            !is_excluded_dir(&name)
        });
    for result in walker {
        let Ok(entry) = result else { continue };
        let path = entry.path();
        if path.extension().is_some_and(is_source_file) {
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

fn get_current_commit_hash(repo_path: &std::path::Path) -> Result<String, crate::IlluError> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_path)
        .output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(crate::IlluError::Git("rev-parse HEAD failed".to_string()))
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
        assert!(skill.contains("53 available"));
        assert!(skill.contains("query"));
        assert!(skill.contains("context"));
        assert!(skill.contains("impact"));
        assert!(skill.contains("diff_impact"));
        assert!(skill.contains("rust_preflight"));
        assert!(skill.contains("std_docs"));
        assert!(skill.contains("quality_gate"));
        assert!(skill.contains("test_impact"));
        assert!(skill.contains("neighborhood"));
        assert!(skill.contains("boundary"));
        assert!(skill.contains("orphaned"));
        assert!(skill.contains("blame"));
        assert!(skill.contains("overview"));
        assert!(skill.contains("docs"));
        assert!(skill.contains("ra_definition"));
        assert!(skill.contains("ra_hover"));
        assert!(skill.contains("ra_rename"));
        assert!(skill.contains("ra_safe_rename"));
        assert!(skill.contains("ra_context"));
        assert!(skill.contains("ra_expand_macro"));
        assert!(skill.contains("ra_ssr"));
        // Rust Design Discipline block — keeps the skill file in sync with
        // CLAUDE.md so the model sees the same rules from both load paths.
        assert!(skill.contains("Rust Design Discipline"));
        assert!(skill.contains("Plan first"));
        assert!(skill.contains("planning data structures documentation comments idiomatic rust"));
    }

    #[test]
    fn test_generate_skill_no_deps() {
        let skill = generate_claude_skill(&[]);
        assert!(skill.contains("No direct dependencies"));
        assert!(skill.contains("Rust Design Discipline"));
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
