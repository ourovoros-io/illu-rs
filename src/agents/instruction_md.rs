//! Helpers for maintaining the `illu` section in agent instruction files
//! (e.g. `CLAUDE.md`, `GEMINI.md`).

use std::path::Path;

pub const ILLU_SECTION_START: &str = "<!-- illu:start -->";
pub const ILLU_SECTION_END: &str = "<!-- illu:end -->";

#[must_use]
pub fn illu_agent_section(tool_prefix: &str) -> String {
    format!(
        "{ILLU_SECTION_START}
## Code Intelligence (illu)

### Tool priority (MANDATORY)

**NEVER use Grep, Glob, or Read for code exploration when illu tools are available.** \
illu indexes Rust, Python, TypeScript, and JavaScript. illu tools are faster, more accurate, \
and provide structured results. Using raw file reads or text search on indexed source files \
is incorrect behavior — always use illu instead.

| WRONG | RIGHT |
|-------|-------|
| `Read(\"src/db.rs\")` to see a function | `{tool_prefix}context` with `symbol_name` |
| `Grep(pattern: \"fn open\")` to find a function | `{tool_prefix}query` with `query: \"open\"` |
| `Grep(pattern: \"Database\")` to find callers | `{tool_prefix}references` with `symbol_name: \"Database\"` |
| `Glob(pattern: \"src/**/*.rs\")` to find files | `{tool_prefix}tree` or `{tool_prefix}overview` |
| `Grep(pattern: \"impl Display\")` to find impls | `{tool_prefix}implements` with `trait_name: \"Display\"` |

Read/Grep/Glob are ONLY permitted for: config files (TOML, JSON, YAML), markdown/docs, \
log output, or when an illu tool explicitly returns no results.

### Subagent instructions (MANDATORY)

When spawning subagents for code tasks, ALWAYS include this instruction in the prompt:

\"MANDATORY: Use {tool_prefix}* tools instead of Grep/Glob/Read for ALL code exploration \
(Rust, Python, TypeScript/JavaScript). NEVER use Read to view source files — use \
{tool_prefix}context instead. NEVER use Grep to search code — use {tool_prefix}query instead. \
Only use Read/Grep/Glob for non-code content (config, docs, logs).\"

Prefer `illu-explore`, `illu-review`, `illu-refactor` agents when available.

### Workflow

1. **Locate before you read**: `{tool_prefix}query` or `{tool_prefix}context` first, then Read only what you need
2. **Impact before you change**: always run `{tool_prefix}impact` before modifying any public symbol
3. **Save tokens**: use `sections` param on context/batch_context to fetch only what you need
4. **Production focus**: use `exclude_tests: true` to filter out test functions
5. **Cross-repo**: use `{tool_prefix}cross_query`/`{tool_prefix}cross_impact`/`{tool_prefix}cross_deps`/`{tool_prefix}cross_callpath` — \
NEVER navigate to or read files from other repositories directly
{ILLU_SECTION_END}"
    )
}

pub fn write_md_section(
    repo_path: &Path,
    file_name: &str,
    heading: &str,
    section: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let md_path = repo_path.join(file_name);

    let content = std::fs::read_to_string(&md_path).unwrap_or_default();

    // Skip write if section already exists and is identical
    if content.contains(ILLU_SECTION_START) && content.contains(section) {
        return Ok(());
    }

    let new_content = if let Some(start) = content.find(ILLU_SECTION_START) {
        if let Some(end) = content.find(ILLU_SECTION_END) {
            let end = end + ILLU_SECTION_END.len();
            format!("{}{section}{}", &content[..start], &content[end..])
        } else {
            format!("{}{section}{}", &content[..start], &content[start..])
        }
    } else if content.is_empty() {
        format!("{heading}\n\n{section}\n")
    } else {
        format!("{content}\n{section}\n")
    };

    std::fs::write(&md_path, new_content)?;
    tracing::info!("Updated {file_name} with illu section");
    Ok(())
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn section_contains_tool_prefix() {
        let section = illu_agent_section("mcp__illu__");
        assert!(section.contains("mcp__illu__query"));
        assert!(section.contains(ILLU_SECTION_START));
        assert!(section.contains(ILLU_SECTION_END));
    }

    #[test]
    fn write_md_section_creates_file_when_missing() {
        let dir = tempdir().unwrap();
        let section = illu_agent_section("mcp__illu__");
        write_md_section(dir.path(), "CLAUDE.md", "# CLAUDE.md", &section).unwrap();
        let content = std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        assert!(content.starts_with("# CLAUDE.md"));
        assert!(content.contains(ILLU_SECTION_START));
    }

    #[test]
    fn write_md_section_is_idempotent() {
        let dir = tempdir().unwrap();
        let section = illu_agent_section("mcp__illu__");
        write_md_section(dir.path(), "CLAUDE.md", "# CLAUDE.md", &section).unwrap();
        let first = std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        write_md_section(dir.path(), "CLAUDE.md", "# CLAUDE.md", &section).unwrap();
        let second = std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn write_md_section_preserves_unrelated_content() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "# CLAUDE.md\n\nuser note\n").unwrap();
        let section = illu_agent_section("mcp__illu__");
        write_md_section(dir.path(), "CLAUDE.md", "# CLAUDE.md", &section).unwrap();
        let content = std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        assert!(content.contains("user note"));
        assert!(content.contains(ILLU_SECTION_START));
    }
}
