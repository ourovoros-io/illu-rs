use crate::db::{Database, StoredSymbol};
use crate::indexer::parser::SymbolKind;
use std::collections::{BTreeMap, HashSet};
use std::fmt::Write;

pub fn handle_rename_plan(
    db: &Database,
    symbol_name: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let symbols = super::resolve_symbol(db, symbol_name)?;
    if symbols.is_empty() {
        return Ok(super::symbol_not_found(db, symbol_name));
    }

    let base_name = super::base_name(symbol_name);

    let mut output = String::new();
    let _ = writeln!(output, "## Rename Plan: `{symbol_name}`\n");

    write_definitions(&mut output, &symbols);
    let call_count = write_call_sites(&mut output, db, &symbols)?;
    let sig_count = write_signature_usage(&mut output, db, base_name)?;
    let field_count = write_field_usage(&mut output, db, base_name)?;
    let impl_count = write_trait_impls(&mut output, db, base_name)?;
    let doc_count = write_doc_mentions(&mut output, db, base_name)?;

    let total = call_count + sig_count + field_count + impl_count + doc_count;
    let _ = writeln!(output, "**Total: ~{total} locations to update**");

    Ok(output)
}

fn write_definitions(output: &mut String, symbols: &[StoredSymbol]) {
    let _ = writeln!(output, "### Definition\n");
    for sym in symbols {
        let qname = super::qualified_name(sym);
        let _ = writeln!(
            output,
            "- **{qname}** ({}) at {}:{}-{}",
            sym.kind, sym.file_path, sym.line_start, sym.line_end
        );
    }
    let _ = writeln!(output);
}

fn write_call_sites(
    output: &mut String,
    db: &Database,
    symbols: &[StoredSymbol],
) -> Result<usize, Box<dyn std::error::Error>> {
    // Collect all callers across definitions, deduplicating by (name, file)
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut by_file: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for sym in symbols {
        let callers = db.callers(&sym.name, &sym.file_path, false, Some("high"))?;
        for c in callers {
            let key = (c.name.clone(), c.file_path.clone());
            if seen.insert(key) {
                by_file.entry(c.file_path).or_default().push(c.name);
            }
        }
    }
    let total_refs = seen.len();
    if total_refs > 0 {
        let _ = writeln!(output, "### Call Sites ({total_refs} references)\n");
        for (file, names) in &by_file {
            let _ = writeln!(output, "#### {file}\n");
            for name in names {
                let _ = writeln!(output, "- {name}");
            }
            let _ = writeln!(output);
        }
    }
    Ok(total_refs)
}

fn write_signature_usage(
    output: &mut String,
    db: &Database,
    base_name: &str,
) -> Result<usize, Box<dyn std::error::Error>> {
    let sig_matches = db.search_symbols_by_signature(base_name)?;
    let filtered: Vec<_> = sig_matches
        .iter()
        .filter(|s| {
            s.name != base_name
                && s.kind != SymbolKind::Use
                && super::type_usage::contains_whole_word(&s.signature, base_name)
        })
        .collect();
    if !filtered.is_empty() {
        let _ = writeln!(
            output,
            "### Type Usage in Signatures ({} functions)\n",
            filtered.len()
        );
        for sym in &filtered {
            let qname = super::qualified_name(sym);
            let _ = writeln!(
                output,
                "- **{qname}** ({}:{}) \u{2014} `{}`",
                sym.file_path, sym.line_start, sym.signature
            );
        }
        let _ = writeln!(output);
    }
    Ok(filtered.len())
}

fn write_field_usage(
    output: &mut String,
    db: &Database,
    base_name: &str,
) -> Result<usize, Box<dyn std::error::Error>> {
    let mut matches = db.search_symbols_by_details(base_name, "")?;
    matches.retain(|s| s.name != base_name);
    if !matches.is_empty() {
        let _ = writeln!(output, "### Struct Fields ({} structs)\n", matches.len());
        for sym in &matches {
            let _ = writeln!(
                output,
                "- **{}** ({}:{})",
                sym.name, sym.file_path, sym.line_start
            );
        }
        let _ = writeln!(output);
    }
    Ok(matches.len())
}

fn write_trait_impls(
    output: &mut String,
    db: &Database,
    base_name: &str,
) -> Result<usize, Box<dyn std::error::Error>> {
    let as_type = db.trait_impls_for_type(base_name)?;
    let as_trait = db.trait_impls_for_trait(base_name)?;
    let all: Vec<_> = as_type.iter().chain(as_trait.iter()).collect();
    if !all.is_empty() {
        let _ = writeln!(output, "### Trait Implementations ({})\n", all.len());
        for ti in &all {
            let _ = writeln!(
                output,
                "- **{}** for **{}** ({}:{}-{})",
                ti.trait_name, ti.type_name, ti.file_path, ti.line_start, ti.line_end
            );
        }
        let _ = writeln!(output);
    }
    Ok(all.len())
}

fn write_doc_mentions(
    output: &mut String,
    db: &Database,
    base_name: &str,
) -> Result<usize, Box<dyn std::error::Error>> {
    let mentions = db.search_symbols_by_doc_comment(base_name)?;
    let filtered: Vec<_> = mentions.iter().filter(|s| s.name != base_name).collect();
    if !filtered.is_empty() {
        let _ = writeln!(
            output,
            "### Doc Comments Mentioning \"{base_name}\" ({} symbols)\n",
            filtered.len()
        );
        for sym in &filtered {
            let qname = super::qualified_name(sym);
            let _ = writeln!(
                output,
                "- **{qname}** ({}:{})",
                sym.file_path, sym.line_start
            );
        }
        let _ = writeln!(output);
    }
    Ok(filtered.len())
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::indexer::parser::{Confidence, RefKind, Symbol, SymbolKind, Visibility};
    use crate::indexer::store::store_symbols;

    fn make_symbol(name: &str, kind: SymbolKind, file: &str, line: usize) -> Symbol {
        Symbol {
            name: name.into(),
            kind,
            visibility: Visibility::Public,
            file_path: file.into(),
            line_start: line,
            line_end: line + 4,
            signature: format!("pub fn {name}()"),
            doc_comment: None,
            body: None,
            details: None,
            attributes: None,
            impl_type: None,
        }
    }

    #[test]
    fn test_rename_plan_with_callers() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();

        let target = Symbol {
            name: "do_work".into(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            file_path: "src/lib.rs".into(),
            line_start: 1,
            line_end: 5,
            signature: "pub fn do_work()".into(),
            doc_comment: None,
            body: None,
            details: None,
            attributes: None,
            impl_type: None,
        };
        let caller1 = make_symbol("caller_a", SymbolKind::Function, "src/lib.rs", 10);
        let caller2 = make_symbol("caller_b", SymbolKind::Function, "src/lib.rs", 20);

        store_symbols(&db, file_id, &[target, caller1, caller2]).unwrap();

        let target_id = db.symbol_id("do_work", "src/lib.rs").unwrap().unwrap();
        let caller_a_id = db.symbol_id("caller_a", "src/lib.rs").unwrap().unwrap();
        let caller_b_id = db.symbol_id("caller_b", "src/lib.rs").unwrap().unwrap();

        db.insert_symbol_ref(caller_a_id, target_id, RefKind::Call, Confidence::High, None)
            .unwrap();
        db.insert_symbol_ref(caller_b_id, target_id, RefKind::Call, Confidence::High, None)
            .unwrap();

        let result = handle_rename_plan(&db, "do_work").unwrap();

        assert!(result.contains("## Rename Plan: `do_work`"));
        assert!(result.contains("### Definition"));
        assert!(result.contains("### Call Sites (2 references)"));
        assert!(result.contains("caller_a"));
        assert!(result.contains("caller_b"));
    }

    #[test]
    fn test_rename_plan_not_found() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_rename_plan(&db, "nonexistent").unwrap();
        assert!(result.contains("No symbol found"));
    }

    #[test]
    fn test_rename_plan_no_references() {
        let db = Database::open_in_memory().unwrap();
        let file_id = db.insert_file("src/lib.rs", "hash1").unwrap();

        let sym = make_symbol("lonely_fn", SymbolKind::Function, "src/lib.rs", 1);
        store_symbols(&db, file_id, &[sym]).unwrap();

        let result = handle_rename_plan(&db, "lonely_fn").unwrap();

        assert!(result.contains("## Rename Plan: `lonely_fn`"));
        assert!(result.contains("### Definition"));
        assert!(result.contains("Total: ~0 locations to update"));
        assert!(!result.contains("### Call Sites"));
    }
}
