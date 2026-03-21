use std::path::{Path, PathBuf};
use std::process::Command;

/// Detect the repo root from a directory using `git rev-parse --show-toplevel`.
/// Works for both regular repos and worktrees.
pub fn detect_repo_root(from: &Path) -> Result<PathBuf, String> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(from)
        .output()
        .map_err(|e| format!("failed to run git: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git rev-parse failed: {}", stderr.trim()));
    }

    let path = String::from_utf8_lossy(&output.stdout);
    Ok(PathBuf::from(path.trim()))
}

/// Get the shared git directory (for worktree dedup).
/// Returns the `.git` dir for regular repos, or the common dir for worktrees.
pub fn git_common_dir(from: &Path) -> Result<PathBuf, String> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .current_dir(from)
        .output()
        .map_err(|e| format!("failed to run git: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git rev-parse failed: {}", stderr.trim()));
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let raw = raw.trim();
    let path = Path::new(raw);

    // Output may be relative; resolve against `from`
    let absolute = if path.is_relative() {
        from.join(path)
    } else {
        path.to_path_buf()
    };

    absolute
        .canonicalize()
        .map_err(|e| format!("failed to canonicalize {}: {e}", absolute.display()))
}

/// Get the primary remote URL (for registry metadata).
#[must_use]
pub fn git_remote_url(from: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(from)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let url = String::from_utf8_lossy(&output.stdout);
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    Some(url.to_owned())
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn detect_repo_root_finds_this_repo() {
        let cwd = std::env::current_dir().unwrap();
        let root = detect_repo_root(&cwd).unwrap();
        assert!(root.join("Cargo.toml").exists());
    }

    #[test]
    fn git_common_dir_returns_valid_path() {
        let cwd = std::env::current_dir().unwrap();
        let common = git_common_dir(&cwd).unwrap();
        // Should be a .git directory (or contain HEAD for worktrees)
        assert!(common.exists());
    }

    #[test]
    fn git_remote_url_returns_some_for_this_repo() {
        let cwd = std::env::current_dir().unwrap();
        // This repo has an origin remote
        let url = git_remote_url(&cwd);
        assert!(url.is_some());
    }

    #[test]
    fn detect_repo_root_fails_for_non_repo() {
        let result = detect_repo_root(Path::new("/"));
        assert!(result.is_err());
    }
}
