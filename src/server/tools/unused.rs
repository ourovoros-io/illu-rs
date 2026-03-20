use crate::db::Database;
use std::fmt::Write;

pub fn handle_unused(
    db: &Database,
    path: Option<&str>,
    kind: Option<&str>,
    include_private: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut symbols = db.get_unreferenced_symbols(path, include_private)?;

    if let Some(k) = kind {
        let k_lower = k.to_lowercase();
        symbols.retain(|s| s.kind.to_string().to_lowercase() == k_lower);
    }

    // Exclude entry points
    symbols.retain(|s| {
        if s.name == "main" {
            return false;
        }
        if let Some(attrs) = &s.attributes
            && attrs.contains("test")
        {
            return false;
        }
        true
    });

    // Exclude enum variants (referenced through their parent enum)
    symbols.retain(|s| s.kind != crate::indexer::parser::SymbolKind::EnumVariant);

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

    let mut current_file = String::new();
    for sym in &symbols {
        if sym.file_path != current_file {
            current_file.clone_from(&sym.file_path);
            let _ = writeln!(output, "### {current_file}\n");
        }
        let _ = writeln!(
            output,
            "- **{}** ({}, {}, line {}-{})",
            sym.name, sym.kind, sym.visibility, sym.line_start, sym.line_end
        );
    }

    let _ = writeln!(
        output,
        "\n*Note: entry points (main, #[test]) are excluded. \
         Symbols used via macros, dynamic dispatch, or external \
         crates may appear as false positives.*"
    );

    Ok(output)
}
