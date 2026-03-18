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

/// Generate dependency docs using `cargo +nightly doc` JSON output.
/// Returns a list of `(dep_name, formatted_docs)` pairs.
pub fn generate_cargo_docs(
    repo_path: &Path,
    dep_names: &[String],
) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
    if dep_names.is_empty() {
        return Ok(Vec::new());
    }

    if !has_nightly_rustdoc() {
        tracing::info!("Nightly rustdoc not available, skipping cargo doc");
        return Ok(Vec::new());
    }

    // Run cargo doc with JSON output — documents project crate only
    // (--no-deps), but for deps we need to run without --no-deps
    tracing::info!("Running cargo +nightly doc for dependency docs");
    let status = std::process::Command::new("cargo")
        .args(["+nightly", "doc"])
        .env("RUSTDOCFLAGS", "-Z unstable-options --output-format json")
        .current_dir(repo_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .status()?;

    if !status.success() {
        return Err("cargo +nightly doc failed".into());
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

        let formatted = match parse_rustdoc_json(&content, dep_name) {
            Ok(doc) => doc,
            Err(e) => {
                tracing::warn!("Failed to parse rustdoc JSON for {dep_name}: {e}");
                continue;
            }
        };

        if !formatted.is_empty() {
            results.push((dep_name.clone(), formatted));
        }
    }

    tracing::info!(count = results.len(), "Generated cargo doc summaries");
    Ok(results)
}

/// Parse a rustdoc JSON file and produce a formatted API summary.
fn parse_rustdoc_json(
    json_str: &str,
    crate_name: &str,
) -> Result<String, Box<dyn std::error::Error>> {
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
    render_section(&mut output, "Traits", &items.traits);
    render_section(&mut output, "Structs", &items.structs);
    render_section(&mut output, "Enums", &items.enums);
    render_section(&mut output, "Functions", &items.functions);
    render_section(&mut output, "Macros", &items.macros);

    Ok(truncate_doc(&output, 8000))
}

struct CollectedItems<'a> {
    traits: Vec<ItemEntry<'a>>,
    structs: Vec<ItemEntry<'a>>,
    enums: Vec<ItemEntry<'a>>,
    functions: Vec<ItemEntry<'a>>,
    macros: Vec<ItemEntry<'a>>,
}

struct ItemEntry<'a> {
    name: &'a str,
    docs: &'a str,
    inner: &'a serde_json::Map<String, serde_json::Value>,
    kind: &'static str,
}

fn collect_public_items(index: &serde_json::Map<String, serde_json::Value>) -> CollectedItems<'_> {
    let mut items = CollectedItems {
        traits: Vec::new(),
        structs: Vec::new(),
        enums: Vec::new(),
        functions: Vec::new(),
        macros: Vec::new(),
    };

    for item in index.values() {
        let Some(name) = item.get("name").and_then(|n| n.as_str()) else {
            continue;
        };
        if item.get("visibility").and_then(|v| v.as_str()) != Some("public") {
            continue;
        }
        let Some(inner) = item.get("inner").and_then(|i| i.as_object()) else {
            continue;
        };
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
            continue;
        };
        let entry = ItemEntry {
            name,
            docs,
            inner,
            kind,
        };
        match kind {
            "trait_" => items.traits.push(entry),
            "struct" => items.structs.push(entry),
            "enum" => items.enums.push(entry),
            "function" => items.functions.push(entry),
            "macro" => items.macros.push(entry),
            _ => {}
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

/// Parse rustdoc JSON and return formatted API summary.
/// Exposed for integration testing.
pub fn parse_rustdoc_json_public(
    json_str: &str,
    crate_name: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    parse_rustdoc_json(json_str, crate_name)
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
