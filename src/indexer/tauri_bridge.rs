use std::sync::LazyLock;

use crate::db::Database;
use crate::indexer::parser::{RefKind, SymbolRef};

#[expect(clippy::expect_used, reason = "static regex literal is always valid")]
static INVOKE_PATTERN: LazyLock<regex_lite::Regex> = LazyLock::new(|| {
    regex_lite::Regex::new(r#"invoke\s*(?:<[^>]*>\s*)?\(\s*['"](\w+)['"]"#)
        .expect("invoke regex is valid")
});

/// Detect if this is a Tauri project.
#[must_use]
pub fn is_tauri_project(repo_path: &std::path::Path) -> bool {
    repo_path.join("src-tauri").is_dir()
        || repo_path.join("tauri.conf.json").exists()
        || repo_path.join("src-tauri/tauri.conf.json").exists()
}

/// Resolve Tauri `invoke('command')` calls in TS to
/// `#[tauri::command]` handlers in Rust.
///
/// Creates `SymbolRef` entries linking TS callers to Rust
/// command handlers via the existing `symbol_refs` table.
pub fn resolve_tauri_bridge(db: &Database) -> Result<usize, crate::IlluError> {
    // Find all TS files and scan for invoke() calls
    let ts_files = db.get_all_file_paths()?;
    let invoke_calls = find_invoke_calls(db, &ts_files)?;

    if invoke_calls.is_empty() {
        return Ok(0);
    }

    // Find Rust functions with tauri::command attribute
    let commands = find_tauri_commands(db)?;

    if commands.is_empty() {
        tracing::debug!(
            invokes = invoke_calls.len(),
            "Found invoke() calls but no #[tauri::command] handlers"
        );
        return Ok(0);
    }

    // Match invoke calls to command handlers
    let mut refs = Vec::new();
    for invoke in &invoke_calls {
        if let Some(cmd) = commands.iter().find(|c| c.name == invoke.command_name) {
            refs.push(SymbolRef {
                source_name: invoke.caller_name.clone(),
                source_file: invoke.caller_file.clone(),
                target_name: cmd.name.clone(),
                kind: RefKind::Call,
                target_file: Some(cmd.file.clone()),
                target_context: Some("tauri_command".to_string()),
                ref_line: Some(invoke.line),
            });
        } else {
            tracing::debug!(
                command = invoke.command_name,
                file = invoke.caller_file,
                "Unresolved Tauri invoke"
            );
        }
    }

    let count = refs.len();
    if count > 0 {
        let symbol_map = db.build_symbol_id_map()?;
        db.begin_transaction()?;
        db.store_symbol_refs_fast(&refs, &symbol_map)?;
        db.commit()?;
        tracing::info!(refs = count, "Resolved Tauri bridge references");
    }

    Ok(count)
}

struct InvokeCall {
    command_name: String,
    caller_name: String,
    caller_file: String,
    line: i64,
}

struct TauriCommand {
    name: String,
    file: String,
}

/// Find `invoke('command_name')` calls in TS source bodies.
fn find_invoke_calls(db: &Database, files: &[String]) -> Result<Vec<InvokeCall>, crate::IlluError> {
    let mut calls = Vec::new();

    for file_path in files {
        if !super::is_ts_file(file_path) {
            continue;
        }

        let symbols = db.get_symbols_by_path_prefix(file_path)?;

        for sym in &symbols {
            let Some(body) = &sym.body else {
                continue;
            };

            for cap in INVOKE_PATTERN.captures_iter(body) {
                if let Some(cmd) = cap.get(1) {
                    let line_offset = body[..cmd.start()].chars().filter(|&c| c == '\n').count();
                    let call_line = sym.line_start + i64::try_from(line_offset).unwrap_or(0);
                    calls.push(InvokeCall {
                        command_name: cmd.as_str().to_string(),
                        caller_name: sym.name.clone(),
                        caller_file: file_path.clone(),
                        line: call_line,
                    });
                }
            }
        }
    }

    Ok(calls)
}

/// Find Rust functions annotated with
/// `#[tauri::command]`.
fn find_tauri_commands(db: &Database) -> Result<Vec<TauriCommand>, crate::IlluError> {
    let symbols = db.search_symbols_by_attribute("tauri::command")?;
    let commands: Vec<TauriCommand> = symbols
        .into_iter()
        .map(|s| TauriCommand {
            name: s.name,
            file: s.file_path,
        })
        .collect();
    Ok(commands)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn test_is_tauri_project() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(!is_tauri_project(dir.path()));

        std::fs::create_dir_all(dir.path().join("src-tauri")).unwrap();
        assert!(is_tauri_project(dir.path()));
    }

    #[test]
    fn test_invoke_regex_patterns() {
        let re = regex_lite::Regex::new(r#"invoke\s*(?:<[^>]*>\s*)?\(\s*['"](\w+)['"]"#).unwrap();

        // Standard invoke
        let cap = re.captures(r"invoke('get_config')").unwrap();
        assert_eq!(&cap[1], "get_config");

        // Typed invoke with generic
        let cap = re
            .captures(r#"invoke<Config>("get_config", { key })"#)
            .unwrap();
        assert_eq!(&cap[1], "get_config");

        // Double quotes
        let cap = re.captures(r#"invoke("save_data")"#).unwrap();
        assert_eq!(&cap[1], "save_data");

        // No match for non-invoke
        assert!(re.captures("console.log('test')").is_none());
    }
}
