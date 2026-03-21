use crate::db::Database;
use std::fmt::Write;
use std::path::Path;

pub fn handle_history(
    db: &Database,
    repo_path: &Path,
    symbol_name: &str,
    max_commits: Option<i64>,
) -> Result<String, Box<dyn std::error::Error>> {
    let symbols = super::resolve_symbol(db, symbol_name)?;
    if symbols.is_empty() {
        return Ok(format!("Symbol '{symbol_name}' not found."));
    }

    let sym = &symbols[0];
    let qname = super::qualified_name(sym);
    let limit = max_commits.unwrap_or(10);

    let log_output = run_git_log(
        repo_path,
        &sym.file_path,
        sym.line_start,
        sym.line_end,
        limit,
    )?;

    let commits = parse_log_output(&log_output);
    let mut output = String::new();
    let _ = writeln!(output, "## History: {qname}\n");
    let _ = writeln!(
        output,
        "- **File:** {}:{}-{}",
        sym.file_path, sym.line_start, sym.line_end
    );

    if commits.is_empty() {
        let _ = writeln!(output, "\nNo git history found for this line range.");
        return Ok(output);
    }

    let _ = writeln!(output, "- **Commits:** {}\n", commits.len());

    for c in &commits {
        let short = &c.hash[..c.hash.len().min(7)];
        let _ = writeln!(output, "### {short} \u{2014} {}", c.subject);
        let _ = writeln!(output, "- **Author:** {}", c.author);
        let _ = writeln!(output, "- **Date:** {}", c.date);
        if !c.body.is_empty() {
            let _ = writeln!(output, "- **Details:** {}", c.body.trim());
        }
        let _ = writeln!(output);
    }

    Ok(output)
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
        let result = handle_history(&db, repo_path, "nonexistent", None).unwrap();
        assert!(result.contains("not found"));
    }
}
