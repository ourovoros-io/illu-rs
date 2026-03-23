use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepoEntry {
    pub name: String,
    pub path: PathBuf,
    pub git_remote: Option<String>,
    pub git_common_dir: PathBuf,
    pub last_indexed: String,
}

#[derive(Deserialize, Serialize, Default)]
struct RegistryFile {
    #[serde(default)]
    repos: Vec<RepoEntry>,
}

pub struct Registry {
    file_path: PathBuf,
    pub repos: Vec<RepoEntry>,
}

impl Registry {
    /// Load registry from a TOML file, or create empty if it
    /// doesn't exist.
    pub fn load(path: &Path) -> Result<Self, String> {
        if path.exists() {
            let content = std::fs::read_to_string(path)
                .map_err(|e| format!("failed to read registry: {e}"))?;
            let file: RegistryFile =
                toml::from_str(&content).map_err(|e| format!("failed to parse registry: {e}"))?;
            Ok(Self {
                file_path: path.to_path_buf(),
                repos: file.repos,
            })
        } else {
            Ok(Self {
                file_path: path.to_path_buf(),
                repos: Vec::new(),
            })
        }
    }

    /// Add or update a repo entry. Deduplicates by
    /// `git_common_dir`. When updating an existing entry, keeps the
    /// original path (main repo) but updates timestamp and remote.
    pub fn register(&mut self, entry: RepoEntry) {
        if let Some(existing) = self
            .repos
            .iter_mut()
            .find(|r| r.git_common_dir == entry.git_common_dir)
        {
            existing.last_indexed = entry.last_indexed;
            existing.git_remote = entry.git_remote;
        } else {
            self.repos.push(entry);
        }
    }

    /// Remove entries whose `path` no longer exists on disk.
    pub fn prune(&mut self) {
        self.repos.retain(|r| r.path.exists());
    }

    /// Write registry to disk, creating parent directories as needed.
    pub fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.file_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create registry dir: {e}"))?;
        }
        let file = RegistryFile {
            repos: self.repos.clone(),
        };
        let content = toml::to_string_pretty(&file)
            .map_err(|e| format!("failed to serialize registry: {e}"))?;
        std::fs::write(&self.file_path, content)
            .map_err(|e| format!("failed to write registry: {e}"))?;
        Ok(())
    }

    /// Return all repos except the one matching `primary_path`.
    #[must_use]
    pub fn other_repos(&self, primary_path: &Path) -> Vec<&RepoEntry> {
        self.repos
            .iter()
            .filter(|r| r.path != primary_path)
            .collect()
    }

    /// Create an empty registry with no file path (fallback).
    #[must_use]
    pub fn empty() -> Self {
        Self {
            file_path: PathBuf::new(),
            repos: Vec::new(),
        }
    }

    /// Default registry path: `~/.illu/registry.toml`.
    pub fn default_path() -> Result<PathBuf, String> {
        let home =
            std::env::var("HOME").map_err(|_| "HOME environment variable not set".to_string())?;
        Ok(PathBuf::from(home).join(".illu").join("registry.toml"))
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    fn make_entry(name: &str, path: PathBuf, common_dir: PathBuf, timestamp: &str) -> RepoEntry {
        RepoEntry {
            name: name.to_string(),
            path,
            git_remote: None,
            git_common_dir: common_dir,
            last_indexed: timestamp.to_string(),
        }
    }

    #[test]
    fn register_new_repo() {
        let dir = tempfile::tempdir().unwrap();
        let reg_path = dir.path().join("registry.toml");

        let mut reg = Registry::load(&reg_path).unwrap();
        assert!(reg.repos.is_empty());

        let entry = make_entry(
            "myrepo",
            dir.path().to_path_buf(),
            dir.path().join(".git"),
            "2026-01-01T00:00:00Z",
        );
        reg.register(entry);

        assert_eq!(reg.repos.len(), 1);
        assert_eq!(reg.repos[0].name, "myrepo");

        reg.save().unwrap();

        let reloaded = Registry::load(&reg_path).unwrap();
        assert_eq!(reloaded.repos.len(), 1);
        assert_eq!(reloaded.repos[0].name, "myrepo");
    }

    #[test]
    fn register_updates_existing() {
        let dir = tempfile::tempdir().unwrap();
        let reg_path = dir.path().join("registry.toml");
        let common = dir.path().join(".git");

        let mut reg = Registry::load(&reg_path).unwrap();

        let entry1 = make_entry(
            "repo",
            dir.path().to_path_buf(),
            common.clone(),
            "2026-01-01T00:00:00Z",
        );
        reg.register(entry1);

        let entry2 = make_entry(
            "repo",
            dir.path().to_path_buf(),
            common,
            "2026-02-01T00:00:00Z",
        );
        reg.register(entry2);

        assert_eq!(reg.repos.len(), 1);
        assert_eq!(reg.repos[0].last_indexed, "2026-02-01T00:00:00Z");
    }

    #[test]
    fn worktree_dedup_by_common_dir() {
        let dir = tempfile::tempdir().unwrap();
        let reg_path = dir.path().join("registry.toml");
        let common = dir.path().join(".git");
        let main_path = dir.path().join("main");
        let wt_path = dir.path().join("worktree");

        let mut reg = Registry::load(&reg_path).unwrap();

        let main_entry = make_entry(
            "repo",
            main_path.clone(),
            common.clone(),
            "2026-01-01T00:00:00Z",
        );
        reg.register(main_entry);

        let wt_entry = make_entry("repo-wt", wt_path, common, "2026-02-01T00:00:00Z");
        reg.register(wt_entry);

        assert_eq!(reg.repos.len(), 1);
        assert_eq!(reg.repos[0].path, main_path);
        assert_eq!(reg.repos[0].last_indexed, "2026-02-01T00:00:00Z");
    }

    #[test]
    fn prune_removes_dead_paths() {
        let dir = tempfile::tempdir().unwrap();
        let reg_path = dir.path().join("registry.toml");

        let mut reg = Registry::load(&reg_path).unwrap();

        let alive = make_entry(
            "alive",
            dir.path().to_path_buf(),
            dir.path().join(".git-alive"),
            "2026-01-01T00:00:00Z",
        );
        let dead = make_entry(
            "dead",
            PathBuf::from("/nonexistent/path"),
            PathBuf::from("/nonexistent/.git"),
            "2026-01-01T00:00:00Z",
        );
        reg.register(alive);
        reg.register(dead);
        assert_eq!(reg.repos.len(), 2);

        reg.prune();

        assert_eq!(reg.repos.len(), 1);
        assert_eq!(reg.repos[0].name, "alive");
    }

    #[test]
    fn other_repos_excludes_primary() {
        let dir = tempfile::tempdir().unwrap();
        let reg_path = dir.path().join("registry.toml");
        let path_a = dir.path().join("repo-a");
        let path_b = dir.path().join("repo-b");

        let mut reg = Registry::load(&reg_path).unwrap();

        let entry_a = make_entry(
            "repo-a",
            path_a.clone(),
            dir.path().join(".git-a"),
            "2026-01-01T00:00:00Z",
        );
        let entry_b = make_entry(
            "repo-b",
            path_b,
            dir.path().join(".git-b"),
            "2026-01-01T00:00:00Z",
        );
        reg.register(entry_a);
        reg.register(entry_b);

        let others = reg.other_repos(&path_a);
        assert_eq!(others.len(), 1);
        assert_eq!(others[0].name, "repo-b");
    }
}
