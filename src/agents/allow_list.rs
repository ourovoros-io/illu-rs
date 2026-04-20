//! Auto-allow the illu MCP tool pattern in Claude-family settings files.

use std::path::Path;

/// Ensure all illu MCP tools are auto-allowed in the settings file at `config_path`.
///
/// The tool pattern (`mcp__illu__*`) is appended to `permissions.allow` if not
/// already present. Other keys are preserved. If the file exists but is not
/// valid JSON, the parse error is propagated so the user's settings are never
/// clobbered.
///
/// # Errors
///
/// Returns an error if the file exists but is not valid JSON, or if I/O on
/// the file or its parent directory fails.
pub fn ensure_tools_allowed(config_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut config: serde_json::Value = match std::fs::read_to_string(config_path) {
        Ok(s) => serde_json::from_str(&s)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(e) => return Err(e.into()),
    };

    let pattern = "mcp__illu__*";

    let allow = config["permissions"]["allow"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let already = allow
        .iter()
        .any(|v| v.as_str().is_some_and(|s| s == pattern));

    if !already {
        let mut allow = allow;
        allow.push(serde_json::json!(pattern));
        config["permissions"]["allow"] = serde_json::Value::Array(allow);
        std::fs::write(config_path, serde_json::to_string_pretty(&config)?)?;
    }

    Ok(())
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn adds_pattern_to_empty_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        ensure_tools_allowed(&path).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("mcp__illu__*"));
    }

    #[test]
    fn is_idempotent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        ensure_tools_allowed(&path).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        ensure_tools_allowed(&path).unwrap();
        let second = std::fs::read_to_string(&path).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn errors_on_malformed_existing_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{ this is not json").unwrap();
        let err = ensure_tools_allowed(&path).unwrap_err();
        // User's file content must NOT be clobbered.
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "{ this is not json");
        // Error should be a parse-like error.
        let msg = format!("{err}").to_lowercase();
        assert!(
            msg.contains("expected")
                || msg.contains("parse")
                || msg.contains("key must")
                || msg.contains("line "),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn preserves_existing_permissions() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"permissions":{"allow":["existing_*"]},"other":"keep"}"#,
        )
        .unwrap();
        ensure_tools_allowed(&path).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let allow = v["permissions"]["allow"].as_array().unwrap();
        assert!(allow.iter().any(|e| e == "existing_*"));
        assert!(allow.iter().any(|e| e == "mcp__illu__*"));
        assert_eq!(v["other"], "keep");
    }
}
