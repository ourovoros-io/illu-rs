use crate::db::Database;
use std::fmt::Write;

pub fn handle_docs(
    db: &Database,
    dep_name: &str,
    topic: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
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
                 fetched. It may not be published on docs.rs."
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
) -> Result<String, Box<dyn std::error::Error>> {
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
        if !modules.is_empty() {
            msg.push_str("\n\nAvailable modules:");
            for m in &modules {
                let _ = write!(msg, "\n- `{m}`");
            }
        }
        return Ok(msg);
    }

    let mut output = String::new();
    let _ = writeln!(output, "## {dep_name} — {topic}\n");
    for doc in &filtered {
        let _ = writeln!(
            output,
            "### {} ({})\n\n{}\n",
            doc.dependency_name, doc.source, doc.content
        );
    }
    Ok(output)
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
}
