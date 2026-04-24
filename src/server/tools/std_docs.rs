use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

const EXCERPT_LIMIT: usize = 4_000;
const TOPIC_CONTEXT: usize = 2_000;

static STD_DOC_INDEX: OnceLock<StdDocIndex> = OnceLock::new();

#[derive(Debug)]
struct StdDocIndex {
    rust_version: String,
    docs_root: PathBuf,
    items: BTreeMap<String, StdDocItem>,
}

#[derive(Debug, Clone)]
struct StdDocItem {
    key: String,
    href: String,
    path: PathBuf,
}

struct ResolvedStdDoc<'a> {
    item: &'a StdDocItem,
    method: Option<String>,
    requested: String,
}

pub fn handle_std_docs(item: &str, topic: Option<&str>) -> Result<String, crate::IlluError> {
    if let Some(index) = STD_DOC_INDEX.get() {
        return render_std_docs(index, item, topic);
    }

    let index = match build_std_doc_index() {
        Ok(index) => {
            // A lost race is harmless: all callers build the same deterministic
            // index from the same local rustdoc root, and the winner is reused.
            let _ = STD_DOC_INDEX.set(index);
            STD_DOC_INDEX.get().ok_or_else(|| {
                crate::IlluError::Other("std docs cache not initialised after set".to_string())
            })?
        }
        Err(error) => {
            return Ok(format!(
                "## Standard Library Docs Unavailable\n\n\
             {error}\n\n\
             Install local docs with `rustup component add rust-docs`, then try `std_docs` again."
            ));
        }
    };

    render_std_docs(index, item, topic)
}

fn build_std_doc_index() -> Result<StdDocIndex, crate::IlluError> {
    let docs_root = resolve_docs_root()?;
    let rust_version = rustc_version();
    build_std_doc_index_from_root(docs_root, rust_version)
}

fn resolve_docs_root() -> Result<PathBuf, crate::IlluError> {
    if let Ok(output) = Command::new("rustup")
        .args(["doc", "--std", "--path"])
        .output()
        && output.status.success()
    {
        let path = String::from_utf8(output.stdout)?;
        let index_path = PathBuf::from(path.trim());
        if index_path.exists()
            && let Some(parent) = index_path.parent()
        {
            return Ok(parent.to_path_buf());
        }
    }

    let output = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()?;
    if output.status.success() {
        let sysroot = String::from_utf8(output.stdout)?;
        let docs_root = PathBuf::from(sysroot.trim()).join("share/doc/rust/html/std");
        if docs_root.join("all.html").exists() {
            return Ok(docs_root);
        }
    }

    Err(crate::IlluError::Docs(
        "local standard-library rustdoc was not found".to_string(),
    ))
}

fn rustc_version() -> String {
    let output = Command::new("rustc").arg("--version").output();
    let Ok(output) = output else {
        return "rustc version unavailable".to_string();
    };
    if !output.status.success() {
        return "rustc version unavailable".to_string();
    }
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn build_std_doc_index_from_root(
    docs_root: PathBuf,
    rust_version: String,
) -> Result<StdDocIndex, crate::IlluError> {
    let all_path = docs_root.join("all.html");
    if !all_path.exists() {
        return Err(crate::IlluError::Docs(format!(
            "standard-library rustdoc index not found at {}",
            all_path.display()
        )));
    }
    let html = std::fs::read_to_string(&all_path)?;
    let items = parse_all_html(&html, &docs_root);
    if items.is_empty() {
        return Err(crate::IlluError::Docs(format!(
            "standard-library rustdoc index at {} did not contain any items",
            all_path.display()
        )));
    }
    Ok(StdDocIndex {
        rust_version,
        docs_root,
        items,
    })
}

fn parse_all_html(html: &str, docs_root: &Path) -> BTreeMap<String, StdDocItem> {
    let mut items = BTreeMap::new();
    let mut rest = html;
    let marker = "<a href=\"";

    while let Some(link_start) = rest.find(marker) {
        let href_start = link_start + marker.len();
        let after_href = &rest[href_start..];
        let Some(href_end) = after_href.find('"') else {
            break;
        };
        let href = &after_href[..href_end];
        let after_quote = &after_href[href_end..];
        let Some(tag_end) = after_quote.find('>') else {
            break;
        };
        let after_tag = &after_quote[tag_end + 1..];
        let Some(text_end) = after_tag.find("</a>") else {
            break;
        };
        let text = html_to_text(&after_tag[..text_end]);
        let key = normalize_item_key(text.trim());
        if !key.is_empty()
            && Path::new(href)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("html"))
        {
            items.entry(key.clone()).or_insert_with(|| StdDocItem {
                key,
                href: href.to_string(),
                path: docs_root.join(href),
            });
        }
        rest = &after_tag[text_end + "</a>".len()..];
    }

    items
}

fn render_std_docs(
    index: &StdDocIndex,
    item: &str,
    topic: Option<&str>,
) -> Result<String, crate::IlluError> {
    let Some(resolved) = resolve_item(index, item) else {
        let suggestions = suggestions(index, item);
        let mut output = format!(
            "## Standard Library Docs: `{item}`\n\nNo standard-library item matched `{item}`."
        );
        if !suggestions.is_empty() {
            output.push_str("\n\nDid you mean:");
            for suggestion in &suggestions {
                let _ = write!(output, "\n- `std::{suggestion}`");
            }
        }
        return Ok(output);
    };

    let html = std::fs::read_to_string(&resolved.item.path)?;
    let extracted = if let Some(method) = &resolved.method {
        extract_method_section(&html, method).unwrap_or_else(|| extract_item_section(&html))
    } else if let Some(topic) = topic {
        extract_method_section(&html, topic)
            .or_else(|| extract_topic_section(&html_to_text(&html), topic))
            .unwrap_or_else(|| extract_item_section(&html))
    } else {
        extract_item_section(&html)
    };
    let excerpt = clean_text(&html_to_text(&extracted));
    let excerpt = crate::truncate_at(excerpt.trim(), EXCERPT_LIMIT);

    let mut output = String::new();
    let _ = writeln!(
        output,
        "## Standard Library Docs: `{}`\n",
        resolved.requested
    );
    let _ = writeln!(output, "- **Rust version:** {}", index.rust_version);
    let _ = writeln!(output, "- **Resolved item:** `std::{}`", resolved.item.key);
    if let Some(method) = &resolved.method {
        let _ = writeln!(output, "- **Resolved section:** `::{method}`");
    }
    if let Some(topic) = topic {
        let _ = writeln!(output, "- **Topic:** `{topic}`");
    }
    let _ = writeln!(output, "- **Docs root:** {}", index.docs_root.display());
    let _ = writeln!(
        output,
        "- **Source rustdoc page:** {}",
        resolved.item.path.display()
    );
    let _ = writeln!(output, "- **Rustdoc href:** `{}`", resolved.item.href);
    let _ = writeln!(output, "\n### Relevant Excerpt\n");
    let _ = writeln!(output, "{excerpt}");
    Ok(output)
}

fn resolve_item<'a>(index: &'a StdDocIndex, item: &str) -> Option<ResolvedStdDoc<'a>> {
    let normalized = normalize_item_key(item);
    if let Some(found) = lookup_item(index, &normalized) {
        return Some(ResolvedStdDoc {
            item: found,
            method: None,
            requested: format!("std::{}", found.key),
        });
    }

    let (owner, method) = normalized.rsplit_once("::")?;
    let owner = lookup_item(index, owner)?;
    Some(ResolvedStdDoc {
        item: owner,
        method: Some(method.to_string()),
        requested: format!("std::{}::{method}", owner.key),
    })
}

fn lookup_item<'a>(index: &'a StdDocIndex, key: &str) -> Option<&'a StdDocItem> {
    if let Some(item) = index.items.get(key) {
        return Some(item);
    }
    let suffix = format!("::{key}");
    let mut matches = index
        .items
        .iter()
        .filter(|(candidate, _)| candidate.ends_with(&suffix));
    let first = matches.next().map(|(_, item)| item)?;
    if matches.next().is_none() {
        Some(first)
    } else {
        None
    }
}

fn suggestions(index: &StdDocIndex, item: &str) -> Vec<String> {
    let needle = normalize_item_key(item).to_ascii_lowercase();
    let leaf = needle.rsplit("::").next().unwrap_or(&needle);
    let mut matches: Vec<String> = index
        .items
        .keys()
        .filter(|key| key.to_ascii_lowercase().contains(leaf))
        .take(5)
        .cloned()
        .collect();
    matches.sort();
    matches
}

fn normalize_item_key(item: &str) -> String {
    item.trim()
        .trim_start_matches("std::")
        .trim_start_matches("core::")
        .trim_start_matches("alloc::")
        .replace("<wbr>", "")
}

fn extract_method_section(html: &str, method: &str) -> Option<String> {
    let anchor = format!("id=\"method.{method}\"");
    let anchor_pos = html.find(&anchor)?;
    let start = html[..anchor_pos].rfind("<details").unwrap_or(anchor_pos);
    let after_start = &html[start..];
    let end = after_start
        .find("</details>")
        .map_or(after_start.len().min(8_000), |pos| pos + "</details>".len());
    Some(after_start[..end].to_string())
}

fn extract_item_section(html: &str) -> String {
    let start = html
        .find("id=\"main-content\"")
        .or_else(|| html.find("<main"))
        .unwrap_or(0);
    let after_start = &html[start..];
    let end = [
        "<h2 id=\"implementations\"",
        "<h2 id=\"trait-implementations\"",
    ]
    .iter()
    .filter_map(|marker| after_start.find(marker))
    .min()
    .unwrap_or_else(|| after_start.len().min(12_000));
    after_start[..end].to_string()
}

fn extract_topic_section(text: &str, topic: &str) -> Option<String> {
    let haystack = text.to_ascii_lowercase();
    let needle = topic.to_ascii_lowercase();
    let pos = haystack.find(&needle)?;
    let start = text.floor_char_boundary(pos.saturating_sub(TOPIC_CONTEXT / 2));
    let end = text.ceil_char_boundary((pos + TOPIC_CONTEXT).min(text.len()));
    Some(text[start..end].to_string())
}

fn html_to_text(html: &str) -> String {
    let mut output = String::new();
    let mut chars = html.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '<' => {
                let mut tag = String::new();
                for tag_ch in chars.by_ref() {
                    if tag_ch == '>' {
                        break;
                    }
                    tag.push(tag_ch);
                }
                if is_block_tag(&tag) {
                    output.push('\n');
                }
            }
            '&' => {
                let mut entity = String::new();
                while let Some(next) = chars.peek().copied() {
                    if next == ';' || entity.len() > 12 {
                        break;
                    }
                    entity.push(next);
                    let _ = chars.next();
                }
                if chars.peek().is_some_and(|next| *next == ';') {
                    let _ = chars.next();
                }
                output.push_str(&decode_entity(&entity));
            }
            _ => output.push(ch),
        }
    }
    output
}

fn is_block_tag(tag: &str) -> bool {
    let tag = tag.trim_start_matches('/').trim_start();
    tag.starts_with('p')
        || tag.starts_with("br")
        || tag.starts_with("div")
        || tag.starts_with("pre")
        || tag.starts_with("li")
        || tag.starts_with('h')
        || tag.starts_with("section")
        || tag.starts_with("summary")
}

fn decode_entity(entity: &str) -> String {
    match entity {
        "amp" => "&".to_string(),
        "lt" => "<".to_string(),
        "gt" => ">".to_string(),
        "quot" => "\"".to_string(),
        "apos" | "#39" | "#x27" => "'".to_string(),
        "nbsp" => " ".to_string(),
        _ => {
            if let Some(decimal) = entity.strip_prefix('#')
                && let Ok(value) = decimal.parse::<u32>()
                && let Some(ch) = char::from_u32(value)
            {
                return ch.to_string();
            }
            if let Some(hex) = entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                && let Ok(value) = u32::from_str_radix(hex, 16)
                && let Some(ch) = char::from_u32(value)
            {
                return ch.to_string();
            }
            format!("&{entity};")
        }
    }
}

fn clean_text(text: &str) -> String {
    let mut cleaned = String::new();
    let mut previous_blank = false;
    for line in text.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            if !previous_blank && !cleaned.is_empty() {
                cleaned.push('\n');
                previous_blank = true;
            }
            continue;
        }
        if !cleaned.is_empty() && !cleaned.ends_with('\n') {
            cleaned.push('\n');
        }
        cleaned.push_str(&line);
        cleaned.push('\n');
        previous_blank = false;
    }
    cleaned
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    fn write_fake_std_docs(root: &Path) {
        std::fs::create_dir_all(root.join("collections")).unwrap();
        std::fs::create_dir_all(root.join("path")).unwrap();
        std::fs::write(
            root.join("all.html"),
            r#"<a href="collections/struct.HashMap.html">collections::HashMap</a>
<a href="path/struct.Path.html">path::Path</a>"#,
        )
        .unwrap();
        std::fs::write(
            root.join("collections/struct.HashMap.html"),
            r#"<main><section id="main-content"><h1>Struct HashMap</h1>
<pre>pub struct HashMap&lt;K, V&gt;</pre><div class="docblock"><p>A hash map.</p></div>
<h2 id="implementations">Methods</h2>
<details><summary><section id="method.iter" class="method"><h4>pub fn iter(&amp;self) -&gt; Iter&lt;'_, K, V&gt;</h4></section></summary>
<div class="docblock"><p>An iterator visiting all key-value pairs in arbitrary order.</p></div></details></section></main>"#,
        )
        .unwrap();
        std::fs::write(
            root.join("path/struct.Path.html"),
            r#"<main><section id="main-content"><h1>Struct Path</h1>
<div class="docblock"><p>A slice of a path.</p></div>
<h2 id="implementations">Methods</h2>
<details><summary><section id="method.strip_prefix" class="method"><h4>pub fn strip_prefix&lt;P&gt;(&amp;self, base: P) -&gt; Result&lt;&amp;Path, StripPrefixError&gt;</h4></section></summary>
<div class="docblock"><p>Returns a path that, when joined onto base, yields self.</p></div></details></section></main>"#,
        )
        .unwrap();
    }

    fn handle_with_fake_root(root: &Path, item: &str) -> String {
        let index =
            build_std_doc_index_from_root(root.to_path_buf(), "rustc test".to_string()).unwrap();
        render_std_docs(&index, item, None).unwrap()
    }

    #[test]
    fn resolves_hashmap() {
        let dir = tempfile::tempdir().unwrap();
        write_fake_std_docs(dir.path());
        let result = handle_with_fake_root(dir.path(), "std::collections::HashMap");
        assert!(result.contains("Resolved item"));
        assert!(result.contains("std::collections::HashMap"));
        assert!(result.contains("A hash map"));
    }

    #[test]
    fn resolves_hashmap_iter() {
        let dir = tempfile::tempdir().unwrap();
        write_fake_std_docs(dir.path());
        let result = handle_with_fake_root(dir.path(), "std::collections::HashMap::iter");
        assert!(result.contains("Resolved section"));
        assert!(result.contains("arbitrary order"));
    }

    #[test]
    fn topic_can_resolve_method_section() {
        let dir = tempfile::tempdir().unwrap();
        write_fake_std_docs(dir.path());
        let index =
            build_std_doc_index_from_root(dir.path().to_path_buf(), "rustc test".to_string())
                .unwrap();
        let result = render_std_docs(&index, "std::collections::HashMap", Some("iter")).unwrap();
        assert!(result.contains("Topic"));
        assert!(result.contains("arbitrary order"));
    }

    #[test]
    fn resolves_path_strip_prefix() {
        let dir = tempfile::tempdir().unwrap();
        write_fake_std_docs(dir.path());
        let result = handle_with_fake_root(dir.path(), "std::path::Path::strip_prefix");
        assert!(result.contains("strip_prefix"));
        assert!(result.contains("joined onto base"));
    }

    #[test]
    fn unknown_item_returns_suggestions() {
        let dir = tempfile::tempdir().unwrap();
        write_fake_std_docs(dir.path());
        let index =
            build_std_doc_index_from_root(dir.path().to_path_buf(), "rustc test".into()).unwrap();
        let result = render_std_docs(&index, "std::collections::HashMop", None).unwrap();
        assert!(result.contains("No standard-library item matched"));
    }

    #[test]
    fn missing_local_docs_reports_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let err = build_std_doc_index_from_root(dir.path().to_path_buf(), "rustc test".into())
            .unwrap_err();
        assert!(err.to_string().contains("rustdoc index not found"));
    }
}
