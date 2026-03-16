pub mod dependencies;
pub mod docs;
pub mod parser;
pub mod store;
pub mod workspace;

use crate::db::Database;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

pub struct IndexConfig {
    pub repo_path: PathBuf,
    pub skip_doc_fetch: bool,
}

pub fn index_repo(db: &Database, config: &IndexConfig) -> Result<(), Box<dyn std::error::Error>> {
    // Clear stale data from previous indexing runs
    db.clear_index()?;

    // Phase 1: Parse dependencies
    let cargo_toml_path = config.repo_path.join("Cargo.toml");
    let cargo_toml = std::fs::read_to_string(&cargo_toml_path)?;
    let direct = dependencies::parse_cargo_toml(&cargo_toml)?;

    let locked = match std::fs::read_to_string(config.repo_path.join("Cargo.lock")) {
        Ok(lock) => dependencies::parse_cargo_lock(&lock)?,
        Err(_) => vec![],
    };

    let resolved = dependencies::resolve_dependencies(&direct, &locked);
    store::store_dependencies(db, &resolved)?;

    // Phase 2: Parse source files
    let src_dir = config.repo_path.join("src");
    if src_dir.exists() {
        for entry in walkdir::WalkDir::new(&src_dir)
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
                let file_id = db.insert_file(&relative, &hash)?;
                let symbols = parser::parse_rust_source(&source, &relative)
                    .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
                store::store_symbols(db, file_id, &symbols)?;
            }
        }
    }

    // Phase 2.5: Extract symbol references
    let known_symbols = db.get_all_symbol_names()?;
    if !known_symbols.is_empty() {
        let src_dir = config.repo_path.join("src");
        if src_dir.exists() {
            let mut ref_count: u64 = 0;
            for entry in walkdir::WalkDir::new(&src_dir)
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
                    let refs = parser::extract_refs(&source, &relative, &known_symbols)
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
            }
            tracing::info!("Stored {ref_count} symbol references");
        }
    }

    // Phase 3: Doc fetching skipped if configured
    // (async doc fetching handled separately)

    // Phase 4: Generate Claude skill file
    let direct_deps = db.get_direct_dependencies()?;
    let dep_names: Vec<String> = direct_deps.iter().map(|d| d.name.clone()).collect();
    let skill_content = generate_claude_skill(&dep_names);
    let skill_dir = config.repo_path.join(".claude").join("skills");
    std::fs::create_dir_all(&skill_dir)?;
    std::fs::write(skill_dir.join("illu-rs.md"), &skill_content)?;
    tracing::info!("Wrote Claude skill to .claude/skills/illu-rs.md");

    // Phase 5: Update metadata
    let commit_hash =
        get_current_commit_hash(&config.repo_path).unwrap_or_else(|_| "unknown".to_string());
    db.set_metadata(&config.repo_path.display().to_string(), &commit_hash)?;

    Ok(())
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
}
