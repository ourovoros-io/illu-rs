//! Writers for each MCP config file format.

use super::{IlluCommand, McpFormat};
use std::path::Path;

pub fn write(path: &Path, format: McpFormat, cmd: &IlluCommand) -> Result<(), crate::IlluError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match format {
        McpFormat::ClaudeCodeJson
        | McpFormat::GeminiJson
        | McpFormat::ClaudeDesktopJson
        | McpFormat::CursorJson
        | McpFormat::AntigravityJson => write_mcp_servers_json(path, cmd),
        McpFormat::VsCodeJson => write_vscode_json(path, cmd),
        McpFormat::CodexToml => write_codex_toml(path, cmd),
    }
}

fn write_mcp_servers_json(path: &Path, cmd: &IlluCommand) -> Result<(), crate::IlluError> {
    let mut config: serde_json::Value = match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            serde_json::json!({"mcpServers": {}})
        }
        Err(e) => return Err(e.into()),
    };

    let entry = serde_json::json!({
        "command": cmd.command,
        "args": cmd.args,
        "env": { "RUST_LOG": "warn" }
    });
    config["mcpServers"]["illu"] = entry;
    std::fs::write(path, serde_json::to_string_pretty(&config)?)?;
    Ok(())
}

fn write_vscode_json(path: &Path, cmd: &IlluCommand) -> Result<(), crate::IlluError> {
    let mut config: serde_json::Value = match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            serde_json::json!({"servers": {}})
        }
        Err(e) => return Err(e.into()),
    };

    let entry = serde_json::json!({
        "type": "stdio",
        "command": cmd.command,
        "args": cmd.args,
        "env": { "RUST_LOG": "warn" }
    });
    config["servers"]["illu"] = entry;
    std::fs::write(path, serde_json::to_string_pretty(&config)?)?;
    Ok(())
}

fn write_codex_toml(path: &Path, cmd: &IlluCommand) -> Result<(), crate::IlluError> {
    use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value};

    let existing = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    let mut doc = existing.parse::<DocumentMut>()?;

    let mcp_servers = doc
        .entry("mcp_servers")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .ok_or("mcp_servers is not a table")?;
    mcp_servers.set_implicit(true);

    let illu = mcp_servers
        .entry("illu")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .ok_or("mcp_servers.illu is not a table")?;

    illu.insert("command", Item::Value(Value::from(cmd.command.clone())));
    let mut args = Array::new();
    for a in &cmd.args {
        args.push(a.clone());
    }
    illu.insert("args", Item::Value(Value::Array(args)));

    let mut env = InlineTable::new();
    env.insert("RUST_LOG", Value::from("warn"));
    illu.insert("env", Item::Value(Value::InlineTable(env)));

    std::fs::write(path, doc.to_string())?;
    Ok(())
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn cmd(repo: &Path) -> IlluCommand {
        IlluCommand::serve(repo)
    }

    #[test]
    fn writes_mcp_servers_json_fresh() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".mcp.json");
        write(&path, McpFormat::ClaudeCodeJson, &cmd(dir.path())).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["illu"]["command"], "illu-rs");
        // `serve(repo)` emits exactly `["--repo", "<abs>", "serve"]`. Pin the
        // length too so any future flag addition is a deliberate test update
        // rather than a silent pass at the serialization layer.
        let args = v["mcpServers"]["illu"]["args"].as_array().unwrap();
        assert_eq!(args.len(), 3, "expected 3 args, got {args:?}");
        assert_eq!(args[0], "--repo");
        assert_eq!(args[2], "serve");
    }

    #[test]
    fn preserves_other_mcp_servers() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".mcp.json");
        std::fs::write(&path, r#"{"mcpServers":{"other":{"command":"x"}}}"#).unwrap();
        write(&path, McpFormat::ClaudeCodeJson, &cmd(dir.path())).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["other"]["command"], "x");
        assert_eq!(v["mcpServers"]["illu"]["command"], "illu-rs");
    }

    #[test]
    fn vscode_uses_servers_key_and_type_stdio() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".vscode/mcp.json");
        write(&path, McpFormat::VsCodeJson, &cmd(dir.path())).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["servers"]["illu"]["type"], "stdio");
        assert_eq!(v["servers"]["illu"]["command"], "illu-rs");
    }

    #[test]
    fn codex_toml_writes_mcp_servers_illu_section() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write(&path, McpFormat::CodexToml, &cmd(dir.path())).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("[mcp_servers.illu]"));
        assert!(content.contains("command = \"illu-rs\""));
        assert!(content.contains("\"--repo\""));
        assert!(content.contains("\"serve\""));
    }

    #[test]
    fn codex_toml_preserves_unrelated_sections() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[user]\nname = \"alice\"\n\n[mcp_servers.existing]\ncommand = \"x\"\n",
        )
        .unwrap();
        write(&path, McpFormat::CodexToml, &cmd(dir.path())).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("[user]"));
        assert!(content.contains("name = \"alice\""));
        assert!(content.contains("[mcp_servers.existing]"));
        assert!(content.contains("[mcp_servers.illu]"));
    }

    #[test]
    fn idempotent_rewrites() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".mcp.json");
        write(&path, McpFormat::ClaudeCodeJson, &cmd(dir.path())).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        write(&path, McpFormat::ClaudeCodeJson, &cmd(dir.path())).unwrap();
        let second = std::fs::read_to_string(&path).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn errors_on_malformed_existing_json() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".mcp.json");
        std::fs::write(&path, "{this is not json").unwrap();
        let err = write(&path, McpFormat::ClaudeCodeJson, &cmd(dir.path())).unwrap_err();
        // Assert the user's existing file is NOT clobbered.
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "{this is not json");
        // Error should be a serde_json parse error referencing a line/column.
        let msg = format!("{err}").to_lowercase();
        assert!(
            msg.contains("expected")
                || msg.contains("parse")
                || msg.contains("json")
                || msg.contains("key must be a string")
                || msg.contains("line "),
            "unexpected error: {err}"
        );
    }
}
