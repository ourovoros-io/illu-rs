#[derive(Debug, Clone)]
pub enum DocSource {
    DocsRs,
    GitHubReadme,
}

impl std::fmt::Display for DocSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DocsRs => f.write_str("docs.rs"),
            Self::GitHubReadme => f.write_str("github_readme"),
        }
    }
}

#[derive(Debug)]
pub struct DocContent {
    pub source: DocSource,
    pub content: String,
    pub dependency_name: String,
    pub version: String,
}

#[must_use]
pub fn docs_rs_url(name: &str, version: &str) -> String {
    format!("https://docs.rs/{name}/{version}/{name}/")
}

#[must_use]
pub fn github_readme_url(repo_url: &str, version: &str) -> String {
    let repo_url = repo_url
        .trim_end_matches('/')
        .trim_end_matches(".git");
    let (owner, repo) = repo_url
        .rsplit_once("github.com/")
        .map_or(repo_url, |(_, path)| path)
        .split_once('/')
        .unwrap_or((repo_url, ""));
    format!(
        "https://raw.githubusercontent.com/\
         {owner}/{repo}/{version}/README.md"
    )
}

#[must_use]
pub fn parse_github_url(url: &str) -> Option<(String, String)> {
    let url = url
        .trim_end_matches('/')
        .trim_end_matches(".git");
    let path = url.rsplit_once("github.com/")?.1;
    let (owner, repo) = path.split_once('/')?;
    Some((owner.to_string(), repo.to_string()))
}

pub async fn fetch_docs_rs(
    name: &str,
    version: &str,
) -> Result<Option<String>, reqwest::Error> {
    let url = docs_rs_url(name, version);
    let client = reqwest::Client::builder()
        .user_agent("illu-rs/0.1.0")
        .build()?;
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

pub async fn fetch_github_readme(
    repo_url: &str,
    version: &str,
) -> Result<Option<String>, reqwest::Error> {
    let client = reqwest::Client::builder()
        .user_agent("illu-rs/0.1.0")
        .build()?;

    let tag_patterns =
        [format!("v{version}"), version.to_string()];
    for tag in &tag_patterns {
        let url = github_readme_url(repo_url, tag);
        let resp = client.get(&url).send().await?;
        if resp.status().is_success() {
            let text = resp.text().await?;
            if !text.is_empty() {
                return Ok(Some(text));
            }
        }
    }
    Ok(None)
}

fn extract_text_from_html(html: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;

    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if in_tag => {}
            _ => text.push(ch),
        }
    }

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
    result.trim().to_string()
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
    fn test_build_github_readme_url() {
        let url = github_readme_url(
            "https://github.com/serde-rs/serde",
            "v1.0.210",
        );
        assert_eq!(
            url,
            "https://raw.githubusercontent.com/\
             serde-rs/serde/v1.0.210/README.md"
        );
    }

    #[test]
    fn test_parse_github_repo_url() {
        let (owner, repo) =
            parse_github_url("https://github.com/serde-rs/serde")
                .unwrap();
        assert_eq!(owner, "serde-rs");
        assert_eq!(repo, "serde");
    }

    #[test]
    fn test_parse_github_url_with_trailing_slash() {
        let (owner, repo) = parse_github_url(
            "https://github.com/tokio-rs/tokio/",
        )
        .unwrap();
        assert_eq!(owner, "tokio-rs");
        assert_eq!(repo, "tokio");
    }

    #[test]
    fn test_parse_github_url_with_git_suffix() {
        let (owner, repo) = parse_github_url(
            "https://github.com/serde-rs/serde.git",
        )
        .unwrap();
        assert_eq!(owner, "serde-rs");
        assert_eq!(repo, "serde");
    }

    #[test]
    fn test_extract_text_from_html() {
        let html =
            "<html><body><h1>Hello</h1><p>World</p></body></html>";
        let text = extract_text_from_html(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
    }

    #[tokio::test]
    #[ignore = "hits network"]
    async fn test_fetch_docs_rs_content() {
        let content =
            fetch_docs_rs("serde", "1.0.210").await.unwrap();
        assert!(content.is_some());
        let text = content.unwrap();
        assert!(!text.is_empty());
    }

    #[tokio::test]
    #[ignore = "hits network"]
    async fn test_fetch_github_readme() {
        let content = fetch_github_readme(
            "https://github.com/serde-rs/serde",
            "1.0.210",
        )
        .await
        .unwrap();
        assert!(content.is_some());
    }
}
