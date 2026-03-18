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
        .timeout(std::time::Duration::from_secs(15))
        .connect_timeout(std::time::Duration::from_secs(5))
        .build()
}

pub struct PendingDoc {
    pub dep_id: crate::db::DepId,
    pub name: String,
    pub version: String,
    pub repository_url: Option<String>,
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
            repository_url: dep.repository_url.clone(),
        });
    }
    Ok(pending)
}

pub struct FetchedDoc {
    pub dep_id: crate::db::DepId,
    pub source: &'static str,
    pub content: String,
}

/// Parse a GitHub/GitLab URL into (owner, repo).
fn parse_github_owner_repo(url: &str) -> Option<(&str, &str)> {
    // Handle: https://github.com/owner/repo[.git][/...]
    let path = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))?;
    let mut parts = path.splitn(3, '/');
    let owner = parts.next()?;
    let repo = parts.next()?.trim_end_matches(".git");
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner, repo))
}

/// Fetch the raw README from a GitHub repository.
/// Tries `main` branch first, then `master`.
async fn fetch_github_readme(client: &reqwest::Client, repo_url: &str) -> Option<String> {
    let (owner, repo) = parse_github_owner_repo(repo_url)?;
    for branch in &["main", "master"] {
        let url = format!("https://raw.githubusercontent.com/{owner}/{repo}/{branch}/README.md");
        let resp = match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => r,
            _ => continue,
        };
        let text = resp.text().await.ok()?;
        if !text.is_empty() {
            return Some(text);
        }
    }
    None
}

/// Query the crates.io API for a crate's repository URL.
async fn fetch_crates_io_repo_url(client: &reqwest::Client, name: &str) -> Option<String> {
    let url = format!("https://crates.io/api/v1/crates/{name}");
    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().await.ok()?;
    json.get("crate")?
        .get("repository")?
        .as_str()
        .map(String::from)
}

/// Try all doc sources for a single dependency.
/// Order: docs.rs → crates.io repo URL → GitHub README → stored repo URL.
async fn fetch_single_doc(
    client: &reqwest::Client,
    dep_id: crate::db::DepId,
    name: &str,
    version: &str,
    repository_url: Option<&str>,
) -> Option<FetchedDoc> {
    tracing::info!("Fetching docs for {name} {version}");

    // 1. Try docs.rs
    match fetch_docs_rs(client, name, version).await {
        Ok(Some(content)) => {
            return Some(FetchedDoc {
                dep_id,
                source: "docs.rs",
                content,
            });
        }
        Err(e) => {
            tracing::debug!("docs.rs failed for {name}: {e}");
        }
        Ok(None) => {
            tracing::debug!("docs.rs returned no content for {name}");
        }
    }

    // 2. Try discovering repo URL via crates.io, then fetch README
    let discovered_url = fetch_crates_io_repo_url(client, name).await;
    let repo_urls: Vec<&str> = discovered_url
        .as_deref()
        .into_iter()
        .chain(repository_url)
        .collect();

    for repo_url in repo_urls {
        tracing::debug!("Trying README from {repo_url} for {name}");
        if let Some(readme) = fetch_github_readme(client, repo_url).await {
            return Some(FetchedDoc {
                dep_id,
                source: "readme",
                content: readme,
            });
        }
    }

    tracing::info!("No docs found for {name} from any source");
    None
}

fn truncate_content(content: &str) -> String {
    crate::truncate_at(content, 8000).into_owned()
}

const MAX_CONCURRENT_FETCHES: usize = 8;

/// Try `cargo +nightly doc` JSON output first, then fall back to network.
/// Returns docs for as many pending deps as possible.
pub async fn fetch_docs(pending: &[PendingDoc], repo_path: &std::path::Path) -> Vec<FetchedDoc> {
    if pending.is_empty() {
        return Vec::new();
    }

    let mut results = Vec::new();

    // Phase 1: Try cargo doc (local, nightly, structured JSON)
    {
        let dep_names: Vec<String> = pending.iter().map(|p| p.name.clone()).collect();
        match super::cargo_doc::generate_cargo_docs(repo_path, &dep_names) {
            Ok(docs) => {
                for (name, content) in docs {
                    if let Some(p) = pending.iter().find(|p| p.name == name) {
                        results.push(FetchedDoc {
                            dep_id: p.dep_id,
                            source: "cargo_doc",
                            content: truncate_content(&content),
                        });
                    }
                }
                tracing::info!(count = results.len(), "Got docs from cargo doc");
            }
            Err(e) => {
                tracing::info!("cargo doc failed, falling back to network: {e}");
            }
        }
    }

    // Phase 2: Network fetch for any deps cargo doc missed
    let covered: std::collections::HashSet<&str> = results
        .iter()
        .filter_map(|r| pending.iter().find(|p| p.dep_id == r.dep_id))
        .map(|p| p.name.as_str())
        .collect();
    let remaining: Vec<&PendingDoc> = pending
        .iter()
        .filter(|p| !covered.contains(p.name.as_str()))
        .collect();

    if !remaining.is_empty() {
        tracing::info!(
            count = remaining.len(),
            "Fetching remaining docs from network"
        );
        let network_docs = fetch_docs_network(&remaining).await;
        results.extend(network_docs);
    }

    results
}

/// Fetch docs over the network (async, no DB needed).
/// Fetches up to `MAX_CONCURRENT_FETCHES` deps concurrently.
async fn fetch_docs_network(pending: &[&PendingDoc]) -> Vec<FetchedDoc> {
    let client = match build_http_client() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to build HTTP client: {e}");
            return Vec::new();
        }
    };
    let total = pending.len();
    tracing::info!(count = total, "Fetching dependency docs");
    crate::status::set(&format!("fetching docs ▸ 0/{total}"));

    let mut results = Vec::new();
    for chunk in pending.chunks(MAX_CONCURRENT_FETCHES) {
        let mut set = tokio::task::JoinSet::new();
        for doc in chunk {
            let client = client.clone();
            let dep_id = doc.dep_id;
            let name = doc.name.clone();
            let version = doc.version.clone();
            let repo_url = doc.repository_url.clone();
            set.spawn(async move {
                fetch_single_doc(&client, dep_id, &name, &version, repo_url.as_deref()).await
            });
        }
        while let Some(result) = set.join_next().await {
            if let Ok(Some(mut fetched)) = result {
                fetched.content = truncate_content(&fetched.content);
                results.push(fetched);
            }
        }
        crate::status::set(&format!("fetching docs ▸ {}/{total}", results.len()));
    }
    tracing::info!(fetched = results.len(), "Doc fetching complete");
    results
}

/// Store fetched docs into DB (sync, needs DB).
pub fn store_fetched_docs(
    db: &crate::db::Database,
    docs: &[FetchedDoc],
) -> Result<usize, Box<dyn std::error::Error>> {
    for doc in docs {
        db.store_doc(doc.dep_id, doc.source, &doc.content)?;
    }
    Ok(docs.len())
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

    #[test]
    fn test_parse_github_owner_repo() {
        let (owner, repo) = parse_github_owner_repo("https://github.com/serde-rs/serde").unwrap();
        assert_eq!(owner, "serde-rs");
        assert_eq!(repo, "serde");
    }

    #[test]
    fn test_parse_github_owner_repo_with_git_suffix() {
        let (owner, repo) = parse_github_owner_repo("https://github.com/user/repo.git").unwrap();
        assert_eq!(owner, "user");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_github_owner_repo_with_subpath() {
        let (owner, repo) =
            parse_github_owner_repo("https://github.com/user/repo/tree/main/subcrate").unwrap();
        assert_eq!(owner, "user");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_github_non_github_url() {
        assert!(parse_github_owner_repo("https://gitlab.com/user/repo").is_none());
    }

    #[test]
    fn test_truncate_content_short() {
        assert_eq!(truncate_content("hello world"), "hello world");
    }

    #[test]
    fn test_truncate_content_long() {
        let long = "a".repeat(9000);
        let result = truncate_content(&long);
        assert!(result.ends_with("..."));
        assert!(result.len() < 8010);
    }

    #[tokio::test]
    #[ignore = "hits network"]
    async fn test_fetch_github_readme() {
        let client = build_http_client().unwrap();
        let readme = fetch_github_readme(&client, "https://github.com/serde-rs/serde").await;
        assert!(readme.is_some());
        assert!(readme.unwrap().contains("serde"));
    }

    #[tokio::test]
    #[ignore = "hits network"]
    async fn test_fetch_crates_io_repo_url() {
        let client = build_http_client().unwrap();
        let url = fetch_crates_io_repo_url(&client, "serde").await;
        assert!(url.is_some());
        assert!(url.unwrap().contains("github.com"));
    }
}
