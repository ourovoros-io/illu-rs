use crate::db::{Database, StoredSymbol};
use crate::indexer::parser::SymbolKind;
use std::collections::BTreeMap;
use std::fmt::Write;

pub fn handle_type_usage(
    db: &Database,
    type_name: &str,
    path: Option<&str>,
    compact: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut sig_matches = db.search_symbols_by_signature(type_name)?;

    // Remove the type's own definition, use imports, and substring false positives
    sig_matches.retain(|s| {
        s.name != type_name
            && s.kind != SymbolKind::Use
            && contains_whole_word(&s.signature, type_name)
    });

    // Apply optional path filter
    if let Some(p) = path {
        sig_matches.retain(|s| s.file_path.starts_with(p));
    }

    // Split into "Returns" (type appears after ->) and "Accepts" (before -> or no ->)
    let mut returns = Vec::new();
    let mut accepts = Vec::new();
    for sym in &sig_matches {
        if let Some(arrow_pos) = sym.signature.find("->") {
            let after_arrow = &sym.signature[arrow_pos..];
            if after_arrow.contains(type_name) {
                returns.push(sym);
            } else {
                accepts.push(sym);
            }
        } else {
            accepts.push(sym);
        }
    }

    // Find structs whose field list (details) mentions the type
    let prefix = path.unwrap_or("");
    let mut field_matches = db.search_symbols_by_details(type_name, prefix)?;
    field_matches.retain(|s| {
        s.name != type_name
            && s.details
                .as_deref()
                .is_some_and(|d| contains_whole_word(d, type_name))
    });

    let mut output = String::new();
    let _ = writeln!(output, "## Type Usage: `{type_name}`\n");

    if returns.is_empty() && accepts.is_empty() && field_matches.is_empty() {
        let _ = writeln!(
            output,
            "No usage of `{type_name}` found in signatures or struct fields."
        );
        return Ok(output);
    }

    if compact {
        render_compact(&mut output, type_name, &returns, &accepts, &field_matches);
    } else {
        render_verbose(&mut output, type_name, &returns, &accepts, &field_matches);
    }

    Ok(output)
}

/// Check if `text` contains `word` as a whole word (not a substring of a longer identifier).
#[must_use]
pub fn contains_whole_word(text: &str, word: &str) -> bool {
    let mut start = 0;
    while let Some(pos) = text[start..].find(word) {
        let abs_pos = start + pos;
        let before_ok = abs_pos == 0
            || !text.as_bytes()[abs_pos - 1].is_ascii_alphanumeric()
                && text.as_bytes()[abs_pos - 1] != b'_';
        let after_pos = abs_pos + word.len();
        let after_ok = after_pos >= text.len()
            || !text.as_bytes()[after_pos].is_ascii_alphanumeric()
                && text.as_bytes()[after_pos] != b'_';
        if before_ok && after_ok {
            return true;
        }
        start = abs_pos + 1;
    }
    false
}

fn render_compact(
    output: &mut String,
    type_name: &str,
    returns: &[&StoredSymbol],
    accepts: &[&StoredSymbol],
    field_matches: &[StoredSymbol],
) {
    fn write_grouped(output: &mut String, syms_files: &[&str]) {
        let mut by_file: BTreeMap<&str, usize> = BTreeMap::new();
        for file in syms_files {
            *by_file.entry(file).or_default() += 1;
        }
        for (file, count) in &by_file {
            let _ = writeln!(output, "- **{file}** ({count})");
        }
        let _ = writeln!(output);
    }

    if !returns.is_empty() {
        let _ = writeln!(
            output,
            "### Returns `{type_name}` ({} sites)\n",
            returns.len()
        );
        let files: Vec<&str> = returns.iter().map(|s| s.file_path.as_str()).collect();
        write_grouped(output, &files);
    }

    if !accepts.is_empty() {
        let _ = writeln!(
            output,
            "### Accepts `{type_name}` ({} sites)\n",
            accepts.len()
        );
        let files: Vec<&str> = accepts.iter().map(|s| s.file_path.as_str()).collect();
        write_grouped(output, &files);
    }

    if !field_matches.is_empty() {
        let _ = writeln!(
            output,
            "### Contains `{type_name}` as field ({} sites)\n",
            field_matches.len()
        );
        let files: Vec<&str> = field_matches.iter().map(|s| s.file_path.as_str()).collect();
        write_grouped(output, &files);
    }
}

fn render_verbose(
    output: &mut String,
    type_name: &str,
    returns: &[&StoredSymbol],
    accepts: &[&StoredSymbol],
    field_matches: &[StoredSymbol],
) {
    if !returns.is_empty() {
        let _ = writeln!(output, "### Returns `{type_name}`\n");
        for sym in returns {
            let qualified = super::qualified_name(sym);
            let _ = writeln!(
                output,
                "- **{qualified}** ({}:{}) — `{}`",
                sym.file_path, sym.line_start, sym.signature
            );
        }
        let _ = writeln!(output);
    }

    if !accepts.is_empty() {
        let _ = writeln!(output, "### Accepts `{type_name}`\n");
        for sym in accepts {
            let qualified = super::qualified_name(sym);
            let _ = writeln!(
                output,
                "- **{qualified}** ({}:{}) — `{}`",
                sym.file_path, sym.line_start, sym.signature
            );
        }
        let _ = writeln!(output);
    }

    if !field_matches.is_empty() {
        let _ = writeln!(output, "### Contains `{type_name}` as field\n");
        for sym in field_matches {
            let qualified = super::qualified_name(sym);
            let _ = writeln!(
                output,
                "- **{qualified}** ({}:{})",
                sym.file_path, sym.line_start
            );
        }
        let _ = writeln!(output);
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::indexer::parser::{Symbol, SymbolKind, Visibility};
    use crate::indexer::store::store_symbols;

    fn setup_db() -> Database {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();

        let symbols = vec![
            // The type itself
            Symbol {
                name: "Config".into(),
                kind: SymbolKind::Struct,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub struct Config".into(),
                doc_comment: None,
                body: None,
                details: Some("path: String, debug: bool".into()),
                attributes: None,
                impl_type: None,
            },
            // Returns Config
            Symbol {
                name: "load_config".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 7,
                line_end: 12,
                signature: "pub fn load_config() -> Config".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            // Accepts Config
            Symbol {
                name: "use_config".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 14,
                line_end: 18,
                signature: "pub fn use_config(cfg: &Config)".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            // Struct containing Config as field
            Symbol {
                name: "AppState".into(),
                kind: SymbolKind::Struct,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 20,
                line_end: 24,
                signature: "pub struct AppState".into(),
                doc_comment: None,
                body: None,
                details: Some("config: Config, name: String".into()),
                attributes: None,
                impl_type: None,
            },
            // Unrelated function
            Symbol {
                name: "unrelated".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 26,
                line_end: 30,
                signature: "pub fn unrelated() -> bool".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
        ];
        store_symbols(&db, file_id, &symbols).unwrap();
        db
    }

    #[test]
    fn test_type_usage_in_signatures() {
        let db = setup_db();
        let result = handle_type_usage(&db, "Config", None, false).unwrap();

        assert!(result.contains("## Type Usage: `Config`"));
        assert!(result.contains("### Returns `Config`"));
        assert!(result.contains("load_config"));
        assert!(result.contains("### Accepts `Config`"));
        assert!(result.contains("use_config"));
        assert!(result.contains("### Contains `Config` as field"));
        assert!(result.contains("AppState"));
        assert!(!result.contains("unrelated"));
    }

    #[test]
    fn test_type_usage_no_results() {
        let db = setup_db();
        let result = handle_type_usage(&db, "Nonexistent", None, false).unwrap();

        assert!(result.contains("No usage of `Nonexistent` found"));
    }

    #[test]
    fn test_type_usage_path_filter() {
        let db = Database::open_in_memory().unwrap();
        let file1 = db.insert_file("src/lib.rs", "hash1").unwrap();
        let file2 = db.insert_file("src/server/mod.rs", "hash2").unwrap();

        let symbols1 = vec![Symbol {
            name: "use_config".into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            file_path: "src/lib.rs".into(),
            line_start: 1,
            line_end: 5,
            signature: "pub fn use_config(cfg: &Config)".into(),
            doc_comment: None,
            body: None,
            details: None,
            attributes: None,
            impl_type: None,
        }];
        let symbols2 = vec![Symbol {
            name: "serve_config".into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            file_path: "src/server/mod.rs".into(),
            line_start: 1,
            line_end: 5,
            signature: "pub fn serve_config(cfg: Config)".into(),
            doc_comment: None,
            body: None,
            details: None,
            attributes: None,
            impl_type: None,
        }];
        store_symbols(&db, file1, &symbols1).unwrap();
        store_symbols(&db, file2, &symbols2).unwrap();

        let result = handle_type_usage(&db, "Config", Some("src/server/"), false).unwrap();
        assert!(result.contains("serve_config"));
        assert!(!result.contains("use_config"));
    }

    #[test]
    fn test_type_usage_compact_mode() {
        let db = setup_db();
        let result = handle_type_usage(&db, "Config", None, true).unwrap();

        // Should have site counts in headers
        assert!(result.contains("(1 sites)"));
        // Should group by file
        assert!(result.contains("**src/lib.rs** (1)"));
        // Should NOT contain full signatures
        assert!(!result.contains("pub fn load_config"));
        assert!(!result.contains("pub fn use_config"));
    }
}
