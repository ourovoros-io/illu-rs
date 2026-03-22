use crate::db::{Database, StoredSymbol};
use std::collections::HashSet;
use std::fmt::Write;

type ScoredSymbol<'a> = (usize, &'a StoredSymbol, Vec<String>);

use super::NOISY_CALLEES;

pub fn handle_similar(
    db: &Database,
    symbol_name: &str,
    path: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let symbols = super::resolve_symbol(db, symbol_name)?;
    if symbols.is_empty() {
        return Ok(super::symbol_not_found(symbol_name));
    }

    let target = &symbols[0];
    let target_sig = &target.signature;
    let return_type = target_sig.split_once("->").map(|(_, r)| r.trim());

    let target_callees: HashSet<String> = db
        .get_callees_by_name(&target.name, None, false)?
        .into_iter()
        .map(|(name, _)| name)
        .collect();

    // Gather candidates from both return type AND parameter types
    let mut candidates = Vec::new();
    if let Some(ret) = return_type {
        let core_type = extract_core_type(ret);
        if core_type.len() >= 3 {
            candidates = db.search_symbols_by_signature(core_type)?;
        } else {
            candidates = db.search_symbols_by_signature(ret.trim())?;
        }
    }

    // Also search by shared parameter types to find matches with identical params
    let target_params = extract_param_section(target_sig);
    if let Some(tp) = target_params {
        for word in tp.split(|c: char| !c.is_alphanumeric() && c != '_') {
            if word.len() >= 4 && word.chars().next().is_some_and(char::is_uppercase) {
                let mut param_matches = db.search_symbols_by_signature(word)?;
                for m in param_matches.drain(..) {
                    if !candidates
                        .iter()
                        .any(|c| c.name == m.name && c.file_path == m.file_path)
                    {
                        candidates.push(m);
                    }
                }
                break;
            }
        }
    }

    // Also include same-file siblings of the same kind as candidates
    let mut same_file = db.get_symbols_by_path_prefix(&target.file_path)?;
    for s in same_file.drain(..) {
        if s.kind == target.kind
            && (s.name != target.name || s.file_path != target.file_path)
            && !candidates
                .iter()
                .any(|c| c.name == s.name && c.file_path == s.file_path)
        {
            candidates.push(s);
        }
    }

    candidates.retain(|s| {
        (s.name != target.name || s.file_path != target.file_path) && s.kind == target.kind
    });
    if let Some(p) = path {
        candidates.retain(|s| s.file_path.starts_with(p));
    }

    let scored = score_candidates(db, target, &candidates, return_type, &target_callees)?;

    let mut output = String::new();
    let qname = super::qualified_name(target);
    let _ = writeln!(output, "## Similar to `{qname}`\n");
    let _ = writeln!(output, "Signature: `{target_sig}`\n");

    if scored.is_empty() {
        let _ = writeln!(output, "No similar symbols found.");
        return Ok(output);
    }

    let _ = writeln!(output, "### Similar Symbols\n");
    for (i, (score, sym, reasons)) in scored.iter().enumerate() {
        let cqname = super::qualified_name(sym);
        let _ = writeln!(
            output,
            "{}. **{cqname}** (score: {score}) — {}:{}",
            i + 1,
            sym.file_path,
            sym.line_start
        );
        let _ = writeln!(output, "   `{}`", sym.signature);
        let _ = writeln!(output, "   Shared: {}", reasons.join(", "));
    }

    Ok(output)
}

fn extract_core_type(ret: &str) -> &str {
    let inner = ret
        .trim_start_matches("Result<")
        .trim_start_matches("Option<");
    // Extract just the type name without generics: Vec<u8> → Vec
    inner
        .split([',', '>', '<', ' '])
        .next()
        .unwrap_or(ret)
        .trim()
}

fn score_candidates<'a>(
    db: &Database,
    target: &StoredSymbol,
    candidates: &'a [StoredSymbol],
    return_type: Option<&str>,
    target_callees: &HashSet<String>,
) -> Result<Vec<ScoredSymbol<'a>>, Box<dyn std::error::Error>> {
    let target_params = extract_param_section(&target.signature);
    let mut scored: Vec<ScoredSymbol<'_>> = Vec::new();

    for candidate in candidates {
        let (score, reasons) = score_one(
            db,
            target_params,
            &candidate.signature,
            &candidate.name,
            return_type,
            target_callees,
        )?;
        if score >= 2 {
            scored.push((score, candidate, reasons));
        }
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.truncate(10);
    Ok(scored)
}

fn score_one(
    db: &Database,
    target_params: Option<&str>,
    cand_sig: &str,
    cand_name: &str,
    return_type: Option<&str>,
    target_callees: &HashSet<String>,
) -> Result<(usize, Vec<String>), Box<dyn std::error::Error>> {
    let mut score = 0usize;
    let mut reasons = Vec::new();

    if let Some(ret) = return_type
        && cand_sig.contains(ret.trim())
    {
        score += 2;
        reasons.push(format!("return type `{}`", ret.trim()));
    }

    let cand_params = extract_param_section(cand_sig);
    if let (Some(tp), Some(cp)) = (target_params, cand_params) {
        // Score self-receiver matching
        if tp.contains("&mut self") && cp.contains("&mut self") {
            score += 2;
            reasons.push("`&mut self` receiver".to_string());
        } else if tp.contains("&self") && cp.contains("&self") {
            score += 1;
            reasons.push("`&self` receiver".to_string());
        }

        // Score ALL matching parameter types (not just the first)
        let mut matched_types = Vec::new();
        for word in tp.split(|c: char| !c.is_alphanumeric() && c != '_') {
            if word.len() >= 3
                && word.chars().next().is_some_and(char::is_uppercase)
                && word != "Self"
                && cp.contains(word)
                && !matched_types.contains(&word)
            {
                score += 1;
                matched_types.push(word);
            }
        }
        if !matched_types.is_empty() {
            reasons.push(format!("param types: {}", matched_types.join(", ")));
        }
    }

    if !target_callees.is_empty() {
        let cand_callees: HashSet<String> = db
            .get_callees_by_name(cand_name, None, false)?
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        let shared: Vec<_> = target_callees
            .intersection(&cand_callees)
            .filter(|name| !NOISY_CALLEES.contains(&name.as_str()))
            .collect();
        if !shared.is_empty() {
            score += shared.len();
            let names: Vec<&str> = shared.iter().take(3).map(|s| s.as_str()).collect();
            reasons.push(format!("shared callees: {}", names.join(", ")));
        }
    }

    Ok((score, reasons))
}

fn extract_param_section(sig: &str) -> Option<&str> {
    let start = sig.find('(')?;
    let end = sig.rfind(')')?;
    if start < end {
        Some(&sig[start + 1..end])
    } else {
        None
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
            Symbol {
                name: "load_config".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 5,
                signature: "pub fn load_config(path: &Path) -> Config".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "parse_config".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 7,
                line_end: 12,
                signature: "pub fn parse_config(data: &str) -> Config".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            },
            Symbol {
                name: "unrelated".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 14,
                line_end: 18,
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
    fn test_similar_finds_matching_signature() {
        let db = setup_db();
        let result = handle_similar(&db, "load_config", None).unwrap();

        assert!(result.contains("## Similar to `load_config`"));
        assert!(result.contains("parse_config"));
        assert!(result.contains("return type `Config`"));
        assert!(!result.contains("unrelated"));
    }

    #[test]
    fn test_similar_not_found() {
        let db = setup_db();
        let result = handle_similar(&db, "nonexistent", None).unwrap();

        assert!(result.contains("No symbol found"));
    }

    #[test]
    fn test_similar_no_matches() {
        let db = setup_db();
        let result = handle_similar(&db, "unrelated", None).unwrap();

        assert!(result.contains("## Similar to `unrelated`"));
        assert!(result.contains("No similar symbols found."));
    }

    #[test]
    fn test_similar_excludes_noisy_callees() {
        use crate::indexer::parser::{RefKind, SymbolRef};

        let db = Database::open_in_memory().unwrap();
        let f1 = db.insert_file("src/lib.rs", "hash1").unwrap();
        let f2 = db.insert_file("src/util.rs", "hash2").unwrap();

        store_symbols(
            &db,
            f1,
            &[Symbol {
                name: "build_report".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                file_path: "src/lib.rs".into(),
                line_start: 1,
                line_end: 10,
                signature: "pub fn build_report() -> String".into(),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: None,
            }],
        )
        .unwrap();

        store_symbols(
            &db,
            f2,
            &[
                Symbol {
                    name: "build_summary".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/util.rs".into(),
                    line_start: 1,
                    line_end: 10,
                    signature: "pub fn build_summary() -> String".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
                Symbol {
                    name: "new".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    file_path: "src/util.rs".into(),
                    line_start: 12,
                    line_end: 15,
                    signature: "pub fn new() -> Self".into(),
                    doc_comment: None,
                    body: None,
                    details: None,
                    attributes: None,
                    impl_type: None,
                },
            ],
        )
        .unwrap();

        let map = db.build_symbol_id_map().unwrap();
        db.store_symbol_refs_fast(
            &[
                SymbolRef {
                    source_name: "build_report".into(),
                    target_name: "new".into(),
                    source_file: "src/lib.rs".into(),
                    target_file: Some("src/util.rs".into()),
                    kind: RefKind::Call,
                    target_context: None,
                    ref_line: Some(3),
                },
                SymbolRef {
                    source_name: "build_summary".into(),
                    target_name: "new".into(),
                    source_file: "src/util.rs".into(),
                    target_file: Some("src/util.rs".into()),
                    kind: RefKind::Call,
                    target_context: None,
                    ref_line: Some(3),
                },
            ],
            &map,
        )
        .unwrap();

        let result = handle_similar(&db, "build_report", None).unwrap();
        // Should find build_summary via return type, not via "new"
        assert!(result.contains("build_summary"));
        assert!(
            !result.contains("shared callees: new"),
            "Should not count 'new' as meaningful shared callee\n{result}"
        );
    }
}
