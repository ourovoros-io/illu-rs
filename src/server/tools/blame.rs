use crate::db::Database;
use super::SymbolTarget;
use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::Path;

/// Run git blame and format the output. Does not require DB access.
pub fn run_and_format_blame(
    repo_path: &Path,
    target: &SymbolTarget,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut output = String::new();
    let _ = writeln!(output, "## Blame: {}\n", target.qname);
    let _ = writeln!(
        output,
        "- **File:** {}:{}-{}",
        target.file_path, target.line_start, target.line_end
    );

    let blame_output = run_git_blame(
        repo_path,
        &target.file_path,
        target.line_start,
        target.line_end,
    )?;

    let entries = parse_blame_output(&blame_output);
    if entries.is_empty() {
        let _ = writeln!(output, "\nNo git blame data available.");
        return Ok(output);
    }

    let mut authors: BTreeMap<String, usize> = BTreeMap::new();
    let mut latest_date = String::new();
    let mut latest_author = String::new();
    let mut latest_commit = String::new();
    let mut latest_summary = String::new();

    for entry in &entries {
        *authors.entry(entry.author.clone()).or_default() += 1;
        if entry.date > latest_date {
            latest_date.clone_from(&entry.date);
            latest_author.clone_from(&entry.author);
            latest_commit.clone_from(&entry.commit_hash);
            latest_summary.clone_from(&entry.summary);
        }
    }

    let formatted_date = latest_date
        .parse::<i64>()
        .map(format_unix_timestamp)
        .unwrap_or(latest_date);
    let _ = writeln!(
        output,
        "- **Last modified:** {formatted_date} by {latest_author}"
    );
    let _ = writeln!(
        output,
        "- **Commit:** {} \u{2014} {latest_summary}",
        super::short_hash(&latest_commit)
    );

    let total_lines = entries.len();
    let _ = writeln!(output, "\n### Authors\n");
    for (author, count) in &authors {
        let pct = count * 100 / total_lines;
        let _ = writeln!(output, "- **{author}** \u{2014} {count} lines ({pct}%)");
    }

    Ok(output)
}

pub fn handle_blame(
    db: &Database,
    repo_path: &Path,
    symbol_name: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let target = match super::resolve_symbol_target(db, symbol_name)? {
        Ok(t) => t,
        Err(msg) => return Ok(msg),
    };
    run_and_format_blame(repo_path, &target)
}

struct BlameEntry {
    commit_hash: String,
    author: String,
    date: String,
    summary: String,
}

fn run_git_blame(
    repo_path: &Path,
    file_path: &str,
    line_start: i64,
    line_end: i64,
) -> Result<String, Box<dyn std::error::Error>> {
    let output = std::process::Command::new("git")
        .current_dir(repo_path)
        .args([
            "blame",
            "--porcelain",
            &format!("-L{line_start},{line_end}"),
            file_path,
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git blame failed: {stderr}").into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn parse_blame_output(output: &str) -> Vec<BlameEntry> {
    let mut entries = Vec::new();
    let mut current_hash = String::new();
    let mut current_author = String::new();
    let mut current_date = String::new();
    let mut current_summary = String::new();

    for line in output.lines() {
        if line.len() >= 40 && line.chars().take(40).all(|c| c.is_ascii_hexdigit()) {
            current_hash = line[..40].to_string();
        } else if let Some(author) = line.strip_prefix("author ") {
            current_author = author.to_string();
        } else if let Some(ts) = line.strip_prefix("author-time ") {
            current_date = ts.to_string();
        } else if let Some(summary) = line.strip_prefix("summary ") {
            current_summary = summary.to_string();
        } else if line.starts_with('\t') {
            entries.push(BlameEntry {
                commit_hash: current_hash.clone(),
                author: current_author.clone(),
                date: current_date.clone(),
                summary: current_summary.clone(),
            });
        }
    }

    entries
}

fn format_unix_timestamp(ts: i64) -> String {
    let secs_per_day: i64 = 86400;
    let days_since_epoch = ts / secs_per_day;
    let mut days = days_since_epoch;
    let mut year: i64 = 1970;
    loop {
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let days_in_year: i64 = if leap { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let months: [i64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month: u32 = 1;
    for &m in &months {
        if days < m {
            break;
        }
        days -= m;
        month += 1;
    }
    format!("{year}-{month:02}-{:02}", days + 1)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn test_parse_blame_output() {
        let sample = "\
abc123def456abc123def456abc123def456abc1 1 1 3
author John Doe
author-mail <john@example.com>
author-time 1711000000
author-tz +0000
committer John Doe
committer-mail <john@example.com>
committer-time 1711000000
committer-tz +0000
summary feat: add something
filename src/lib.rs
\tpub fn foo() {
abc123def456abc123def456abc123def456abc1 2 2
author John Doe
author-mail <john@example.com>
author-time 1711000000
author-tz +0000
committer John Doe
committer-mail <john@example.com>
committer-time 1711000000
committer-tz +0000
summary feat: add something
filename src/lib.rs
\t    42
abc123def456abc123def456abc123def456abc1 3 3
author Jane Smith
author-mail <jane@example.com>
author-time 1711100000
author-tz +0000
committer Jane Smith
committer-mail <jane@example.com>
committer-time 1711100000
committer-tz +0000
summary fix: something else
filename src/lib.rs
\t}";

        let entries = parse_blame_output(sample);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].author, "John Doe");
        assert_eq!(
            entries[0].commit_hash,
            "abc123def456abc123def456abc123def456abc1"
        );
        assert!(entries[0].summary.contains("feat: add something"));
        assert_eq!(entries[2].author, "Jane Smith");
        assert!(entries[2].summary.contains("fix: something else"));
    }

    #[test]
    fn test_parse_blame_output_empty() {
        let entries = parse_blame_output("");
        assert!(entries.is_empty());
    }

    #[test]
    fn test_blame_symbol_not_found() {
        let db = Database::open_in_memory().unwrap();
        let repo_path = Path::new("/tmp");
        let result = handle_blame(&db, repo_path, "nonexistent").unwrap();
        assert!(result.contains("No symbol found"));
    }

    #[test]
    fn test_format_unix_timestamp() {
        // 2024-03-21 (approx) = 1711000000
        let formatted = format_unix_timestamp(1_711_000_000);
        assert_eq!(formatted, "2024-03-21");
    }

    #[test]
    fn test_format_unix_timestamp_epoch() {
        let formatted = format_unix_timestamp(0);
        assert_eq!(formatted, "1970-01-01");
    }

    #[test]
    fn test_format_unix_timestamp_leap_year() {
        // 2024-02-29 = day 60 of 2024 (leap year)
        // 2024-01-01 00:00:00 UTC = 1704067200
        // Day 59 (Feb 29) = 1704067200 + 59 * 86400 = 1709164800
        let formatted = format_unix_timestamp(1_709_164_800);
        assert_eq!(formatted, "2024-02-29");
    }
}
