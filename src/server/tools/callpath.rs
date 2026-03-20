use crate::db::Database;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write;

pub fn handle_callpath(
    db: &Database,
    from: &str,
    to: &str,
    max_depth: Option<i64>,
) -> Result<String, Box<dyn std::error::Error>> {
    let max_depth = usize::try_from(max_depth.unwrap_or(10).max(1))
        .unwrap_or(10);

    let from_syms = db.search_symbols(from)?;
    if from_syms.is_empty() {
        return Ok(format!("Source symbol '{from}' not found."));
    }
    let to_syms = db.search_symbols(to)?;
    if to_syms.is_empty() {
        return Ok(format!("Target symbol '{to}' not found."));
    }

    let mut visited: HashSet<String> = HashSet::new();
    let mut parent: HashMap<String, String> = HashMap::new();
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();

    visited.insert(from.to_string());
    queue.push_back((from.to_string(), 0));

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

            if callee_name == to {
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

    let mut path = vec![to.to_string()];
    let mut current = to.to_string();
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
