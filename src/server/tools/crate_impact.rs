use crate::db::Database;
use std::collections::BTreeMap;
use std::fmt::Write;

pub fn handle_crate_impact(db: &Database, symbol_name: &str) -> Result<String, crate::IlluError> {
    let crate_count = db.get_crate_count()?;
    if crate_count <= 1 {
        return Ok("Crate impact analysis requires a workspace with multiple crates.".into());
    }

    let symbols = super::resolve_symbol(db, symbol_name)?;
    if symbols.is_empty() {
        return Ok(super::symbol_not_found(db, symbol_name));
    }

    let mut output = String::new();
    let _ = writeln!(output, "## Crate Impact: {symbol_name}\n");

    let sym = &symbols[0];
    let defining_crate = db.get_crate_for_file(&sym.file_path)?;
    if let Some(ref stored_crate) = defining_crate {
        let _ = writeln!(output, "**Defined in crate:** `{}`\n", stored_crate.name);
    }

    if let Some(ref stored_crate) = defining_crate {
        let dependents = db.get_transitive_crate_dependents(stored_crate.id)?;
        if dependents.is_empty() {
            let _ = writeln!(output, "No other crates depend on `{}`.", stored_crate.name);
        } else {
            let _ = writeln!(output, "### Affected Crates\n");
            for (i, dep) in dependents.iter().enumerate() {
                let _ = writeln!(output, "{}. `{}`", i + 1, dep.name);
            }
        }
    } else {
        let _ = writeln!(output, "Could not determine the crate for this symbol.");
    }

    let dependents = db.impact_dependents(&sym.name, sym.impl_type.as_deref())?;
    if !dependents.is_empty() {
        let mut crate_counts: BTreeMap<String, usize> = BTreeMap::new();
        for dep in &dependents {
            let prefix = dep.file_path.split('/').next().unwrap_or(&dep.file_path);
            *crate_counts.entry(prefix.to_string()).or_default() += 1;
        }

        let _ = writeln!(output, "\n### Symbol-Level Impact by Module\n");
        for (prefix, count) in &crate_counts {
            let _ = writeln!(output, "- `{prefix}` -- {count} dependent symbol(s)");
        }
    }

    Ok(output)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn test_crate_impact_single_crate() {
        let db = Database::open_in_memory().unwrap();
        let result = handle_crate_impact(&db, "anything").unwrap();
        assert!(result.contains("requires a workspace"));
    }

    #[test]
    fn test_crate_impact_not_found() {
        let db = Database::open_in_memory().unwrap();
        db.insert_crate("crate_a", "crate_a/").unwrap();
        db.insert_crate("crate_b", "crate_b/").unwrap();
        let result = handle_crate_impact(&db, "nonexistent").unwrap();
        assert!(result.contains("No symbol found"));
    }
}
