#[must_use]
fn docs_rs_url(name: &str, version: &str) -> String {
    format!("https://docs.rs/{name}/{version}/{name}/")
}

async fn fetch_docs_rs(
    client: &reqwest::Client,
    name: &str,
    version: &str,
) -> Result<Option<String>, reqwest::Error> {
    let url = docs_rs_url(name, version);
    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let html = resp.text().await?;
    let text = extract_text_from_html(&html);
    if text.is_empty() {
        return Ok(None);
    }
    Ok(Some(text))
}

fn build_http_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .user_agent("illu-rs/0.1.0")
        .build()
}

pub struct PendingDoc {
    pub dep_id: crate::db::DepId,
    pub name: String,
    pub version: String,
}

/// Determine which dependencies need docs fetched (sync, needs DB).
pub fn pending_docs(
    db: &crate::db::Database,
) -> Result<Vec<PendingDoc>, Box<dyn std::error::Error>> {
    let deps = db.get_direct_dependencies()?;
    let mut pending = Vec::new();

    for dep in &deps {
        let existing = db.get_docs_for_dependency(&dep.name)?;
        if !existing.is_empty() {
            continue;
        }
        let Some(dep_id) = db.get_dependency_id(&dep.name)? else {
            continue;
        };
        pending.push(PendingDoc {
            dep_id,
            name: dep.name.clone(),
            version: dep.version.clone(),
        });
    }
    Ok(pending)
}

pub struct FetchedDoc {
    pub dep_id: crate::db::DepId,
    pub content: String,
}

/// Fetch docs over the network (async, no DB needed).
pub async fn fetch_docs(pending: &[PendingDoc]) -> Vec<FetchedDoc> {
    let client = match build_http_client() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to build HTTP client: {e}");
            return Vec::new();
        }
    };
    let mut results = Vec::new();
    for doc in pending {
        tracing::info!("Fetching docs for {} {}", doc.name, doc.version);
        match fetch_docs_rs(&client, &doc.name, &doc.version).await {
            Err(e) => {
                tracing::warn!("Failed to fetch docs for {} {}: {e}", doc.name, doc.version);
            }
            Ok(Some(content)) => {
                let truncated = if content.len() > 8000 {
                    format!("{}...", &content[..8000])
                } else {
                    content
                };
                results.push(FetchedDoc {
                    dep_id: doc.dep_id,
                    content: truncated,
                });
            }
            Ok(None) => {}
        }
    }
    tracing::info!("Fetched docs for {} dependencies", results.len());
    results
}

/// Store fetched docs into DB (sync, needs DB).
pub fn store_fetched_docs(
    db: &crate::db::Database,
    docs: &[FetchedDoc],
) -> Result<usize, Box<dyn std::error::Error>> {
    for doc in docs {
        db.store_doc(doc.dep_id, "docs.rs", &doc.content)?;
    }
    Ok(docs.len())
}

/// Convenience: fetch all missing docs in one call (async, holds DB reference).
pub async fn fetch_dependency_docs(
    db: &crate::db::Database,
) -> Result<usize, Box<dyn std::error::Error>> {
    let pending = pending_docs(db)?;
    if pending.is_empty() {
        return Ok(0);
    }
    let fetched = fetch_docs(&pending).await;
    store_fetched_docs(db, &fetched)
}

fn extract_text_from_html(html: &str) -> String {
    // Strip script and style blocks first
    let mut cleaned = html.to_string();
    for tag in &["script", "style", "head"] {
        let open = format!("<{tag}");
        let close = format!("</{tag}>");
        while let Some(start) = cleaned.find(&open) {
            let Some(end) = cleaned[start..].find(&close) else {
                break;
            };
            let end = start + end + close.len();
            cleaned.replace_range(start..end, " ");
        }
    }

    // Try to extract just the main content area
    let content = if let Some(start) = cleaned.find("Expand description") {
        &cleaned[start..]
    } else if let Some(start) = cleaned.find("<main") {
        &cleaned[start..]
    } else {
        &cleaned
    };

    // Strip remaining HTML tags
    let mut text = String::new();
    let mut in_tag = false;
    for ch in content.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                text.push(' ');
            }
            _ if in_tag => {}
            _ => text.push(ch),
        }
    }

    // Collapse whitespace
    let mut result = String::new();
    let mut last_was_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !last_was_space {
                result.push(' ');
                last_was_space = true;
            }
        } else {
            result.push(ch);
            last_was_space = false;
        }
    }

    // Decode common HTML entities
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .trim()
        .to_string()
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn test_build_docs_rs_url() {
        let url = docs_rs_url("serde", "1.0.210");
        assert_eq!(url, "https://docs.rs/serde/1.0.210/serde/");
    }

    #[test]
    fn test_extract_text_from_html() {
        let html = "<html><body><h1>Hello</h1><p>World</p></body></html>";
        let text = extract_text_from_html(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
    }

    #[tokio::test]
    #[ignore = "hits network"]
    async fn test_fetch_docs_rs_content() {
        let client = build_http_client().unwrap();
        let content = fetch_docs_rs(&client, "serde", "1.0.210").await.unwrap();
        assert!(content.is_some());
        let text = content.unwrap();
        assert!(!text.is_empty());
    }
}
