use crate::db::Database;
use crate::indexer::parser::SymbolKind;
use std::collections::BTreeMap;
use std::fmt::Write;

pub fn handle_boundary(db: &Database, path: &str) -> Result<String, Box<dyn std::error::Error>> {
    let symbols = db.get_symbols_by_path_prefix_filtered(path, false)?;
    if symbols.is_empty() {
        return Ok(format!("No public symbols found under '{path}'."));
    }

    let mut output = String::new();
    let _ = writeln!(output, "## Module Boundary: {path}\n");

    let mut external_api: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let mut internal_only: Vec<(String, String)> = Vec::new();

    for sym in &symbols {
        if sym.kind == SymbolKind::Use
            || sym.kind == SymbolKind::Mod
            || sym.kind == SymbolKind::EnumVariant
            || sym.kind == SymbolKind::Impl
        {
            continue;
        }

        let callers = db.get_callers(&sym.name, &sym.file_path, false, None)?;
        let mut ext_files: Vec<&str> = callers
            .iter()
            .filter(|c| !c.file_path.starts_with(path))
            .map(|c| c.file_path.as_str())
            .collect();

        let qname = super::qualified_name(sym);
        if ext_files.is_empty() {
            internal_only.push((qname, sym.file_path.clone()));
        } else {
            let site_count = ext_files.len();
            ext_files.sort_unstable();
            ext_files.dedup();
            let summary = if ext_files.len() <= 3 {
                format!("{site_count} site(s) in {}", ext_files.join(", "))
            } else {
                format!(
                    "{site_count} site(s) across {} files ({}, ...)",
                    ext_files.len(),
                    ext_files[..3].join(", ")
                )
            };
            external_api
                .entry(sym.file_path.clone())
                .or_default()
                .push((qname, summary));
        }
    }

    if !external_api.is_empty() {
        let total: usize = external_api.values().map(Vec::len).sum();
        let _ = writeln!(output, "### Public API ({total} symbols used externally)\n");
        for (file, syms) in &external_api {
            let _ = writeln!(output, "#### {file}\n");
            for (name, summary) in syms {
                let _ = writeln!(output, "- **{name}** — {summary}");
            }
            let _ = writeln!(output);
        }
    }

    if !internal_only.is_empty() {
        let _ = writeln!(
            output,
            "### Internal Only ({} symbols, safe to refactor)\n",
            internal_only.len()
        );
        let mut current_file = String::new();
        for (name, file) in &internal_only {
            if *file != current_file {
                current_file.clone_from(file);
                let _ = writeln!(output, "#### {current_file}\n");
            }
            let _ = writeln!(output, "- {name}");
        }
        let _ = writeln!(output);
    }

    if external_api.is_empty() && internal_only.is_empty() {
        let _ = writeln!(output, "No analyzable symbols found.");
    }

    Ok(output)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::indexer::parser::{Symbol, SymbolKind, Visibility};
    use crate::indexer::store::store_symbols;

    fn setup_db() -> Database {
        let db = Database::open_in_memory().unwrap();

        let mod_file_id = db.insert_file("src/mod/lib.rs", "hash1").unwrap();
        let mod_symbols = vec![
            Symbol {
                name: "public_fn".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/mod/lib.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub fn public_fn() -> bool".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "internal_fn".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/mod/lib.rs".into(),
                line_start: 7,
                line_end: 12,
                signature: "pub fn internal_fn() -> bool".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
        ];
        store_symbols(&db, mod_file_id, &mod_symbols).unwrap();

        let other_file_id = db.insert_file("src/other/main.rs", "hash2").unwrap();
        let other_symbols = vec![Symbol {
            name: "caller".into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            file_path: "src/other/main.rs".into(),
            line_start: 1,
            line_end: 5,
            signature: "pub fn caller()".into(),
            doc_comment: None,
            body: None,
            details: None,
            attributes: None,
            impl_type: None,
        }];
        store_symbols(&db, other_file_id, &other_symbols).unwrap();

        let internal_caller_id = db.insert_file("src/mod/helper.rs", "hash3").unwrap();
        let internal_symbols = vec![Symbol {
            name: "helper".into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            file_path: "src/mod/helper.rs".into(),
            line_start: 1,
            line_end: 5,
            signature: "pub fn helper()".into(),
            doc_comment: None,
            body: None,
            details: None,
            attributes: None,
            impl_type: None,
        }];
        store_symbols(&db, internal_caller_id, &internal_symbols).unwrap();

        // External caller -> public_fn
        let caller_id = db
            .get_symbol_id("caller", "src/other/main.rs")
            .unwrap()
            .unwrap();
        let public_fn_id = db
            .get_symbol_id("public_fn", "src/mod/lib.rs")
            .unwrap()
            .unwrap();
        db.insert_symbol_ref(caller_id, public_fn_id, "call", "high", None)
            .unwrap();

        // Internal caller -> internal_fn
        let helper_id = db
            .get_symbol_id("helper", "src/mod/helper.rs")
            .unwrap()
            .unwrap();
        let internal_fn_id = db
            .get_symbol_id("internal_fn", "src/mod/lib.rs")
            .unwrap()
            .unwrap();
        db.insert_symbol_ref(helper_id, internal_fn_id, "call", "high", None)
            .unwrap();

        db
    }

    #[test]
    fn test_boundary_external_usage() {
        let db = setup_db();
        let result = handle_boundary(&db, "src/mod/").unwrap();

        assert!(result.contains("Public API"));
        assert!(result.contains("public_fn"));
        assert!(
            result.contains("1 site(s) in src/other/main.rs"),
            "should show summarized usage: {result}"
        );
    }

    #[test]
    fn test_boundary_internal_only() {
        let db = setup_db();
        let result = handle_boundary(&db, "src/mod/").unwrap();

        assert!(result.contains("Internal Only"));
        assert!(result.contains("internal_fn"));
    }

    #[test]
    fn test_boundary_empty() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_boundary(&db, "nonexistent/").unwrap();

        assert!(result.contains("No public symbols found under 'nonexistent/'."));
    }

    #[test]
    fn test_boundary_detects_low_confidence_external_callers() {
        let db = Database::open_in_memory().unwrap();
        let mod_file_id = db.insert_file("src/tools/context.rs", "hash1").unwrap();
        store_symbols(
            &db,
            mod_file_id,
            &[Symbol {
                name: "handle_context".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/tools/context.rs".into(),
                line_start: 1,
                line_end: 10,
                signature: "pub fn handle_context()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            }],
        )
        .unwrap();

        let ext_file_id = db.insert_file("src/server.rs", "hash2").unwrap();
        store_symbols(
            &db,
            ext_file_id,
            &[Symbol {
                name: "dispatch".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/server.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub fn dispatch()".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            }],
        )
        .unwrap();

        // Low-confidence external ref (simulates module path resolution)
        let dispatch_id = db
            .get_symbol_id("dispatch", "src/server.rs")
            .unwrap()
            .unwrap();
        let handle_id = db
            .get_symbol_id("handle_context", "src/tools/context.rs")
            .unwrap()
            .unwrap();
        db.insert_symbol_ref(dispatch_id, handle_id, "call", "low", None)
            .unwrap();

        let result = handle_boundary(&db, "src/tools/").unwrap();
        assert!(
            result.contains("Public API"),
            "Low-confidence external caller should make symbol public: {result}"
        );
        assert!(result.contains("handle_context"));
    }
}
