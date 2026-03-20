use crate::db::{Database, StoredSymbol};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write;

/// Resolve a symbol name supporting `Type::method` syntax.
fn resolve_symbol(
    db: &Database,
    name: &str,
) -> Result<Vec<StoredSymbol>, Box<dyn std::error::Error>> {
    if let Some((impl_type, method)) = name.split_once("::") {
        let results = db.search_symbols_by_impl(impl_type, method)?;
        if !results.is_empty() {
            return Ok(results);
        }
    }
    Ok(db.search_symbols(name)?)
}

/// Extract the base symbol name (without `Type::` prefix).
fn base_name(name: &str) -> &str {
    name.split_once("::").map_or(name, |(_, m)| m)
}

pub fn handle_callpath(
    db: &Database,
    from: &str,
    to: &str,
    max_depth: Option<i64>,
) -> Result<String, Box<dyn std::error::Error>> {
    let max_depth = usize::try_from(max_depth.unwrap_or(10).max(1))
        .unwrap_or(10);

    let from_syms = resolve_symbol(db, from)?;
    if from_syms.is_empty() {
        return Ok(format!("Source symbol '{from}' not found."));
    }
    let to_syms = resolve_symbol(db, to)?;
    if to_syms.is_empty() {
        return Ok(format!("Target symbol '{to}' not found."));
    }

    let from_name = base_name(from);
    let to_name = base_name(to);

    let mut visited: HashSet<String> = HashSet::new();
    let mut parent: HashMap<String, String> = HashMap::new();
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();

    visited.insert(from_name.to_string());
    queue.push_back((from_name.to_string(), 0));

    let mut found = false;
    while let Some((current, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }

        let callees = db.get_callees_by_name(&current)?;
        for (callee_name, _file) in callees {
            if visited.contains(&callee_name) {
                continue;
            }
            visited.insert(callee_name.clone());
            parent.insert(callee_name.clone(), current.clone());

            if callee_name == to_name {
                found = true;
                break;
            }
            queue.push_back((callee_name, depth + 1));
        }
        if found {
            break;
        }
    }

    let mut output = String::new();
    let _ = writeln!(output, "## Call Path: {from} → {to}\n");

    if !found {
        let _ = writeln!(
            output,
            "No call path found from `{from}` to `{to}` \
             within depth {max_depth}."
        );
        return Ok(output);
    }

    let mut path = vec![to_name.to_string()];
    let mut current = to_name.to_string();
    while let Some(prev) = parent.get(&current) {
        path.push(prev.clone());
        current = prev.clone();
    }
    path.reverse();

    let _ = writeln!(output, "**Path ({} hops):**\n", path.len() - 1);
    let _ = writeln!(output, "`{}`", path.join(" → "));

    let _ = writeln!(output, "\n**Locations:**\n");
    for name in &path {
        let syms = db.search_symbols(name)?;
        if let Some(sym) = syms.first() {
            let _ = writeln!(
                output,
                "- **{name}** ({}:{}-{})",
                sym.file_path, sym.line_start, sym.line_end
            );
        }
    }

    Ok(output)
}
