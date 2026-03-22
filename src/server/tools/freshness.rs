use crate::db::Database;
use std::fmt::Write;
use std::path::Path;
use std::process::Command;

/// Data extracted from the DB needed for freshness check.
pub struct FreshnessDbState {
    pub indexed_hash: Option<String>,
    pub stored_version: Option<String>,
}

/// Extract the stored index state from the database.
pub fn get_freshness_db_state(
    db: &Database,
    repo_path: &Path,
) -> Result<FreshnessDbState, Box<dyn std::error::Error>> {
    let repo_str = repo_path.to_string_lossy();
    let indexed_hash = db.commit_hash(&repo_str)?;
    let stored_version = db.index_version(&repo_str)?;
    Ok(FreshnessDbState {
        indexed_hash,
        stored_version,
    })
}

/// Format the freshness report using stored DB state and live git data.
/// Does not require DB access.
pub fn format_freshness_report(
    repo_path: &Path,
    state: &FreshnessDbState,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut output = String::new();
    let _ = writeln!(output, "## Index Freshness\n");

    let head_output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_path)
        .output()?;
    let current_head = String::from_utf8_lossy(&head_output.stdout)
        .trim()
        .to_string();

    let indexed = state
        .indexed_hash
        .as_deref()
        .unwrap_or("not indexed yet");
    let _ = writeln!(output, "- **Indexed commit:** `{indexed}`");
    let _ = writeln!(output, "- **Current HEAD:** `{current_head}`");

    let current_version = crate::indexer::INDEX_VERSION;
    let _ = writeln!(
        output,
        "- **Index version:** {}",
        state
            .stored_version
            .as_deref()
            .unwrap_or("unknown")
    );
    let _ = writeln!(output, "- **Binary version:** {current_version}");

    let version_current = state.stored_version.as_deref() == Some(current_version);
    let commit_current =
        state.indexed_hash.as_deref() == Some(current_head.as_str());
    let is_current = commit_current && version_current;

    if version_current {
        let _ = writeln!(
            output,
            "- **Status:** {}",
            if is_current {
                "up to date"
            } else {
                "**STALE**"
            }
        );
    } else {
        let _ = writeln!(
            output,
            "- **Status:** **STALE** (version mismatch — will re-index on next refresh)"
        );
    }

    if !commit_current {
        if let Some(hash) = &state.indexed_hash {
            let diff_output = Command::new("git")
                .args(["diff", "--name-only", hash, "HEAD"])
                .current_dir(repo_path)
                .output()?;
            let changed = String::from_utf8_lossy(&diff_output.stdout);
            let files: Vec<&str> = changed.lines().filter(|l| !l.is_empty()).collect();
            if !files.is_empty() {
                let _ = writeln!(
                    output,
                    "\n### Changed since index ({} files)\n",
                    files.len()
                );
                for f in &files {
                    let _ = writeln!(output, "- {f}");
                }
            }
        }

        let unstaged = Command::new("git")
            .args(["diff", "--name-only"])
            .current_dir(repo_path)
            .output()?;
        let unstaged_files = String::from_utf8_lossy(&unstaged.stdout);
        let ufiles: Vec<&str> = unstaged_files.lines().filter(|l| !l.is_empty()).collect();
        if !ufiles.is_empty() {
            let _ = writeln!(output, "\n### Unstaged changes ({} files)\n", ufiles.len());
            for f in &ufiles {
                let _ = writeln!(output, "- {f}");
            }
        }
    }

    Ok(output)
}

pub fn handle_freshness(
    db: &Database,
    repo_path: &Path,
) -> Result<String, Box<dyn std::error::Error>> {
    let state = get_freshness_db_state(db, repo_path)?;
    format_freshness_report(repo_path, &state)
}
