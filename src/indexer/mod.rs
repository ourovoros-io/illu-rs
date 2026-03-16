pub mod dependencies;
pub mod docs;
pub mod parser;
pub mod store;

use crate::db::Database;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

pub struct IndexConfig {
    pub repo_path: PathBuf,
    pub skip_doc_fetch: bool,
}

pub fn index_repo(
    db: &Database,
    config: &IndexConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    // Phase 1: Parse dependencies
    let cargo_toml_path = config.repo_path.join("Cargo.toml");
    let cargo_toml = std::fs::read_to_string(&cargo_toml_path)?;
    let direct = dependencies::parse_cargo_toml(&cargo_toml)?;

    let locked = match std::fs::read_to_string(
        config.repo_path.join("Cargo.lock"),
    ) {
        Ok(lock) => dependencies::parse_cargo_lock(&lock)?,
        Err(_) => vec![],
    };

    let resolved =
        dependencies::resolve_dependencies(&direct, &locked);
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
                let symbols = parser::parse_rust_source(
                    &source, &relative,
                )
                .map_err(|e| -> Box<dyn std::error::Error> {
                    e.into()
                })?;
                store::store_symbols(db, file_id, &symbols)?;
            }
        }
    }

    // Phase 3: Doc fetching skipped if configured
    // (async doc fetching handled separately)

    // Phase 4: Update metadata
    let commit_hash =
        get_current_commit_hash(&config.repo_path)
            .unwrap_or_else(|_| "unknown".to_string());
    db.set_metadata(
        &config.repo_path.display().to_string(),
        &commit_hash,
    )?;

    Ok(())
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
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim()
            .to_string())
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
