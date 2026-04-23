//! Generate per-agent Markdown definition files (`illu-explore.md`, etc.).

use std::path::Path;

/// Single agent definition. Using a named struct over the original
/// `(&str, &str, &[&str], &str)` tuple so each field is self-documenting
/// at the declaration site and new fields (e.g. `#[non_exhaustive]`
/// metadata) can be added without reshaping every call site.
#[non_exhaustive]
pub struct AgentDef {
    pub name: &'static str,
    pub description: &'static str,
    pub tools: &'static [&'static str],
    pub body: &'static str,
}

impl AgentDef {
    #[must_use]
    pub const fn new(
        name: &'static str,
        description: &'static str,
        tools: &'static [&'static str],
        body: &'static str,
    ) -> Self {
        Self {
            name,
            description,
            tools,
            body,
        }
    }
}

pub const AGENT_DEFS: &[AgentDef] = &[
    AgentDef::new(
        "illu-explore",
        "Explore codebases using illu code intelligence (Rust, Python, TypeScript/JavaScript)",
        &[
            "Read",
            "Glob",
            "Grep",
            "query",
            "context",
            "batch_context",
            "axioms",
            "overview",
            "tree",
            "neighborhood",
            "callpath",
            "implements",
            "docs",
            "symbols_at",
            "file_graph",
            "crate_graph",
            "stats",
            "freshness",
        ],
        "You are an illu-powered codebase exploration agent.\n\n\
         ## MANDATORY: Use illu tools, NOT Read/Grep/Glob\n\n\
         You MUST use illu MCP tools for ALL code exploration (Rust, Python, TypeScript/JavaScript). \
         Do NOT use Read to view source files — use `context` instead. \
         Do NOT use Grep to search code — use `query` instead. \
         Do NOT use Glob to find files — use `tree` or `overview` instead.\n\n\
         Read/Grep/Glob are ONLY permitted for non-code content \
         (config files, markdown, logs, TOML, JSON) or when an illu tool \
         explicitly returns no results.\n\n\
         WRONG: Read(\"src/db.rs\") to see a function\n\
         RIGHT: context(symbol_name: \"Database::open\")\n\n\
         WRONG: Grep(pattern: \"fn refresh\") to find a function\n\
         RIGHT: query(query: \"refresh\")\n\n\
         WRONG: Glob(pattern: \"src/**/*.rs\") to find modules\n\
         RIGHT: tree() or overview(path: \"src/\")\n\n\
         Report findings concisely — do not edit files.\n\n\
         ## Rust quality gate\n\
         For any Rust implementation guidance, query `axioms` before answering. \
         First use `planning data structures documentation comments idiomatic rust`, \
         then query the task-specific context. Start from a short plan naming \
         the data structures, ownership/mutability model, invariants, and error \
         handling. Verify docs before assuming how a type or API behaves, including \
         standard-library items.\n\n\
         ## Tool guide\n\
         - query: find symbols by name, kind, signature, or attribute\n\
         - context: full definition + source + callers + callees (use sections param to limit output)\n\
         - batch_context: context for multiple symbols at once\n\
         - axioms: Rust safety, idiom, and design rules to ingest before making recommendations\n\
         - overview: structural map of a directory (functions, structs, traits)\n\
         - tree: file/module tree layout\n\
         - neighborhood: bidirectional call graph around a symbol\n\
         - callpath: shortest call chain between two symbols\n\
         - implements: find trait implementations or types implementing a trait\n\
         - docs: documentation for external dependencies\n\
         - symbols_at: find symbols at a specific file:line\n\
         - file_graph: file-level dependency visualization\n\
         - crate_graph: workspace crate dependency graph\n\
         - stats: codebase statistics dashboard\n\
         - freshness: check if the index is current\n\n\
         ## Workflow\n\
         query to locate → context to understand → \
         neighborhood/callpath to trace flow. \
         Use sections: [\"source\", \"callers\"] to save tokens. \
         Use exclude_tests: true to focus on production code.",
    ),
    AgentDef::new(
        "illu-review",
        "Review code changes using illu code intelligence (Rust, Python, TypeScript/JavaScript)",
        &[
            "Read",
            "Glob",
            "Grep",
            "query",
            "axioms",
            "impact",
            "diff_impact",
            "test_impact",
            "boundary",
            "references",
            "blame",
            "history",
            "context",
            "docs",
            "doc_coverage",
        ],
        "You are an illu-powered code review agent.\n\n\
         ## MANDATORY: Use illu tools, NOT Read/Grep/Glob\n\n\
         You MUST use illu MCP tools for ALL code analysis (Rust, Python, TypeScript/JavaScript). \
         Do NOT use Read to view source files — use `context` instead. \
         Do NOT use Grep to search code — use `query` instead. \
         Do NOT use Glob to find files — use `tree` or `overview` instead.\n\n\
         Read/Grep/Glob are ONLY permitted for non-code content \
         (config files, markdown, logs, TOML, JSON) or when an illu tool \
         explicitly returns no results.\n\n\
         WRONG: Read(\"src/db.rs\") to see a function\n\
         RIGHT: context(symbol_name: \"Database::open\")\n\n\
         WRONG: Grep(pattern: \"fn refresh\") to find a function\n\
         RIGHT: query(query: \"refresh\")\n\n\
         Report findings concisely — do not edit files.\n\n\
         ## Rust quality gate\n\
         Before recommending Rust changes, query `axioms` with \
         `planning data structures documentation comments idiomatic rust` and then \
         with the concrete review topic. Make your review from a short design plan: \
         data structures, ownership/mutability, invariants, and failure handling. \
         Verify docs before assuming the behavior of any non-trivial type or API.\n\n\
         ## Tool guide\n\
         - query: find symbols by name to start analysis\n\
         - axioms: Rust safety, idiom, and design rules to ground review feedback\n\
         - context: full definition + callers + callees (use sections param to limit output)\n\
         - impact: see all downstream dependents of a symbol before changes\n\
         - diff_impact: analyze impact of git diff changes (use compact: true for large diffs)\n\
         - test_impact: find which tests break when changing a symbol\n\
         - boundary: classify symbols as public API vs internal (safe to refactor)\n\
         - references: unified view of all references (callers, type usage, trait impls)\n\
         - blame: git blame on a symbol's line range\n\
         - history: git commit history for a symbol (use show_diff: true for code changes)\n\
         - docs: documentation for external dependencies mentioned in the change\n\
         - doc_coverage: find symbols missing doc comments\n\n\
         ## Workflow\n\
         axioms before Rust recommendations → diff_impact for changed symbols → impact on key symbols → \
         test_impact to verify coverage → boundary to check API surface. \
         Use exclude_tests: true to focus on production callers.",
    ),
    AgentDef::new(
        "illu-refactor",
        "Plan refactoring using illu code intelligence (Rust, Python, TypeScript/JavaScript)",
        &[
            "Read",
            "Glob",
            "Grep",
            "axioms",
            "rename_plan",
            "unused",
            "orphaned",
            "similar",
            "type_usage",
            "hotspots",
            "context",
            "impact",
            "references",
            "boundary",
            "docs",
        ],
        "You are an illu-powered refactoring agent.\n\n\
         ## MANDATORY: Use illu tools, NOT Read/Grep/Glob\n\n\
         You MUST use illu MCP tools for ALL code analysis (Rust, Python, TypeScript/JavaScript). \
         Do NOT use Read to view source files — use `context` instead. \
         Do NOT use Grep to search code — use `query` instead. \
         Do NOT use Glob to find files — use `tree` or `overview` instead.\n\n\
         Read/Grep/Glob are ONLY permitted for non-code content \
         (config files, markdown, logs, TOML, JSON) or when an illu tool \
         explicitly returns no results.\n\n\
         WRONG: Read(\"src/db.rs\") to see a function\n\
         RIGHT: context(symbol_name: \"Database::open\")\n\n\
         WRONG: Grep(pattern: \"fn refresh\") to find a function\n\
         RIGHT: query(query: \"refresh\")\n\n\
         Report findings concisely — do not edit files.\n\n\
         ## Rust quality gate\n\
         Before proposing Rust refactors, query `axioms` with \
         `planning data structures documentation comments idiomatic rust` and then \
         with the task-specific refactor context. Start from a short plan naming \
         the target data structures, ownership/mutability changes, invariants, and \
         error behavior. Verify docs before assuming the semantics of any type or API.\n\n\
         ## Tool guide\n\
         - axioms: Rust safety, idiom, and design rules to ingest before planning changes\n\
         - rename_plan: preview all locations affected by renaming a symbol\n\
         - unused: find symbols with zero incoming references\n\
         - orphaned: find symbols with no callers AND no test coverage (safe to remove)\n\
         - similar: find structurally similar symbols (candidates for dedup)\n\
         - type_usage: find where a type appears in signatures and struct fields\n\
         - hotspots: identify high-complexity and high-coupling symbols\n\
         - context: full definition + callers + callees (use sections param to limit output)\n\
         - impact: see all downstream dependents before changing a symbol\n\
         - references: unified view of all references to a symbol\n\
         - boundary: classify symbols as public API vs internal\n\
         - docs: documentation for external dependencies involved in the refactor\n\n\
         ## Workflow\n\
         axioms before Rust planning → hotspots to find targets → unused/orphaned for dead code → \
         impact before any change → rename_plan to preview renames → \
         boundary to verify API surface. \
         Use exclude_tests: true to focus on production code.",
    ),
];

pub const BUILTIN_TOOLS: &[&str] = &["Read", "Glob", "Grep"];

/// Write one Markdown file per entry in [`AGENT_DEFS`] into `agents_dir`.
///
/// Each file gets YAML frontmatter (`name`, `description`, `tools`) followed by
/// the agent's body text. Tool names are rewritten with `tool_prefix` unless
/// they are built-in (see [`BUILTIN_TOOLS`]).
pub fn generate_agent_files(
    agents_dir: &Path,
    tool_prefix: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::fmt::Write;

    std::fs::create_dir_all(agents_dir)?;

    for agent in AGENT_DEFS {
        let mut tools_yaml = String::new();
        for tool in agent.tools {
            let full_name = if BUILTIN_TOOLS.contains(tool) {
                (*tool).to_string()
            } else {
                format!("{tool_prefix}{tool}")
            };
            writeln!(tools_yaml, "  - {full_name}")?;
        }
        let AgentDef {
            name,
            description,
            body,
            ..
        } = agent;
        let content = format!(
            "---\nname: {name}\n\
             description: {description}\n\
             tools:\n{tools_yaml}\
             ---\n\n{body}\n"
        );
        std::fs::write(agents_dir.join(format!("{name}.md")), content)?;
    }

    Ok(())
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_generate_agent_files_creates_three_files() {
        let dir = tempdir().unwrap();
        let agents_dir = dir.path().join("agents");

        // Test Claude variant
        generate_agent_files(&agents_dir, "mcp__illu__").unwrap();

        let explore = std::fs::read_to_string(agents_dir.join("illu-explore.md")).unwrap();
        let review = std::fs::read_to_string(agents_dir.join("illu-review.md")).unwrap();
        let refactor = std::fs::read_to_string(agents_dir.join("illu-refactor.md")).unwrap();

        // All three exist and have frontmatter
        assert!(explore.starts_with("---"));
        assert!(review.starts_with("---"));
        assert!(refactor.starts_with("---"));

        // Each has the correct name
        assert!(explore.contains("name: illu-explore"));
        assert!(review.contains("name: illu-review"));
        assert!(refactor.contains("name: illu-refactor"));

        // None allow Edit, Write, or Bash in tools section
        for content in [&explore, &review, &refactor] {
            let tool_entries: Vec<&str> =
                content.lines().filter(|l| l.starts_with("  - ")).collect();
            assert!(!tool_entries.is_empty(), "should have tool entries");
            for entry in &tool_entries {
                assert!(!entry.contains("Edit"), "tools should not contain Edit");
                assert!(!entry.contains("Write"), "tools should not contain Write");
                assert!(!entry.contains("Bash"), "tools should not contain Bash");
            }
        }

        // Frontmatter structure validation
        for content in [&explore, &review, &refactor] {
            assert!(
                content.matches("---").count() >= 2,
                "should have opening and closing frontmatter"
            );
        }

        // Each has illu tools with correct prefix
        assert!(explore.contains("mcp__illu__query"));
        assert!(explore.contains("mcp__illu__axioms"));
        assert!(review.contains("mcp__illu__impact"));
        assert!(review.contains("mcp__illu__axioms"));
        assert!(refactor.contains("mcp__illu__rename_plan"));
        assert!(refactor.contains("mcp__illu__axioms"));
        assert!(
            refactor.contains("planning data structures documentation comments idiomatic rust")
        );

        // tools: key must be a YAML array (no inline value on same line)
        for content in [&explore, &review, &refactor] {
            assert!(
                content.contains("\ntools:\n"),
                "tools must be a YAML array (no inline value)"
            );
        }

        // Test Gemini variant
        let gemini_dir = dir.path().join("gemini_agents");
        generate_agent_files(&gemini_dir, "mcp_illu_").unwrap();

        let explore_g = std::fs::read_to_string(gemini_dir.join("illu-explore.md")).unwrap();
        assert!(explore_g.contains("mcp_illu_query"));
        assert!(!explore_g.contains("mcp__illu__query"));
    }
}
