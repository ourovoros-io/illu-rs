use crate::db::Database;
use super::SymbolTarget;
use std::fmt::Write;
use std::path::Path;

const MAX_OUTPUT_CHARS: usize = 4000;
const MAX_DIFF_LINES_PER_COMMIT: usize = 60;

/// Run git log and format the output. Does not require DB access.
pub fn run_and_format_history(
    repo_path: &Path,
    target: &SymbolTarget,
    max_commits: Option<i64>,
    show_diff: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let limit = max_commits.unwrap_or(10);

    if show_diff {
        return format_diff_history(
            repo_path,
            &target.qname,
            &target.file_path,
            target.line_start,
            target.line_end,
            limit,
        );
    }

    let log_output = run_git_log(
        repo_path,
        &target.file_path,
        target.line_start,
        target.line_end,
        limit,
    )?;

    let commits = parse_log_output(&log_output);
    let mut output = String::new();
    let _ = writeln!(output, "## History: {}\n", target.qname);
    let _ = writeln!(
        output,
        "- **File:** {}:{}-{}",
        target.file_path, target.line_start, target.line_end
    );

    if commits.is_empty() {
        let _ = writeln!(output, "\nNo git history found for this line range.");
        return Ok(output);
    }

    let _ = writeln!(output, "- **Commits:** {}\n", commits.len());

    for c in &commits {
        let _ = writeln!(output, "### {} \u{2014} {}", super::short_hash(&c.hash), c.subject);
        let _ = writeln!(output, "- **Author:** {}", c.author);
        let _ = writeln!(output, "- **Date:** {}", c.date);
        if !c.body.is_empty() {
            let _ = writeln!(output, "- **Details:** {}", c.body.trim());
        }
        let _ = writeln!(output);
    }

    Ok(output)
}

pub fn handle_history(
    db: &Database,
    repo_path: &Path,
    symbol_name: &str,
    max_commits: Option<i64>,
    show_diff: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let target = match super::resolve_symbol_target(db, symbol_name)? {
        Ok(t) => t,
        Err(msg) => return Ok(msg),
    };
    run_and_format_history(repo_path, &target, max_commits, show_diff)
}

fn format_diff_history(
    repo_path: &Path,
    qname: &str,
    file_path: &str,
    line_start: i64,
    line_end: i64,
    limit: i64,
) -> Result<String, Box<dyn std::error::Error>> {
    let raw = run_git_log_with_diff(repo_path, file_path, line_start, line_end, limit)?;

    let commits = parse_diff_log_output(&raw);
    let total = commits.len();

    let mut output = String::new();
    let _ = writeln!(output, "## History (with diffs): {qname}\n");
    let _ = writeln!(output, "- **File:** {file_path}:{line_start}-{line_end}");

    if commits.is_empty() {
        let _ = writeln!(output, "\nNo git history found for this line range.");
        return Ok(output);
    }

    let _ = writeln!(output, "- **Commits:** {total}\n");

    for (shown, c) in commits.iter().enumerate() {
        let entry = format_diff_entry(c);
        if output.len() + entry.len() > MAX_OUTPUT_CHARS {
            let _ = writeln!(
                output,
                "\n(output truncated, {shown} commits shown of {total} total)"
            );
            break;
        }
        output.push_str(&entry);
    }

    Ok(output)
}

fn run_git_log_with_diff(
    repo_path: &Path,
    file: &str,
    line_start: i64,
    line_end: i64,
    limit: i64,
) -> Result<String, Box<dyn std::error::Error>> {
    let range = format!("{line_start},{line_end}");
    let output = std::process::Command::new("git")
        .current_dir(repo_path)
        .args([
            "log",
            &format!("-L{range}:{file}"),
            &format!("-n{limit}"),
            "--no-color",
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git log -L failed: {stderr}").into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

struct DiffLogEntry {
    hash: String,
    author: String,
    date: String,
    subject: String,
    diff: String,
}

fn parse_diff_log_output(output: &str) -> Vec<DiffLogEntry> {
    let mut entries = Vec::new();
    let mut current_lines: Vec<&str> = Vec::new();

    for line in output.lines() {
        if line.starts_with("commit ") && line.len() >= 10 && !current_lines.is_empty() {
            if let Some(entry) = build_diff_entry(&current_lines) {
                entries.push(entry);
            }
            current_lines.clear();
        }
        current_lines.push(line);
    }
    if !current_lines.is_empty()
        && let Some(entry) = build_diff_entry(&current_lines)
    {
        entries.push(entry);
    }

    entries
}

fn build_diff_entry(lines: &[&str]) -> Option<DiffLogEntry> {
    if lines.is_empty() {
        return None;
    }

    let hash = lines[0]
        .strip_prefix("commit ")
        .unwrap_or_default()
        .trim()
        .to_string();

    let mut author = String::new();
    let mut date = String::new();
    let mut subject = String::new();
    let mut diff_start = lines.len();
    let mut in_header = true;
    let mut found_blank = false;

    for (i, line) in lines.iter().enumerate().skip(1) {
        if in_header {
            if let Some(a) = line.strip_prefix("Author: ") {
                author = a.trim().to_string();
            } else if let Some(d) = line.strip_prefix("Date:   ") {
                date = d.trim().to_string();
            } else if line.is_empty() {
                if found_blank {
                    // Second blank = end of commit message
                    in_header = false;
                    diff_start = i + 1;
                }
                found_blank = true;
            } else if found_blank && subject.is_empty() {
                subject = line.trim().to_string();
            }
        }
        if line.starts_with("diff --git") {
            diff_start = i;
            break;
        }
    }

    let diff_lines: Vec<&str> = lines[diff_start..].to_vec();
    let diff = diff_lines.join("\n");

    if hash.is_empty() {
        return None;
    }

    Some(DiffLogEntry {
        hash,
        author,
        date,
        subject,
        diff,
    })
}

fn format_diff_entry(entry: &DiffLogEntry) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "### {} \u{2014} {}", super::short_hash(&entry.hash), entry.subject);
    let _ = writeln!(out, "- **Author:** {}", entry.author);
    let _ = writeln!(out, "- **Date:** {}", entry.date);

    if !entry.diff.is_empty() {
        // Keep only +/- lines and @@ headers, cap at limit
        let relevant: Vec<&str> = entry
            .diff
            .lines()
            .filter(|l| l.starts_with('+') || l.starts_with('-') || l.starts_with("@@"))
            .take(MAX_DIFF_LINES_PER_COMMIT)
            .collect();

        if !relevant.is_empty() {
            let _ = writeln!(out, "\n```diff");
            for line in &relevant {
                let _ = writeln!(out, "{line}");
            }
            let _ = writeln!(out, "```");
        }
    }
    let _ = writeln!(out);
    out
}

struct LogEntry {
    hash: String,
    author: String,
    date: String,
    subject: String,
    body: String,
}

fn run_git_log(
    repo_path: &Path,
    file_path: &str,
    line_start: i64,
    line_end: i64,
    limit: i64,
) -> Result<String, Box<dyn std::error::Error>> {
    let output = std::process::Command::new("git")
        .current_dir(repo_path)
        .args([
            "log",
            &format!("-{limit}"),
            "--format=%H%n%an%n%aI%n%s%n%b%x00",
            &format!("-L{line_start},{line_end}:{file_path}"),
            "--no-patch",
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git log failed: {stderr}").into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn parse_log_output(output: &str) -> Vec<LogEntry> {
    let mut entries = Vec::new();

    for block in output.split('\0') {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }
        let mut lines = block.lines();
        let hash = lines.next().unwrap_or_default().to_string();
        let author = lines.next().unwrap_or_default().to_string();
        let date = lines.next().unwrap_or_default().to_string();
        let subject = lines.next().unwrap_or_default().to_string();
        let body: String = lines.collect::<Vec<_>>().join("\n");

        if !hash.is_empty() {
            entries.push(LogEntry {
                hash,
                author,
                date,
                subject,
                body,
            });
        }
    }

    entries
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn test_parse_log_output() {
        let sample = "abc123def456abc123def456abc123def456abc1\n\
                       John Doe\n\
                       2024-03-21T10:00:00+00:00\n\
                       feat: add new feature\n\
                       Extended description here\0\
                       def456abc123def456abc123def456abc123def4\n\
                       Jane Smith\n\
                       2024-03-20T09:00:00+00:00\n\
                       fix: correct bug\n\
                       \0";
        let entries = parse_log_output(sample);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].author, "John Doe");
        assert!(entries[0].subject.contains("feat: add new feature"));
        assert!(entries[0].body.contains("Extended description"));
        assert_eq!(entries[1].author, "Jane Smith");
        assert!(entries[1].subject.contains("fix: correct bug"));
    }

    #[test]
    fn test_parse_log_output_empty() {
        let entries = parse_log_output("");
        assert!(entries.is_empty());
    }

    #[test]
    fn test_history_symbol_not_found() {
        let db = Database::open_in_memory().unwrap();
        let repo_path = Path::new("/tmp");
        let result = handle_history(&db, repo_path, "nonexistent", None, false).unwrap();
        assert!(result.contains("No symbol found"));
    }

    #[test]
    fn test_parse_diff_log_output() {
        let sample = "\
commit abc123def456abc123def456abc123def456abc1
Author: Alice <alice@example.com>
Date:   2024-01-15 10:00:00 +0000

    Fix the parsing bug

diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,5 +10,6 @@
-    old_line();
+    new_line();
+    extra_line();

commit def456abc123def456abc123def456abc123def4
Author: Bob <bob@example.com>
Date:   2024-01-10 09:00:00 +0000

    Initial implementation

diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,0 +1,5 @@
+fn something() {
+    old_line();
+}
";
        let entries = parse_diff_log_output(sample);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].author, "Alice <alice@example.com>");
        assert_eq!(entries[0].subject, "Fix the parsing bug");
        assert!(entries[0].diff.contains("-    old_line();"));
        assert!(entries[0].diff.contains("+    new_line();"));
        assert_eq!(entries[1].author, "Bob <bob@example.com>");
        assert_eq!(entries[1].subject, "Initial implementation");
    }

    #[test]
    fn test_parse_diff_log_output_empty() {
        let entries = parse_diff_log_output("");
        assert!(entries.is_empty());
    }

    #[test]
    fn test_format_diff_entry_caps_lines() {
        let entry = DiffLogEntry {
            hash: "abc123def456abc123def456abc123def456abc1".to_string(),
            author: "Test".to_string(),
            date: "2024-01-01".to_string(),
            subject: "test commit".to_string(),
            diff: String::new(),
        };
        let formatted = format_diff_entry(&entry);
        assert!(formatted.contains("abc123d"));
        assert!(formatted.contains("test commit"));
        // No diff block when diff is empty
        assert!(!formatted.contains("```diff"));
    }
}
