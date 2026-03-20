use crate::db::Database;
use std::fmt::Write;
use std::path::Path;
use std::process::Command;

pub fn handle_freshness(
    db: &Database,
    repo_path: &Path,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut output = String::new();
    let _ = writeln!(output, "## Index Freshness\n");

    let repo_str = repo_path.to_string_lossy();
    let indexed_hash = db.get_commit_hash(&repo_str)?;

    let head_output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_path)
        .output()?;
    let current_head = String::from_utf8_lossy(&head_output.stdout)
        .trim()
        .to_string();

    let indexed = indexed_hash.as_deref().unwrap_or("(none)");
    let _ = writeln!(output, "- **Indexed commit:** `{indexed}`");
    let _ = writeln!(output, "- **Current HEAD:** `{current_head}`");

    let is_current = indexed_hash.as_deref() == Some(current_head.as_str());
    let _ = writeln!(
        output,
        "- **Status:** {}",
        if is_current {
            "up to date"
        } else {
            "**STALE**"
        }
    );

    if !is_current {
        if let Some(hash) = &indexed_hash {
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
