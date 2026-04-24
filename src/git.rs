#![allow(dead_code, missing_docs, unreachable_pub)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// Run a `git` subcommand in `repo_path`, returning its stdout on success.
///
/// On non-zero exit, returns `"git {args[0]} failed: {stderr}"`. Stdout is
/// decoded via `from_utf8_lossy` because git porcelain output (blame, log -L)
/// can contain source bytes that are not valid UTF-8.
pub(crate) fn run_git(repo_path: &Path, args: &[&str]) -> Result<String, crate::IlluError> {
    let output = Command::new("git")
        .current_dir(repo_path)
        .args(args)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let subcommand = args.first().copied().unwrap_or("?");
        return Err(crate::IlluError::Git(format!(
            "{subcommand} failed: {stderr}"
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

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

/// Detect the Rust project root by finding the nearest `Cargo.toml` ancestor.
///
/// Walks up from `from` (inclusive) toward `git_root`. If the git root
/// itself has a `Cargo.toml`, returns the git root. Otherwise, returns
/// the nearest ancestor of `from` that contains a `Cargo.toml`.
/// Falls back to the git root if no `Cargo.toml` is found.
///
/// This handles Rust projects nested inside larger git repos (e.g.
/// monorepos with `tools/my-rust-project/`).
#[must_use]
pub fn detect_cargo_root(from: &Path, git_root: &Path) -> PathBuf {
    // If git root has Cargo.toml, prefer it (standard layout)
    if git_root.join("Cargo.toml").exists() {
        return git_root.to_path_buf();
    }

    // Walk up from CWD looking for Cargo.toml
    let mut dir = from.to_path_buf();
    loop {
        if dir.join("Cargo.toml").exists() {
            return dir;
        }
        if dir == git_root || !dir.starts_with(git_root) {
            break;
        }
        if !dir.pop() {
            break;
        }
    }

    // Fallback to git root
    git_root.to_path_buf()
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

    #[test]
    fn detect_cargo_root_prefers_git_root_with_cargo_toml() {
        let cwd = std::env::current_dir().unwrap();
        let git_root = detect_repo_root(&cwd).unwrap();
        // This repo has Cargo.toml at git root, so it should return git root
        let result = detect_cargo_root(&cwd, &git_root);
        assert_eq!(result, git_root);
    }

    #[test]
    fn detect_cargo_root_finds_nested_cargo_toml() {
        let cwd = std::env::current_dir().unwrap();
        let git_root = detect_repo_root(&cwd).unwrap();
        // Simulate: CWD=git_root/src (no Cargo.toml), git_root has Cargo.toml
        // Should still find git_root since it has Cargo.toml
        let nested = git_root.join("src");
        let result = detect_cargo_root(&nested, &git_root);
        assert_eq!(result, git_root);
    }

    #[test]
    fn detect_cargo_root_falls_back_to_git_root() {
        // When no Cargo.toml exists anywhere, falls back to git root
        let fake_root = Path::new("/tmp/fake-git-root");
        let fake_cwd = Path::new("/tmp/fake-git-root/sub/dir");
        let result = detect_cargo_root(fake_cwd, fake_root);
        assert_eq!(result, fake_root);
    }
}
