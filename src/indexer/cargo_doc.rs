use std::fmt::Write;
use std::path::Path;

/// Check whether `cargo +nightly` with rustdoc JSON output is available.
fn has_nightly_rustdoc() -> bool {
    std::process::Command::new("cargo")
        .args(["+nightly", "rustdoc", "--version"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Named docs for a single crate: (`crate_name`, per-module docs).
pub type CrateDocs = (String, Vec<ModuleDoc>);

/// Generate dependency docs using `cargo +nightly doc` JSON output.
/// Returns a list of `(dep_name, module_docs)` pairs.
pub fn generate_cargo_docs(
    repo_path: &Path,
    dep_names: &[String],
) -> Result<Vec<CrateDocs>, crate::IlluError> {
    if dep_names.is_empty() {
        return Ok(Vec::new());
    }

    if !has_nightly_rustdoc() {
        tracing::info!("Nightly rustdoc not available, skipping cargo doc");
        return Ok(Vec::new());
    }

    // Run cargo doc with JSON output — documents project crate only
    // (--no-deps), but for deps we need to run without --no-deps.
    // Timeout after 60s to avoid blocking on large builds.
    tracing::info!("Running cargo +nightly doc for dependency docs");
    let mut child = std::process::Command::new("cargo")
        .args(["+nightly", "doc"])
        .env("RUSTDOCFLAGS", "-Z unstable-options --output-format json")
        .current_dir(repo_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;

    let start = std::time::Instant::now();
    let deadline = start + std::time::Duration::from_mins(1);
    loop {
        match child.try_wait()? {
            Some(status) if status.success() => break,
            Some(_) => return Err("cargo +nightly doc failed".into()),
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("cargo +nightly doc timed out (>60s)".into());
            }
            None => {
                let elapsed = start.elapsed().as_secs();
                crate::status::set(&format!("fetching docs ▸ cargo doc ({elapsed}s)"));
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
    }

    let doc_dir = repo_path.join("target").join("doc");
    let mut results = Vec::new();

    for dep_name in dep_names {
        // rustdoc JSON uses hyphens converted to underscores
        let file_name = dep_name.replace('-', "_");
        let json_path = doc_dir.join(format!("{file_name}.json"));
        if !json_path.exists() {
            tracing::debug!("No JSON doc found for {dep_name}");
            continue;
        }

        let content = match std::fs::read_to_string(&json_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Failed to read {}: {e}", json_path.display());
                continue;
            }
        };

        let module_docs = match parse_rustdoc_json_modules(&content, dep_name) {
            Ok(docs) => docs,
            Err(e) => {
                tracing::warn!("Failed to parse rustdoc JSON for {dep_name}: {e}");
                continue;
            }
        };

        if !module_docs.is_empty() {
            results.push((dep_name.clone(), module_docs));
        }
    }

    tracing::info!(count = results.len(), "Generated cargo doc summaries");
    Ok(results)
}

/// Parse a rustdoc JSON file and produce a formatted API summary.
#[cfg(test)]
fn parse_rustdoc_json(json_str: &str, crate_name: &str) -> Result<String, crate::IlluError> {
    let doc: serde_json::Value = serde_json::from_str(json_str)?;
    let version = doc
        .get("crate_version")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let index = doc
        .get("index")
        .and_then(|v| v.as_object())
        .ok_or("missing index")?;

    let mut output = String::new();
    let _ = writeln!(output, "# {crate_name} {version}\n");

    // Extract crate-level docs from the root module
    if let Some(root_id) = doc.get("root") {
        let root_key = root_id.to_string().replace('"', "");
        if let Some(root_item) = index.get(&root_key)
            && let Some(docs) = root_item.get("docs").and_then(|d| d.as_str())
        {
            let truncated = truncate_doc(docs, 500);
            let _ = writeln!(output, "{truncated}\n");
        }
    }

    let items = collect_public_items(index);
    items.render(&mut output);

    Ok(truncate_doc(&output, 8000))
}

struct CollectedItems<'a> {
    traits: Vec<ItemEntry<'a>>,
    structs: Vec<ItemEntry<'a>>,
    enums: Vec<ItemEntry<'a>>,
    functions: Vec<ItemEntry<'a>>,
    macros: Vec<ItemEntry<'a>>,
}

impl<'a> CollectedItems<'a> {
    fn new() -> Self {
        Self {
            traits: Vec::new(),
            structs: Vec::new(),
            enums: Vec::new(),
            functions: Vec::new(),
            macros: Vec::new(),
        }
    }

    fn push(&mut self, entry: ItemEntry<'a>) {
        match entry.kind {
            "trait_" => self.traits.push(entry),
            "struct" => self.structs.push(entry),
            "enum" => self.enums.push(entry),
            "function" => self.functions.push(entry),
            "macro" => self.macros.push(entry),
            _ => {}
        }
    }

    fn render(&self, output: &mut String) {
        render_section(output, "Traits", &self.traits);
        render_section(output, "Structs", &self.structs);
        render_section(output, "Enums", &self.enums);
        render_section(output, "Functions", &self.functions);
        render_section(output, "Macros", &self.macros);
    }
}

struct ItemEntry<'a> {
    name: &'a str,
    docs: &'a str,
    inner: &'a serde_json::Map<String, serde_json::Value>,
    kind: &'static str,
}

#[cfg(test)]
fn collect_public_items(index: &serde_json::Map<String, serde_json::Value>) -> CollectedItems<'_> {
    let mut items = CollectedItems::new();
    for item in index.values() {
        let Some(inner) = item.get("inner").and_then(|i| i.as_object()) else {
            continue;
        };
        if let Some(entry) = classify_item(item, inner) {
            items.push(entry);
        }
    }
    items
}

fn render_section(output: &mut String, heading: &str, items: &[ItemEntry<'_>]) {
    if items.is_empty() {
        return;
    }
    let _ = writeln!(output, "## {heading}\n");
    for entry in items {
        match entry.kind {
            "trait_" => {
                let _ = write!(output, "- **{}**", entry.name);
                if let Some(count) = entry
                    .inner
                    .get("trait_")
                    .and_then(|t| t.get("items"))
                    .and_then(|i| i.as_array())
                    .map(Vec::len)
                {
                    let _ = write!(output, " ({count} items)");
                }
            }
            "struct" => {
                let _ = write!(output, "- **{}**", entry.name);
                write_generics(output, entry.inner, "struct");
            }
            "function" => {
                let sig = format_fn_signature(entry.name, entry.inner);
                let _ = write!(output, "- `{sig}`");
            }
            "macro" => {
                let _ = write!(output, "- **{}!**", entry.name);
            }
            _ => {
                let _ = write!(output, "- **{}**", entry.name);
            }
        }
        if !entry.docs.is_empty() {
            let first_line = first_doc_line(entry.docs);
            let _ = write!(output, " — {first_line}");
        }
        let _ = writeln!(output);
    }
    let _ = writeln!(output);
}

fn first_doc_line(docs: &str) -> &str {
    docs.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
}

fn truncate_doc(text: &str, max_len: usize) -> String {
    crate::truncate_at(text, max_len).into_owned()
}

fn write_generics(
    output: &mut String,
    inner: &serde_json::Map<String, serde_json::Value>,
    kind: &str,
) {
    let Some(kind_obj) = inner.get(kind) else {
        return;
    };
    let Some(generics) = kind_obj.get("generics") else {
        return;
    };
    let Some(params) = generics.get("params").and_then(|p| p.as_array()) else {
        return;
    };
    if params.is_empty() {
        return;
    }
    let names: Vec<&str> = params
        .iter()
        .filter_map(|p| p.get("name").and_then(|n| n.as_str()))
        .collect();
    if !names.is_empty() {
        let _ = write!(output, "<{}>", names.join(", "));
    }
}

fn format_fn_signature(name: &str, inner: &serde_json::Map<String, serde_json::Value>) -> String {
    let Some(func) = inner.get("function") else {
        return format!("fn {name}()");
    };
    let Some(sig) = func.get("sig") else {
        return format!("fn {name}()");
    };

    let mut params = Vec::new();
    if let Some(inputs) = sig.get("inputs").and_then(|i| i.as_array()) {
        for input in inputs {
            let Some(arr) = input.as_array() else {
                continue;
            };
            let param_name = arr.first().and_then(|n| n.as_str()).unwrap_or("_");
            params.push(param_name.to_string());
        }
    }

    let has_output = sig.get("output").is_some_and(|o| !o.is_null());

    if has_output {
        format!("fn {name}({}) -> ...", params.join(", "))
    } else {
        format!("fn {name}({})", params.join(", "))
    }
}

/// A single module's documentation content.
pub struct ModuleDoc {
    /// Module name ("" for summary/root).
    pub module: String,
    /// Rendered markdown content.
    pub content: String,
}

/// Collect items from a list of item IDs in the index.
fn collect_items_from_ids<'a>(
    item_ids: &[serde_json::Value],
    index: &'a serde_json::Map<String, serde_json::Value>,
) -> CollectedItems<'a> {
    let mut items = CollectedItems::new();
    for child_id in item_ids {
        let key = child_id.to_string().replace('"', "");
        let Some(child) = index.get(&key) else {
            continue;
        };
        let Some(inner) = child.get("inner").and_then(|i| i.as_object()) else {
            continue;
        };
        if let Some(entry) = classify_item(child, inner) {
            items.push(entry);
        }
    }
    items
}

/// Render collected items into a module doc string.
fn render_collected_items(
    heading: &str,
    docs: &str,
    items: &CollectedItems<'_>,
    max_len: usize,
) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "# {heading}\n");
    if !docs.is_empty() {
        let truncated = truncate_doc(docs, 500);
        let _ = writeln!(output, "{truncated}\n");
    }
    items.render(&mut output);
    truncate_doc(&output, max_len)
}

/// Check if an item is a non-crate submodule. Returns the module name.
fn as_submodule<'a>(
    item: &'a serde_json::Value,
    inner: &serde_json::Map<String, serde_json::Value>,
) -> Option<&'a str> {
    let module = inner.get("module")?;
    let is_crate = module
        .get("is_crate")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if is_crate {
        return None;
    }
    item.get("name").and_then(|n| n.as_str())
}

/// Parse rustdoc JSON into a summary plus per-module detail docs.
///
/// Returns: summary (module="") with module listing + top-level items,
/// plus one entry per submodule with its items.
pub fn parse_rustdoc_json_modules(
    json_str: &str,
    crate_name: &str,
) -> Result<Vec<ModuleDoc>, crate::IlluError> {
    let doc: serde_json::Value = serde_json::from_str(json_str)?;
    let version = doc
        .get("crate_version")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let index = doc
        .get("index")
        .and_then(|v| v.as_object())
        .ok_or("missing index")?;

    let root_id = doc
        .get("root")
        .ok_or("missing root")?
        .to_string()
        .replace('"', "");
    let root_item = index.get(&root_id).ok_or("root item not in index")?;
    let root_items = root_item
        .get("inner")
        .and_then(|i| i.get("module"))
        .and_then(|m| m.get("items"))
        .and_then(|i| i.as_array());
    let crate_docs = root_item.get("docs").and_then(|d| d.as_str()).unwrap_or("");

    // Separate root children into submodules vs top-level items
    let mut submodules: Vec<(String, String)> = Vec::new();
    let mut top_level_ids = Vec::new();

    if let Some(items) = root_items {
        for item_id in items {
            let key = item_id.to_string().replace('"', "");
            let Some(item) = index.get(&key) else {
                continue;
            };
            let Some(inner) = item.get("inner").and_then(|i| i.as_object()) else {
                continue;
            };
            if let Some(name) = as_submodule(item, inner) {
                submodules.push((name.to_string(), key));
            } else {
                top_level_ids.push(item_id.clone());
            }
        }
    }

    let top_level = collect_items_from_ids(&top_level_ids, index);
    let mut results = Vec::new();

    // Summary doc with module listing
    let mut summary = render_collected_items(
        &format!("{crate_name} {version}"),
        crate_docs,
        &top_level,
        4000,
    );
    if !submodules.is_empty() {
        // Insert module listing before the sections
        let mut mod_list = String::from("\n## Modules\n\n");
        for (name, _) in &submodules {
            let _ = writeln!(mod_list, "- **{name}**");
        }
        // Insert after the header + crate docs, before ## sections
        if let Some(pos) = summary.find("\n## ") {
            summary.insert_str(pos, &mod_list);
        } else {
            summary.push_str(&mod_list);
        }
        summary = truncate_doc(&summary, 4000);
    }
    results.push(ModuleDoc {
        module: String::new(),
        content: summary,
    });

    // Per-module docs
    for (mod_name, mod_key) in &submodules {
        let Some(mod_item) = index.get(mod_key) else {
            continue;
        };
        let mod_docs = mod_item.get("docs").and_then(|d| d.as_str()).unwrap_or("");
        let child_ids: Vec<serde_json::Value> = mod_item
            .get("inner")
            .and_then(|i| i.get("module"))
            .and_then(|m| m.get("items"))
            .and_then(|i| i.as_array())
            .cloned()
            .unwrap_or_default();
        let mod_items = collect_items_from_ids(&child_ids, index);
        let content = render_collected_items(
            &format!("{crate_name}::{mod_name}"),
            mod_docs,
            &mod_items,
            8000,
        );
        if content.lines().count() > 2 {
            results.push(ModuleDoc {
                module: mod_name.clone(),
                content,
            });
        }
    }

    Ok(results)
}

/// Classify a single item into an `ItemEntry` if it's a public,
/// recognized kind (trait, struct, enum, function, macro).
fn classify_item<'a>(
    item: &'a serde_json::Value,
    inner: &'a serde_json::Map<String, serde_json::Value>,
) -> Option<ItemEntry<'a>> {
    let name = item.get("name").and_then(|n| n.as_str())?;
    if item.get("visibility").and_then(|v| v.as_str()) != Some("public") {
        return None;
    }
    let docs = item.get("docs").and_then(|d| d.as_str()).unwrap_or("");

    let kind = if inner.contains_key("trait_") {
        "trait_"
    } else if inner.contains_key("struct") {
        "struct"
    } else if inner.contains_key("enum") {
        "enum"
    } else if inner.contains_key("function") {
        "function"
    } else if inner.contains_key("macro") {
        "macro"
    } else {
        return None;
    };

    Some(ItemEntry {
        name,
        docs,
        inner,
        kind,
    })
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn test_first_doc_line() {
        assert_eq!(first_doc_line("Hello world\nMore text"), "Hello world");
        assert_eq!(first_doc_line("\n\nActual line\n"), "Actual line");
        assert_eq!(first_doc_line(""), "");
    }

    #[test]
    fn test_truncate_doc() {
        let short = "hello";
        assert_eq!(truncate_doc(short, 100), "hello");
        let long = "a".repeat(200);
        let result = truncate_doc(&long, 100);
        assert!(result.len() <= 103); // 100 + "..."
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_format_fn_signature_no_func() {
        let inner = serde_json::Map::new();
        assert_eq!(format_fn_signature("test", &inner), "fn test()");
    }

    #[test]
    fn test_parse_minimal_rustdoc_json() {
        let json = serde_json::json!({
            "root": 1,
            "crate_version": "0.1.0",
            "index": {
                "1": {
                    "id": 1,
                    "crate_id": 0,
                    "name": "mylib",
                    "visibility": "public",
                    "docs": "A test library.",
                    "inner": {
                        "module": {
                            "is_crate": true,
                            "items": [2]
                        }
                    }
                },
                "2": {
                    "id": 2,
                    "crate_id": 0,
                    "name": "Config",
                    "visibility": "public",
                    "docs": "Application configuration.",
                    "inner": {
                        "struct": {
                            "kind": { "plain": { "fields": [], "has_stripped_fields": false } },
                            "generics": { "params": [], "where_predicates": [] },
                            "impls": []
                        }
                    }
                }
            },
            "paths": {},
            "external_crates": {},
            "format_version": 39
        });

        let result = parse_rustdoc_json(&json.to_string(), "mylib").unwrap();
        assert!(result.contains("# mylib 0.1.0"));
        assert!(result.contains("A test library."));
        assert!(result.contains("**Config**"));
        assert!(result.contains("Application configuration."));
    }

    #[test]
    fn test_parse_rustdoc_json_with_modules() {
        let json = serde_json::json!({
            "root": "0",
            "crate_version": "1.0.0",
            "index": {
                "0": {
                    "id": 0, "crate_id": 0, "name": "mylib",
                    "visibility": "public",
                    "docs": "A great library.",
                    "inner": {
                        "module": {
                            "is_crate": true,
                            "items": ["1", "2", "3"]
                        }
                    }
                },
                "1": {
                    "id": 1, "crate_id": 0, "name": "Config",
                    "visibility": "public",
                    "docs": "Top-level config.",
                    "inner": {
                        "struct": {
                            "kind": { "plain": { "fields": [], "has_stripped_fields": false } },
                            "generics": { "params": [], "where_predicates": [] },
                            "impls": []
                        }
                    }
                },
                "2": {
                    "id": 2, "crate_id": 0, "name": "net",
                    "visibility": "public",
                    "docs": "Networking utilities.",
                    "inner": {
                        "module": {
                            "is_crate": false,
                            "items": ["4"]
                        }
                    }
                },
                "3": {
                    "id": 3, "crate_id": 0, "name": "sync",
                    "visibility": "public",
                    "docs": "Synchronization primitives.",
                    "inner": {
                        "module": {
                            "is_crate": false,
                            "items": ["5"]
                        }
                    }
                },
                "4": {
                    "id": 4, "crate_id": 0, "name": "TcpListener",
                    "visibility": "public",
                    "docs": "A TCP listener.",
                    "inner": {
                        "struct": {
                            "kind": { "plain": { "fields": [], "has_stripped_fields": false } },
                            "generics": { "params": [], "where_predicates": [] },
                            "impls": []
                        }
                    }
                },
                "5": {
                    "id": 5, "crate_id": 0, "name": "Mutex",
                    "visibility": "public",
                    "docs": "An async mutex.",
                    "inner": {
                        "struct": {
                            "kind": { "plain": { "fields": [], "has_stripped_fields": false } },
                            "generics": { "params": [{"name": "T"}], "where_predicates": [] },
                            "impls": []
                        }
                    }
                }
            },
            "paths": {},
            "external_crates": {},
            "format_version": 39
        });

        let results = parse_rustdoc_json_modules(&json.to_string(), "mylib").unwrap();

        // Should have summary + 2 modules
        assert_eq!(results.len(), 3);

        // Summary (module="")
        let summary = &results[0];
        assert_eq!(summary.module, "");
        assert!(summary.content.contains("# mylib 1.0.0"));
        assert!(summary.content.contains("A great library."));
        assert!(summary.content.contains("## Modules"));
        assert!(summary.content.contains("**net**"));
        assert!(summary.content.contains("**sync**"));
        assert!(summary.content.contains("**Config**"));
        // Summary should NOT contain items from submodules
        assert!(!summary.content.contains("TcpListener"));
        assert!(!summary.content.contains("Mutex"));

        // Net module
        let net = results.iter().find(|m| m.module == "net").unwrap();
        assert!(net.content.contains("mylib::net"));
        assert!(net.content.contains("TcpListener"));
        assert!(net.content.contains("A TCP listener."));

        // Sync module
        let sync = results.iter().find(|m| m.module == "sync").unwrap();
        assert!(sync.content.contains("mylib::sync"));
        assert!(sync.content.contains("Mutex"));
        assert!(sync.content.contains("An async mutex."));
    }

    #[test]
    fn test_parse_rustdoc_with_traits_and_functions() {
        let json = serde_json::json!({
            "root": 1,
            "crate_version": "1.0.0",
            "index": {
                "1": {
                    "id": 1, "crate_id": 0, "name": "testcrate",
                    "visibility": "public", "docs": null,
                    "inner": { "module": { "is_crate": true, "items": [2, 3] } }
                },
                "2": {
                    "id": 2, "crate_id": 0, "name": "Serialize",
                    "visibility": "public",
                    "docs": "Serialize a value.",
                    "inner": {
                        "trait_": {
                            "items": [10, 11],
                            "generics": { "params": [], "where_predicates": [] },
                            "bounds": []
                        }
                    }
                },
                "3": {
                    "id": 3, "crate_id": 0, "name": "to_string",
                    "visibility": "public",
                    "docs": "Convert to string.",
                    "inner": {
                        "function": {
                            "sig": {
                                "inputs": [["value", {"generic": "T"}]],
                                "output": {"resolved_path": {"path": "String"}}
                            },
                            "generics": { "params": [], "where_predicates": [] }
                        }
                    }
                }
            },
            "paths": {},
            "external_crates": {},
            "format_version": 39
        });

        let result = parse_rustdoc_json(&json.to_string(), "testcrate").unwrap();
        assert!(result.contains("## Traits"));
        assert!(result.contains("**Serialize**"));
        assert!(result.contains("Serialize a value."));
        assert!(result.contains("## Functions"));
        assert!(result.contains("fn to_string(value)"));
        assert!(result.contains("Convert to string."));
    }
}
