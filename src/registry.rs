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

    /// Add or update a repo entry. Deduplicates by `git_common_dir`
    /// so multiple worktrees of the same repo share a single entry.
    /// When updating an existing entry, the current invocation's
    /// `path`, `name`, `git_remote`, and timestamp take over — the
    /// registry always reflects the most recently active checkout,
    /// which is where the live `.illu/index.db` lives.
    pub fn register(&mut self, entry: RepoEntry) {
        if let Some(existing) = self
            .repos
            .iter_mut()
            .find(|r| r.git_common_dir == entry.git_common_dir)
        {
            existing.name = entry.name;
            existing.path = entry.path;
            existing.git_remote = entry.git_remote;
            existing.last_indexed = entry.last_indexed;
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

    /// Return all repos except the one identifying as primary.
    ///
    /// Exclusion is keyed on `git_common_dir` when available so that
    /// worktrees of the current repo are recognized as "self" even
    /// when the registry holds a sibling checkout's path. Filtering
    /// on `path` alone would leak the current repo's other checkouts
    /// into cross-repo results, and their DBs may be pinned at a
    /// different commit (stale self-hits). Falls back to `path`
    /// equality when `primary_common_dir` is `None` (e.g. invocation
    /// outside a git repo or `git rev-parse` unavailable).
    #[must_use]
    pub fn other_repos(
        &self,
        primary_path: &Path,
        primary_common_dir: Option<&Path>,
    ) -> Vec<&RepoEntry> {
        self.repos
            .iter()
            .filter(|r| match primary_common_dir {
                Some(cd) => r.git_common_dir.as_path() != cd,
                None => r.path != primary_path,
            })
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

        let main_entry = make_entry("repo", main_path, common.clone(), "2026-01-01T00:00:00Z");
        reg.register(main_entry);

        let wt_entry = make_entry("repo-wt", wt_path.clone(), common, "2026-02-01T00:00:00Z");
        reg.register(wt_entry);

        // Latest invocation wins: path, name, and timestamp all
        // reflect the worktree that just registered. This keeps the
        // registry aligned with where the live DB actually is.
        assert_eq!(reg.repos.len(), 1);
        assert_eq!(reg.repos[0].path, wt_path);
        assert_eq!(reg.repos[0].name, "repo-wt");
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
    fn other_repos_excludes_primary_by_common_dir() {
        let dir = tempfile::tempdir().unwrap();
        let reg_path = dir.path().join("registry.toml");
        let path_a = dir.path().join("repo-a");
        let path_b = dir.path().join("repo-b");
        let common_a = dir.path().join(".git-a");
        let common_b = dir.path().join(".git-b");

        let mut reg = Registry::load(&reg_path).unwrap();

        reg.register(make_entry(
            "repo-a",
            path_a.clone(),
            common_a.clone(),
            "2026-01-01T00:00:00Z",
        ));
        reg.register(make_entry(
            "repo-b",
            path_b,
            common_b,
            "2026-01-01T00:00:00Z",
        ));

        let others = reg.other_repos(&path_a, Some(&common_a));
        assert_eq!(others.len(), 1);
        assert_eq!(others[0].name, "repo-b");
    }

    #[test]
    fn other_repos_falls_back_to_path_without_common_dir() {
        let dir = tempfile::tempdir().unwrap();
        let reg_path = dir.path().join("registry.toml");
        let path_a = dir.path().join("repo-a");
        let path_b = dir.path().join("repo-b");

        let mut reg = Registry::load(&reg_path).unwrap();

        reg.register(make_entry(
            "repo-a",
            path_a.clone(),
            dir.path().join(".git-a"),
            "2026-01-01T00:00:00Z",
        ));
        reg.register(make_entry(
            "repo-b",
            path_b,
            dir.path().join(".git-b"),
            "2026-01-01T00:00:00Z",
        ));

        let others = reg.other_repos(&path_a, None);
        assert_eq!(others.len(), 1);
        assert_eq!(others[0].name, "repo-b");
    }

    /// Regression test for `cross_query` returning stale self-hits.
    ///
    /// Scenario: the same logical repo is registered once (dedup
    /// collapses main + worktree into a single entry by `common_dir`).
    /// `other_repos` must exclude it regardless of which checkout's
    /// path is currently stored and regardless of which checkout's
    /// path the caller is invoking from. Keying on `git_common_dir`
    /// makes both directions work.
    #[test]
    fn other_repos_excludes_self_across_worktrees() {
        let dir = tempfile::tempdir().unwrap();
        let reg_path = dir.path().join("registry.toml");
        let common = dir.path().join(".git");
        let main_path = dir.path().join("main");
        let wt_path = dir.path().join("worktree");
        let sibling_common = dir.path().join(".git-sibling");

        let mut reg = Registry::load(&reg_path).unwrap();

        // Main checkout registers first, then a worktree of the same
        // repo registers. Dedup keeps one entry (path flips to the
        // worktree as the most recent). A completely unrelated repo
        // also sits in the registry.
        reg.register(make_entry(
            "repo",
            main_path,
            common.clone(),
            "2026-01-01T00:00:00Z",
        ));
        reg.register(make_entry(
            "repo-wt",
            wt_path,
            common.clone(),
            "2026-02-01T00:00:00Z",
        ));
        reg.register(make_entry(
            "sibling",
            dir.path().join("sibling"),
            sibling_common,
            "2026-01-01T00:00:00Z",
        ));

        assert_eq!(reg.repos.len(), 2);

        // Called from either checkout — both resolve to the same
        // common_dir via `git rev-parse --git-common-dir` — the self
        // repo is excluded and only the unrelated sibling remains.
        // The stored entry's `path` may be either checkout depending
        // on which registered most recently; common_dir is the
        // identity that matters.
        let main_invocation = dir.path().join("main");
        let wt_invocation = dir.path().join("worktree");

        let from_main = reg.other_repos(&main_invocation, Some(&common));
        assert_eq!(from_main.len(), 1);
        assert_eq!(from_main[0].name, "sibling");

        let from_wt = reg.other_repos(&wt_invocation, Some(&common));
        assert_eq!(from_wt.len(), 1);
        assert_eq!(from_wt[0].name, "sibling");
    }
}
