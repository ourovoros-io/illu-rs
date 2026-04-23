use crate::db::Database;
use std::fmt::Write;

pub fn handle_docs(
    db: &Database,
    dep_name: &str,
    topic: Option<&str>,
) -> Result<String, crate::IlluError> {
    if let Some(topic) = topic {
        return handle_docs_with_topic(db, dep_name, topic);
    }

    // No topic — return summary doc (module=""), list available modules
    let summary = db.get_doc_by_module(dep_name, "")?;
    if let Some(doc) = summary {
        let mut output = String::new();
        let _ = writeln!(
            output,
            "### {} v{} ({})\n\n{}\n",
            doc.dependency_name, doc.version, doc.source, doc.content
        );
        let modules = db.get_doc_modules(dep_name)?;
        if !modules.is_empty() {
            let _ = writeln!(
                output,
                "---\n**Available modules** \
                (use `topic` parameter to view):"
            );
            for m in &modules {
                let _ = writeln!(output, "- `{m}`");
            }
            let _ = writeln!(output);
        }
        return Ok(output);
    }

    // No summary doc — fall back to returning all docs
    let docs = db.get_docs_for_dependency(dep_name)?;
    if docs.is_empty() {
        let dep = db.get_dependency_by_name(dep_name)?;
        return match dep {
            Some(_) => Ok(format!(
                "'{dep_name}' is a known dependency but no docs were \
                 fetched. The crate may not be on docs.rs, or doc \
                 fetching may have been skipped."
            )),
            None => Ok(format!(
                "'{dep_name}' is not a known dependency of this project."
            )),
        };
    }

    let mut output = String::new();
    let _ = writeln!(output, "## Documentation: {dep_name}\n");
    for doc in &docs {
        let _ = writeln!(
            output,
            "### {} v{} ({})\n\n{}\n",
            doc.dependency_name, doc.version, doc.source, doc.content
        );
    }
    Ok(output)
}

fn handle_docs_with_topic(
    db: &Database,
    dep_name: &str,
    topic: &str,
) -> Result<String, crate::IlluError> {
    // Try exact module match first
    if let Some(doc) = db.get_doc_by_module(dep_name, topic)? {
        let mut output = String::new();
        let _ = writeln!(output, "## {dep_name}::{topic}\n");
        let _ = writeln!(output, "{}\n", doc.content);
        return Ok(output);
    }

    // Fall back to FTS search
    let results = db.search_docs(topic)?;
    let filtered: Vec<_> = results
        .iter()
        .filter(|d| d.dependency_name == dep_name)
        .collect();

    if filtered.is_empty() {
        // LIKE fallback for terms FTS can't tokenize (e.g. "FTS5")
        let like_results = db.search_docs_content(dep_name, topic)?;
        if !like_results.is_empty() {
            let mut output = String::new();
            let _ = writeln!(output, "## {dep_name} — {topic}\n");
            for doc in &like_results {
                let content = extract_topic_section(&doc.content, topic);
                let _ = writeln!(
                    output,
                    "### {} ({})\n\n{}\n",
                    doc.dependency_name, doc.source, content
                );
            }
            return Ok(output);
        }

        let dep = db.get_dependency_by_name(dep_name)?;
        let mut msg = match dep {
            Some(_) => format!(
                "'{dep_name}' is a known dependency but no docs match \
                 topic '{topic}'."
            ),
            None => {
                return Ok(format!(
                    "'{dep_name}' is not a known dependency of this project."
                ));
            }
        };
        let modules = db.get_doc_modules(dep_name)?;
        if modules.is_empty() {
            msg.push_str("\n\nNo module-level docs available for this dependency.");
        } else {
            msg.push_str("\n\nAvailable modules:");
            for m in &modules {
                let _ = write!(msg, "\n- `{m}`");
            }
        }
        if let Ok(Some(summary)) = db.get_doc_by_module(dep_name, "") {
            let excerpt: String = summary.content.chars().take(500).collect();
            let truncated = summary.content.chars().count() > 500;
            let _ = write!(
                msg,
                "\n\n**Crate summary** (use without topic for full docs):\n{}{}",
                excerpt,
                if truncated { "..." } else { "" }
            );
        }
        return Ok(msg);
    }

    let mut output = String::new();
    let _ = writeln!(output, "## {dep_name} — {topic}\n");
    for doc in &filtered {
        let content = extract_topic_section(&doc.content, topic);
        let _ = writeln!(
            output,
            "### {} ({})\n\n{}\n",
            doc.dependency_name, doc.source, content
        );
    }
    Ok(output)
}

/// Extract the relevant section around a topic from a large doc string.
/// Finds paragraphs containing the topic and returns surrounding context.
fn extract_topic_section(content: &str, topic: &str) -> String {
    let topic_lower = topic.to_lowercase();
    let lines: Vec<&str> = content.lines().collect();
    let mut relevant_ranges: Vec<(usize, usize)> = Vec::new();

    // Find lines containing the topic
    for (i, line) in lines.iter().enumerate() {
        if line.to_lowercase().contains(&topic_lower) {
            let start = i.saturating_sub(2);
            let end = (i + 5).min(lines.len());
            relevant_ranges.push((start, end));
        }
    }

    if relevant_ranges.is_empty() {
        // No specific match — return truncated content
        let truncated: String = content.chars().take(1000).collect();
        if content.len() > 1000 {
            return format!("{truncated}\n\n*(truncated — {topic} not found in specific section)*");
        }
        return truncated;
    }

    // Merge overlapping ranges
    relevant_ranges.sort_by_key(|r| r.0);
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for range in &relevant_ranges {
        if let Some(last) = merged.last_mut()
            && range.0 <= last.1 + 1
        {
            last.1 = last.1.max(range.1);
            continue;
        }
        merged.push(*range);
    }

    let mut result = String::new();
    for (i, (start, end)) in merged.iter().enumerate() {
        if i > 0 {
            result.push_str("\n...\n\n");
        }
        for line in &lines[*start..*end] {
            result.push_str(line);
            result.push('\n');
        }
    }

    if merged.len() < relevant_ranges.len() || merged.last().is_some_and(|r| r.1 < lines.len()) {
        let total = lines.len();
        let shown: usize = merged.iter().map(|(s, e)| e - s).sum();
        let _ = write!(
            result,
            "\n*(showing {shown} of {total} lines matching '{topic}')*"
        );
    }

    result
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn test_docs_for_dependency() {
        let db = Database::open_in_memory().unwrap();
        let dep_id = db
            .insert_dependency("serde", "1.0.210", true, None)
            .unwrap();
        db.store_doc(
            dep_id,
            "docs.rs",
            "Serde is a serialization framework for Rust",
        )
        .unwrap();

        let result = handle_docs(&db, "serde", None).unwrap();
        assert!(result.contains("serde"));
        assert!(result.contains("serialization"));
    }

    #[test]
    fn test_docs_with_topic() {
        let db = Database::open_in_memory().unwrap();
        let dep_id = db
            .insert_dependency("serde", "1.0.210", true, None)
            .unwrap();
        db.store_doc(dep_id, "docs.rs", "Serde serialization and deserialization")
            .unwrap();

        let result = handle_docs(&db, "serde", Some("serialization")).unwrap();
        assert!(result.contains("serialization"));
    }

    #[test]
    fn test_docs_unknown_dependency() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_docs(&db, "nonexistent", None).unwrap();
        assert!(result.contains("not a known dependency"));
    }

    #[test]
    fn test_docs_known_but_no_docs() {
        let db = Database::open_in_memory().unwrap();
        db.insert_dependency("obscure_crate", "0.1.0", true, None)
            .unwrap();
        let result = handle_docs(&db, "obscure_crate", None).unwrap();
        assert!(result.contains("known dependency"));
        assert!(result.contains("no docs were fetched"));
    }

    #[test]
    fn test_docs_topic_not_found() {
        let db = Database::open_in_memory().unwrap();
        let dep_id = db.insert_dependency("serde", "1.0", true, None).unwrap();
        db.store_doc(dep_id, "docs.rs", "Serde framework").unwrap();

        let result = handle_docs(&db, "serde", Some("graphql")).unwrap();
        assert!(result.contains("no docs match topic"));
    }

    #[test]
    fn test_docs_topic_not_found_shows_summary() {
        let db = Database::open_in_memory().unwrap();
        let dep_id = db
            .insert_dependency("mycrate", "1.0.0", false, None)
            .unwrap();
        db.store_doc(
            dep_id,
            "docs.rs",
            "A powerful crate for widget processing and data transformation.",
        )
        .unwrap();

        let result = handle_docs(&db, "mycrate", Some("nonexistent_topic")).unwrap();

        assert!(
            result.contains("nonexistent_topic"),
            "should mention the failed topic"
        );
        assert!(
            result.contains("widget processing"),
            "should show summary excerpt when topic not found"
        );
        assert!(
            result.contains("Crate summary"),
            "should have Crate summary header"
        );
    }

    #[test]
    fn test_docs_topic_like_fallback() {
        let db = Database::open_in_memory().unwrap();
        let dep_id = db
            .insert_dependency("rusqlite", "0.31.0", true, None)
            .unwrap();
        db.store_doc_with_module(
            dep_id,
            "docs.rs",
            "rusqlite supports FTS5 full-text search via virtual tables",
            "features",
        )
        .unwrap();

        // "FTS5" may not match via FTS tokenization,
        // but LIKE fallback should find it
        let result = handle_docs(&db, "rusqlite", Some("FTS5")).unwrap();
        assert!(
            result.contains("FTS5"),
            "LIKE fallback should find FTS5 in doc content\n{result}"
        );
    }

    #[test]
    fn test_docs_topic_like_case_insensitive() {
        let db = Database::open_in_memory().unwrap();
        let dep_id = db.insert_dependency("tokio", "1.0.0", true, None).unwrap();
        db.store_doc_with_module(
            dep_id,
            "docs.rs",
            "Tokio provides async Runtime for executing futures",
            "runtime",
        )
        .unwrap();

        // Search with lowercase — should find "Runtime" in uppercase
        let result = handle_docs(&db, "tokio", Some("runtime")).unwrap();
        assert!(
            result.contains("Runtime"),
            "case-insensitive LIKE should find Runtime via 'runtime'\n{result}"
        );
    }
}
