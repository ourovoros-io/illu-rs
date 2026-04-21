use crate::db::Database;
use crate::indexer::parser::SymbolKind;
use std::fmt::Write;

pub fn handle_unused(
    db: &Database,
    path: Option<&str>,
    kind: Option<&str>,
    include_private: bool,
    untested: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    if untested {
        return handle_untested(db, path, kind, include_private);
    }

    let mut symbols = db.get_unreferenced_symbols(path, include_private)?;

    super::retain_kind(&mut symbols, kind);

    symbols.retain(|s| !super::is_entry_point(s));
    symbols.retain(|s| s.kind != SymbolKind::EnumVariant && s.kind != SymbolKind::Impl);

    let mut output = String::new();
    let _ = writeln!(output, "## Potentially Unused Symbols\n");

    if symbols.is_empty() {
        let _ = writeln!(output, "No unreferenced symbols found.");
        return Ok(output);
    }

    let _ = writeln!(
        output,
        "Found {} symbols with no incoming references:\n",
        symbols.len()
    );

    render_symbol_list(&mut output, &symbols);

    let _ = writeln!(
        output,
        "\n*Note: entry points (main, #[test]) are excluded. \
         Symbols used via macros, dynamic dispatch, or external \
         crates may appear as false positives.*"
    );

    Ok(output)
}

fn handle_untested(
    db: &Database,
    path: Option<&str>,
    kind: Option<&str>,
    include_private: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let prefix = path.unwrap_or("");
    let mut symbols = db.get_symbols_by_path_prefix_filtered(prefix, include_private)?;

    // Only functions are meaningfully "testable"
    let kind_filter = kind.unwrap_or("function");
    let k_lower = kind_filter.to_lowercase();
    symbols.retain(|s| s.kind.to_string().to_lowercase() == k_lower);

    symbols.retain(|s| !super::is_entry_point(s));
    symbols.retain(|s| {
        s.kind != SymbolKind::EnumVariant
            && s.kind != SymbolKind::Use
            && s.kind != SymbolKind::Mod
            && s.kind != SymbolKind::Impl
    });

    // Filter to symbols with no related tests
    let mut untested = Vec::new();
    for sym in symbols {
        let tests = db.get_related_tests(&sym.name, sym.impl_type.as_deref())?;
        if tests.is_empty() {
            untested.push(sym);
        }
    }

    let mut output = String::new();
    let _ = writeln!(output, "## Untested Symbols\n");

    if untested.is_empty() {
        let _ = writeln!(output, "All matching symbols have test coverage.");
        return Ok(output);
    }

    let _ = writeln!(
        output,
        "Found {} symbols with no test coverage:\n",
        untested.len()
    );

    render_symbol_list(&mut output, &untested);

    let _ = writeln!(
        output,
        "\n*Note: symbols tested only via macros or dynamic \
         dispatch may appear as false positives.*"
    );

    Ok(output)
}

fn render_symbol_list(output: &mut String, symbols: &[crate::db::StoredSymbol]) {
    let mut current_file = String::new();
    for sym in symbols {
        if sym.file_path != current_file {
            current_file.clone_from(&sym.file_path);
            let _ = writeln!(output, "### {current_file}\n");
        }
        let qualified = super::qualified_name(sym);
        let _ = writeln!(
            output,
            "- **{qualified}** ({}, {}, line {}-{})",
            sym.kind, sym.visibility, sym.line_start, sym.line_end
        );
    }
}
