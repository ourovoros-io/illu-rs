use clap::{Parser, Subcommand};
use illu_rs::db::Database;
use illu_rs::indexer::{IndexConfig, index_repo};
use illu_rs::server::IlluServer;
use illu_rs::server::tools::{
    context::handle_context, docs::handle_docs, impact::handle_impact, query::handle_query,
    tree::handle_tree,
};
use rmcp::ServiceExt;
use rmcp::transport::stdio;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "illu",
    about = "Code intelligence for Rust, Python, and TypeScript/JavaScript"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Path to the repo
    #[arg(long, short, global = true, default_value = ".")]
    repo: PathBuf,

    /// Disable rust-analyzer integration (RA tools will not be available)
    #[arg(long, global = true)]
    no_ra: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Index the repo and start the MCP server
    Serve,
    /// Search for symbols, docs, or files
    Query {
        /// Search term
        search: String,
        /// Scope: all, symbols, docs, files
        #[arg(short, long, default_value = "all")]
        scope: String,
        /// Filter by symbol kind: function, struct, enum, trait, etc.
        #[arg(short, long)]
        kind: Option<String>,
    },
    /// Get full context for a symbol
    Context {
        /// Symbol name
        symbol: String,
    },
    /// Show what depends on a symbol
    Impact {
        /// Symbol name
        symbol: String,
    },
    /// Search dependency documentation
    Docs {
        /// Dependency name
        dep: String,
        /// Filter by topic
        #[arg(short, long)]
        topic: Option<String>,
    },
    /// Show file/module tree with symbol counts
    Tree {
        /// Path prefix
        #[arg(default_value = "src/")]
        path: String,
    },
    /// Show web dashboard with indexing status and health
    #[cfg(feature = "dashboard")]
    Dashboard {
        /// Port to run the dashboard on
        #[arg(short, long, default_value_t = 7879)]
        port: u16,
    },
    /// Set up illu in a repo (configures Claude Code + Gemini CLI, builds index)
    Init,
    /// Install illu globally (configures Claude Code + Gemini CLI for all repos)
    Install,
    /// Manage cross-repo registry
    Repo {
        #[command(subcommand)]
        command: RepoCommand,
    },
}

#[derive(Subcommand)]
enum RepoCommand {
    /// List all currently indexed repositories
    List,
    /// Remove a repository from the registry and its index database
    Remove {
        /// Name or path of the repository to remove
        identifier: String,
    },
}

fn write_mcp_config_to(
    config_path: &Path,
    args: &[&str],
) -> Result<(), Box<dyn std::error::Error>> {
    let binary = "illu-rs";

    let illu_entry = serde_json::json!({
        "command": binary,
        "args": args,
        "env": { "RUST_LOG": "warn" }
    });

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut config: serde_json::Value = std::fs::read_to_string(config_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({"mcpServers": {}}));

    config["mcpServers"]["illu"] = illu_entry;

    std::fs::write(config_path, serde_json::to_string_pretty(&config)?)?;
    tracing::info!("Wrote MCP config to {}", config_path.display());
    Ok(())
}

fn write_mcp_server_config(config_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    write_mcp_config_to(config_path, &["serve"])
}

fn write_mcp_config(repo_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    write_mcp_server_config(&repo_path.join(".mcp.json"))
}

fn write_gemini_config(repo_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    write_mcp_server_config(&repo_path.join(".gemini/settings.json"))
}

const ILLU_SECTION_START: &str = "<!-- illu:start -->";
const ILLU_SECTION_END: &str = "<!-- illu:end -->";

fn illu_agent_section(tool_prefix: &str) -> String {
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

/// Agent definitions: (name, description, tool short names, body text).
const AGENT_DEFS: &[(&str, &str, &[&str], &str)] = &[
    (
        "illu-explore",
        "Explore codebases using illu code intelligence (Rust, Python, TypeScript/JavaScript)",
        &[
            "Read",
            "Glob",
            "Grep",
            "query",
            "context",
            "batch_context",
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
         ## Tool guide\n\
         - query: find symbols by name, kind, signature, or attribute\n\
         - context: full definition + source + callers + callees (use sections param to limit output)\n\
         - batch_context: context for multiple symbols at once\n\
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
    (
        "illu-review",
        "Review code changes using illu code intelligence (Rust, Python, TypeScript/JavaScript)",
        &[
            "Read",
            "Glob",
            "Grep",
            "query",
            "impact",
            "diff_impact",
            "test_impact",
            "boundary",
            "references",
            "blame",
            "history",
            "context",
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
         ## Tool guide\n\
         - query: find symbols by name to start analysis\n\
         - context: full definition + callers + callees (use sections param to limit output)\n\
         - impact: see all downstream dependents of a symbol before changes\n\
         - diff_impact: analyze impact of git diff changes (use compact: true for large diffs)\n\
         - test_impact: find which tests break when changing a symbol\n\
         - boundary: classify symbols as public API vs internal (safe to refactor)\n\
         - references: unified view of all references (callers, type usage, trait impls)\n\
         - blame: git blame on a symbol's line range\n\
         - history: git commit history for a symbol (use show_diff: true for code changes)\n\
         - doc_coverage: find symbols missing doc comments\n\n\
         ## Workflow\n\
         diff_impact for changed symbols → impact on key symbols → \
         test_impact to verify coverage → boundary to check API surface. \
         Use exclude_tests: true to focus on production callers.",
    ),
    (
        "illu-refactor",
        "Plan refactoring using illu code intelligence (Rust, Python, TypeScript/JavaScript)",
        &[
            "Read",
            "Glob",
            "Grep",
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
         ## Tool guide\n\
         - rename_plan: preview all locations affected by renaming a symbol\n\
         - unused: find symbols with zero incoming references\n\
         - orphaned: find symbols with no callers AND no test coverage (safe to remove)\n\
         - similar: find structurally similar symbols (candidates for dedup)\n\
         - type_usage: find where a type appears in signatures and struct fields\n\
         - hotspots: identify high-complexity and high-coupling symbols\n\
         - context: full definition + callers + callees (use sections param to limit output)\n\
         - impact: see all downstream dependents before changing a symbol\n\
         - references: unified view of all references to a symbol\n\
         - boundary: classify symbols as public API vs internal\n\n\
         ## Workflow\n\
         hotspots to find targets → unused/orphaned for dead code → \
         impact before any change → rename_plan to preview renames → \
         boundary to verify API surface. \
         Use exclude_tests: true to focus on production code.",
    ),
];

const BUILTIN_TOOLS: &[&str] = &["Read", "Glob", "Grep"];

fn generate_agent_files(
    agents_dir: &Path,
    tool_prefix: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::fmt::Write;

    std::fs::create_dir_all(agents_dir)?;

    for (name, description, tools, body) in AGENT_DEFS {
        let mut tools_yaml = String::new();
        for tool in *tools {
            let full_name = if BUILTIN_TOOLS.contains(tool) {
                (*tool).to_string()
            } else {
                format!("{tool_prefix}{tool}")
            };
            writeln!(tools_yaml, "  - {full_name}")?;
        }
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

fn write_md_section(
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

fn write_claude_md_section(repo_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let section = illu_agent_section("mcp__illu__");
    write_md_section(repo_path, "CLAUDE.md", "# CLAUDE.md", &section)
}

fn write_gemini_md_section(repo_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let section = illu_agent_section("mcp_illu_");
    write_md_section(repo_path, "GEMINI.md", "# GEMINI.md", &section)
}

fn open_or_index(repo_path: &Path) -> Result<Database, Box<dyn std::error::Error>> {
    illu_rs::status::init(repo_path);
    let db_path = repo_path.join(".illu/index.db");
    if db_path.exists() {
        let db = Database::open(&db_path)?;
        let config = IndexConfig {
            repo_path: repo_path.to_path_buf(),
        };
        let refreshed = illu_rs::indexer::refresh_index(&db, &config)?;
        if refreshed > 0 {
            tracing::info!("Refreshed {refreshed} file(s)");
        }
        return Ok(db);
    }
    tracing::info!("No index found — indexing {}", repo_path.display());
    ensure_indexed(repo_path)
}

fn ensure_indexed(repo_path: &Path) -> Result<Database, Box<dyn std::error::Error>> {
    let db_dir = repo_path.join(".illu");
    std::fs::create_dir_all(&db_dir)?;
    let db_path = db_dir.join("index.db");
    let db = Database::open(&db_path)?;

    tracing::info!("Indexing {}", repo_path.display());
    let config = IndexConfig {
        repo_path: repo_path.to_path_buf(),
    };
    index_repo(&db, &config)?;
    tracing::info!("Indexing complete");
    Ok(db)
}

#[expect(clippy::print_stdout, reason = "CLI output")]
fn print_result(result: &str) {
    println!("{result}");
}

#[expect(clippy::print_stdout, reason = "CLI output")]
fn init_repo(repo_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let repo_path = repo_path.canonicalize()?;

    // Verify it's a supported project
    let has_cargo = repo_path.join("Cargo.toml").exists();
    let has_ts =
        repo_path.join("tsconfig.json").exists() || repo_path.join("package.json").exists();
    let has_python = illu_rs::indexer::has_python_project(&repo_path);
    if !has_cargo && !has_ts && !has_python {
        return Err(format!(
            "No Cargo.toml, tsconfig.json, package.json, or Python project found in {}",
            repo_path.display()
        )
        .into());
    }

    println!("Setting up illu in {}", repo_path.display());

    // 1. Write MCP configs for both Claude Code and Gemini CLI
    write_mcp_config(&repo_path)?;
    println!("  wrote .mcp.json (Claude Code)");
    write_gemini_config(&repo_path)?;
    println!("  wrote .gemini/settings.json (Gemini CLI)");

    // 2. Update instruction files for both
    write_claude_md_section(&repo_path)?;
    println!("  updated CLAUDE.md");
    write_gemini_md_section(&repo_path)?;
    println!("  updated GEMINI.md");

    // 3. Generate agent definition files
    generate_agent_files(&repo_path.join(".claude/agents"), "mcp__illu__")?;
    println!("  wrote .claude/agents/ (Claude agent files)");
    generate_agent_files(&repo_path.join(".gemini/agents"), "mcp_illu_")?;
    println!("  wrote .gemini/agents/ (Gemini agent files)");

    // 4. Auto-allow illu tools in project-level Claude settings
    let local_settings = repo_path.join(".claude/settings.local.json");
    ensure_tools_allowed(&local_settings)?;
    println!("  auto-allowed illu tools in .claude/settings.local.json");

    // 5. Build initial index
    println!("  indexing...");
    illu_rs::status::init(&repo_path);
    ensure_indexed(&repo_path)?;
    println!("  index built");

    // 6. Add .illu/ to .gitignore if not already there
    if ensure_gitignore(&repo_path)? {
        println!("  updated .gitignore with illu entries");
    }

    println!("\nDone. Start Claude Code or Gemini CLI in this repo — illu will run automatically.");
    Ok(())
}

fn append_gitignore_entry(path: &Path) -> Result<bool, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let entries = [
        ".illu/",
        ".claude/agents/illu-*.md",
        ".gemini/agents/illu-*.md",
    ];
    let mut added = false;
    let mut out = content;
    for entry in entries {
        if !out.lines().any(|l| {
            let trimmed = l.trim();
            trimmed == entry || trimmed == entry.trim_end_matches('/')
        }) {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(entry);
            out.push('\n');
            added = true;
        }
    }
    if added {
        std::fs::write(path, out)?;
    }
    Ok(added)
}

fn ensure_gitignore(repo_path: &Path) -> Result<bool, Box<dyn std::error::Error>> {
    append_gitignore_entry(&repo_path.join(".gitignore"))
}

fn register_repo(repo_path: &Path) {
    let Ok(registry_path) = illu_rs::registry::Registry::default_path() else {
        return;
    };
    let Ok(mut registry) = illu_rs::registry::Registry::load(&registry_path) else {
        return;
    };

    let name = repo_path
        .file_name()
        .map_or_else(|| "unknown".into(), |n| n.to_string_lossy().into_owned());

    let git_remote = illu_rs::git::git_remote_url(repo_path);
    let git_common_dir =
        illu_rs::git::git_common_dir(repo_path).unwrap_or_else(|_| repo_path.join(".git"));

    let now = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map_or_else(|_| "0".into(), |d| d.as_secs().to_string());

    registry.register(illu_rs::registry::RepoEntry {
        name,
        path: repo_path.to_path_buf(),
        git_remote,
        git_common_dir,
        last_indexed: now,
    });
    registry.prune();
    let _ = registry.save();
}

fn write_global_mcp_config(config_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    write_mcp_config_to(config_path, &["serve"])
}

/// Ensure all illu MCP tools are auto-allowed in Claude settings.
fn ensure_tools_allowed(config_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut config: serde_json::Value = std::fs::read_to_string(config_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));

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

const STATUSLINE_SH: &str = include_str!("../assets/statusline.sh");

#[expect(clippy::print_stdout, reason = "CLI output")]
fn install_statusline(home: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // Write the script to ~/.illu/statusline.sh
    let illu_dir = home.join(".illu");
    std::fs::create_dir_all(&illu_dir)?;
    let script_path = illu_dir.join("statusline.sh");
    std::fs::write(&script_path, STATUSLINE_SH)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))?;
    }

    // Check if Claude settings already has a statusLine entry
    let claude_settings = home.join(".claude/settings.json");
    let config: serde_json::Value = std::fs::read_to_string(&claude_settings)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));

    if config.get("statusLine").is_some() {
        println!(
            "  statusLine already configured in {} — skipping",
            claude_settings.display()
        );
        println!("  To use illu's statusline, set in ~/.claude/settings.json:");
        println!(
            "    \"statusLine\": {{ \"type\": \"command\", \"command\": \"{}\" }}",
            script_path.display()
        );
        return Ok(());
    }

    // Write the statusLine entry
    let mut config = config;
    config["statusLine"] = serde_json::json!({
        "type": "command",
        "command": script_path.to_string_lossy()
    });
    std::fs::write(&claude_settings, serde_json::to_string_pretty(&config)?)?;
    println!("  statusline installed at {}", script_path.display());

    Ok(())
}

fn ensure_global_gitignore(home: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let gitignore_path = home.join(".config/git/ignore");
    if let Some(parent) = gitignore_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    append_gitignore_entry(&gitignore_path)?;
    Ok(())
}

#[expect(clippy::print_stdout, reason = "CLI output")]
fn install_global() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::var("HOME").map_err(|_| "HOME environment variable not set")?;
    let home = PathBuf::from(home);

    println!("Installing illu globally...");

    let claude_settings = home.join(".claude/settings.json");
    write_global_mcp_config(&claude_settings)?;
    ensure_tools_allowed(&claude_settings)?;
    println!("  wrote {}", claude_settings.display());

    let gemini_settings = home.join(".gemini/settings.json");
    write_global_mcp_config(&gemini_settings)?;
    println!("  wrote {}", gemini_settings.display());

    write_md_section(
        &home,
        ".claude/CLAUDE.md",
        "# CLAUDE.md",
        &illu_agent_section("mcp__illu__"),
    )?;
    println!("  updated {}", home.join(".claude/CLAUDE.md").display());

    write_md_section(
        &home,
        ".gemini/GEMINI.md",
        "# GEMINI.md",
        &illu_agent_section("mcp_illu_"),
    )?;
    println!("  updated {}", home.join(".gemini/GEMINI.md").display());

    generate_agent_files(&home.join(".claude/agents"), "mcp__illu__")?;
    println!("  wrote {}", home.join(".claude/agents/").display());
    generate_agent_files(&home.join(".gemini/agents"), "mcp_illu_")?;
    println!("  wrote {}", home.join(".gemini/agents/").display());

    install_statusline(&home)?;

    ensure_global_gitignore(&home)?;

    println!("\nDone. illu will auto-start in any Rust, TypeScript/JavaScript, or Python project.");
    Ok(())
}

/// Poll `git rev-parse HEAD` and refresh the index when HEAD changes.
/// Runs as a background task — detects `git pull`, `git checkout`, etc.
async fn head_watcher(db: std::sync::Arc<std::sync::Mutex<Database>>, config: IndexConfig) {
    const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

    let repo_path = config.repo_path.clone();
    let mut last_head = git_head(&repo_path);

    loop {
        tokio::time::sleep(POLL_INTERVAL).await;

        let current = git_head(&repo_path);
        if current == last_head {
            continue;
        }

        tracing::info!(
            old = last_head.as_deref().unwrap_or("unknown"),
            new = current.as_deref().unwrap_or("unknown"),
            "HEAD changed — refreshing index"
        );

        let db = db.clone();
        let config = config.clone();
        let result = tokio::task::spawn_blocking(move || {
            let Ok(db) = db.lock() else {
                tracing::warn!("Background refresh: could not acquire DB lock");
                return;
            };
            illu_rs::status::set("refreshing ▸ background");
            match illu_rs::indexer::refresh_index(&db, &config) {
                Ok(count) if count > 0 => {
                    tracing::info!(count, "Background refresh: re-indexed files");
                }
                Ok(_) => {
                    tracing::info!(
                        "Background refresh: HEAD updated, \
                         no source changes"
                    );
                }
                Err(e) => {
                    tracing::warn!("Background refresh failed: {e}");
                }
            }
            illu_rs::status::set(illu_rs::status::READY);
        })
        .await;

        if let Err(e) = result {
            tracing::warn!("Background refresh task panicked: {e}");
        }

        last_head = git_head(&repo_path);
    }
}

fn git_head(repo_path: &Path) -> Option<String> {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_path)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

#[expect(clippy::print_stdout, reason = "CLI output")]
fn handle_repo_command(command: RepoCommand) -> Result<(), Box<dyn std::error::Error>> {
    let registry_path = illu_rs::registry::Registry::default_path()
        .map_err(|e| format!("Could not get default registry path: {e}"))?;
    let mut registry = illu_rs::registry::Registry::load(&registry_path)
        .map_err(|e| format!("Could not load registry: {e}"))?;

    match command {
        RepoCommand::List => {
            registry.prune();
            if let Err(e) = registry.save() {
                println!("Warning: Could not save registry: {e}");
            }

            if registry.repos.is_empty() {
                println!("No repositories currently registered.");
                return Ok(());
            }

            println!("{:<20} | {:<50} | Last Indexed", "Name", "Path");
            println!("{:-<20}-+-{:-<50}-+-{:-<20}", "", "", "");

            let now = std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);

            for repo in &registry.repos {
                let ts: u64 = repo.last_indexed.parse().unwrap_or(0);
                let diff = now.saturating_sub(ts);
                let time_str = if ts == 0 {
                    "Unknown".to_string()
                } else if diff < 60 {
                    "Just now".to_string()
                } else if diff < 3600 {
                    let m = diff / 60;
                    format!("{m} minute{} ago", if m == 1 { "" } else { "s" })
                } else if diff < 86400 {
                    let h = diff / 3600;
                    format!("{h} hour{} ago", if h == 1 { "" } else { "s" })
                } else {
                    let d = diff / 86400;
                    format!("{d} day{} ago", if d == 1 { "" } else { "s" })
                };

                let name_chars = repo.name.chars().count();
                let display_name = if name_chars > 20 {
                    let prefix: String = repo.name.chars().take(17).collect();
                    format!("{prefix}...")
                } else {
                    repo.name.clone()
                };

                let path_str = repo.path.to_string_lossy();
                let chars_count = path_str.chars().count();
                let display_path = if chars_count > 50 {
                    let suffix: String = path_str
                        .chars()
                        .skip(chars_count.saturating_sub(47))
                        .collect();
                    format!("...{suffix}")
                } else {
                    path_str.into_owned()
                };

                println!("{display_name:<20} | {display_path:<50} | {time_str}");
            }
        }
        RepoCommand::Remove { identifier } => {
            let initial_len = registry.repos.len();
            let mut removed_paths = Vec::new();
            let target_path = Path::new(&identifier);

            registry.repos.retain(|r| {
                let matches = r.name == identifier || r.path == target_path;
                if matches {
                    removed_paths.push(r.path.clone());
                }
                !matches
            });

            if registry.repos.len() == initial_len {
                println!("No repository found matching '{identifier}'.");
                return Ok(());
            }

            if let Err(e) = registry.save() {
                println!("Warning: Could not save registry: {e}");
            }

            for path in removed_paths {
                let db_dir = path.join(".illu");
                if db_dir.exists() {
                    if let Err(e) = std::fs::remove_dir_all(&db_dir) {
                        println!(
                            "Removed '{identifier}' from registry, but failed to delete index at {}: {e}",
                            db_dir.display()
                        );
                    } else {
                        println!(
                            "Removed '{identifier}' from registry and deleted index at {}.",
                            db_dir.display()
                        );
                    }
                } else {
                    println!(
                        "Removed '{identifier}' from registry. (No index found at {})",
                        db_dir.display()
                    );
                }
            }
        }
    }
    Ok(())
}

#[tokio::main]
#[expect(clippy::too_many_lines, reason = "CLI dispatch with many subcommands")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let detect_from_cwd = || -> PathBuf {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        match illu_rs::git::detect_repo_root(&cwd) {
            Ok(git_root) => illu_rs::git::detect_cargo_root(&cwd, &git_root),
            Err(_) => cwd,
        }
    };
    let repo_path = if cli.repo == Path::new(".") {
        detect_from_cwd()
    } else if cli.repo.exists() {
        cli.repo.clone()
    } else {
        tracing::warn!(
            path = %cli.repo.display(),
            "Specified --repo path does not exist, detecting from CWD"
        );
        detect_from_cwd()
    };
    let repo_path = &repo_path;

    match cli.command {
        None | Some(Command::Serve) => {
            tracing::info!(repo = %repo_path.display(), "Starting illu server");
            let has_cargo = repo_path.join("Cargo.toml").exists();
            let has_ts =
                repo_path.join("tsconfig.json").exists() || repo_path.join("package.json").exists();
            let has_python = illu_rs::indexer::has_python_project(repo_path);
            let has_project = has_cargo || has_ts || has_python;

            let (db, config) = if has_project {
                let db_dir = repo_path.join(".illu");
                std::fs::create_dir_all(&db_dir)?;
                illu_rs::status::init(repo_path);
                illu_rs::status::set("starting");
                write_mcp_config(repo_path)?;
                write_claude_md_section(repo_path)?;
                write_gemini_config(repo_path)?;
                write_gemini_md_section(repo_path)?;
                generate_agent_files(&repo_path.join(".claude/agents"), "mcp__illu__")?;
                generate_agent_files(&repo_path.join(".gemini/agents"), "mcp_illu_")?;
                let local_settings = repo_path.join(".claude/settings.local.json");
                if let Err(e) = ensure_tools_allowed(&local_settings) {
                    tracing::warn!("Could not auto-allow tools: {e}");
                }

                let config = IndexConfig {
                    repo_path: repo_path.clone(),
                };

                let db_path = db_dir.join("index.db");
                tracing::debug!(path = %db_path.display(), "Opening database");
                let db = Database::open(&db_path)?;

                illu_rs::status::set("indexing");
                let refreshed = illu_rs::indexer::refresh_index(&db, &config)?;
                if refreshed > 0 {
                    tracing::info!(count = refreshed, "Refreshed changed files");
                }

                (db, config)
            } else {
                tracing::warn!(
                    "No Cargo.toml/tsconfig.json/pyproject.toml — starting with empty index (cross-repo tools only)"
                );
                let db = Database::open_in_memory()?;
                let config = IndexConfig {
                    repo_path: repo_path.clone(),
                };
                (db, config)
            };

            if has_project {
                register_repo(repo_path);
            }

            let registry = {
                let path = illu_rs::registry::Registry::default_path()
                    .unwrap_or_else(|_| repo_path.join(".illu/registry.toml"));
                illu_rs::registry::Registry::load(&path)
                    .unwrap_or_else(|_| illu_rs::registry::Registry::empty())
            };

            // Check for pending docs before handing DB to the server
            let pending_docs = if has_cargo {
                illu_rs::indexer::docs::pending_docs(&db)?
            } else {
                Vec::new()
            };

            let mut server = IlluServer::new(db, config.clone(), registry);

            // Start rust-analyzer in the background if available
            if !cli.no_ra && has_cargo {
                match illu_rs::ra::RaClient::start(repo_path).await {
                    Ok(ra) => {
                        let ra = std::sync::Arc::new(ra);
                        server = server.with_ra(ra.clone());
                        tokio::spawn(async move {
                            match ra.wait_for_ready(std::time::Duration::from_secs(120)).await {
                                Ok(()) => tracing::info!("rust-analyzer is ready"),
                                Err(e) => {
                                    tracing::warn!("rust-analyzer readiness timeout: {e}");
                                }
                            }
                        });
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Could not start rust-analyzer: {e}. RA tools will be unavailable."
                        );
                    }
                }
            }

            let db_arc = server.db();
            let watcher_db = server.db();
            let transport = stdio();
            tracing::info!("MCP transport ready, starting handshake");
            if has_cargo {
                illu_rs::status::set(illu_rs::status::READY);
            }
            let service = server.serve(transport).await?;
            tracing::info!("MCP server initialized, waiting for requests");

            // Watch for HEAD changes (git pull) and refresh index
            if has_project {
                let watcher_config = config.clone();
                tokio::spawn(async move {
                    head_watcher(watcher_db, watcher_config).await;
                });
            }

            // Fetch docs in background — server is already accepting requests
            if !pending_docs.is_empty() {
                let db = db_arc;
                let repo_path = config.repo_path.clone();
                tokio::spawn(async move {
                    let total = pending_docs.len();
                    tracing::info!(count = total, "Fetching docs in background");
                    illu_rs::status::set(&format!("fetching docs ▸ 0/{total}"));
                    let fetched =
                        illu_rs::indexer::docs::fetch_docs(&pending_docs, &repo_path).await;
                    if !fetched.is_empty() {
                        let Ok(db) = db.lock() else { return };
                        let count = fetched.len();
                        let _ = illu_rs::indexer::docs::store_fetched_docs(&db, &fetched);
                        tracing::info!(count, "Stored dependency docs");
                    }
                    illu_rs::status::set(illu_rs::status::READY);
                });
            }

            let shutdown = async {
                #[cfg(unix)]
                {
                    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    {
                        Ok(mut sigterm) => {
                            tokio::select! {
                                _ = tokio::signal::ctrl_c() => {},
                                _ = sigterm.recv() => {},
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to install SIGTERM handler: {e}");
                            let _ = tokio::signal::ctrl_c().await;
                        }
                    }
                }
                #[cfg(not(unix))]
                {
                    let _ = tokio::signal::ctrl_c().await;
                }
            };

            tokio::select! {
                res = service.waiting() => {
                    if let Err(e) = res {
                        tracing::error!("MCP server error: {e}");
                    }
                }
                () = shutdown => {
                    tracing::info!("Received shutdown signal, shutting down");
                }
            }

            tracing::info!("MCP server shut down");
            if has_cargo {
                illu_rs::status::clear();
            }
        }
        Some(Command::Query {
            search,
            scope,
            kind,
        }) => {
            let db = open_or_index(repo_path)?;
            let result = handle_query(
                &db,
                &search,
                Some(&scope),
                kind.as_deref(),
                None,
                None,
                None,
                None,
            )?;
            print_result(&result);
        }
        Some(Command::Context { symbol }) => {
            let db = open_or_index(repo_path)?;
            let result = handle_context(&db, &symbol, false, None, None, None, false)?;
            print_result(&result);
        }
        Some(Command::Impact { symbol }) => {
            let db = open_or_index(repo_path)?;
            let result = handle_impact(&db, &symbol, None, false, false)?;
            print_result(&result);
        }
        Some(Command::Docs { dep, topic }) => {
            let db = open_or_index(repo_path)?;
            let result = handle_docs(&db, &dep, topic.as_deref())?;
            print_result(&result);
        }
        Some(Command::Tree { path }) => {
            let db = open_or_index(repo_path)?;
            let result = handle_tree(&db, &path)?;
            print_result(&result);
        }
        #[cfg(feature = "dashboard")]
        Some(Command::Dashboard { port }) => {
            let registry = {
                let path = illu_rs::registry::Registry::default_path()
                    .unwrap_or_else(|_| repo_path.join(".illu/registry.toml"));
                illu_rs::registry::Registry::load(&path)
                    .unwrap_or_else(|_| illu_rs::registry::Registry::empty())
            };
            illu_rs::server::dashboard::start_dashboard(registry, port).await?;
        }
        Some(Command::Init) => {
            init_repo(repo_path)?;
        }
        Some(Command::Install) => {
            install_global()?;
        }
        Some(Command::Repo { command }) => {
            handle_repo_command(command)?;
        }
    }

    Ok(())
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn test_generate_agent_files_creates_three_files() {
        let dir = tempfile::tempdir().unwrap();
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
        assert!(review.contains("mcp__illu__impact"));
        assert!(refactor.contains("mcp__illu__rename_plan"));

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

    #[test]
    fn test_append_gitignore_entry_manages_entries() {
        let dir = tempfile::tempdir().unwrap();
        let gitignore = dir.path().join(".gitignore");

        // Empty file -> adds all 3
        std::fs::write(&gitignore, "").unwrap();
        assert!(append_gitignore_entry(&gitignore).unwrap());
        let content = std::fs::read_to_string(&gitignore).unwrap();
        assert!(content.contains(".illu/"));
        assert!(content.contains(".claude/agents/illu-*.md"));
        assert!(content.contains(".gemini/agents/illu-*.md"));

        // Idempotency: second call returns false, no duplicates
        assert!(!append_gitignore_entry(&gitignore).unwrap());
        let content2 = std::fs::read_to_string(&gitignore).unwrap();
        assert_eq!(content, content2);

        // File with old `.illu` (no slash) -> recognizes it, adds only agent entries
        let gitignore2 = dir.path().join(".gitignore2");
        std::fs::write(&gitignore2, ".illu\n").unwrap();
        assert!(append_gitignore_entry(&gitignore2).unwrap());
        let content3 = std::fs::read_to_string(&gitignore2).unwrap();
        // Only original `.illu`, no `.illu/` added
        assert_eq!(
            content3.matches(".illu").count() - content3.matches(".illu/").count(),
            1
        );
        assert!(content3.contains(".claude/agents/illu-*.md"));

        // No trailing newline
        let gitignore3 = dir.path().join(".gitignore3");
        std::fs::write(&gitignore3, "/target").unwrap();
        assert!(append_gitignore_entry(&gitignore3).unwrap());
        let content4 = std::fs::read_to_string(&gitignore3).unwrap();
        assert!(content4.starts_with("/target\n"));
        assert!(content4.contains(".illu/"));
    }

    #[test]
    fn test_ensure_tools_allowed() {
        let dir = tempfile::tempdir().unwrap();

        // Creates parent dir and file if they don't exist
        let settings = dir.path().join("subdir/.claude/settings.local.json");
        ensure_tools_allowed(&settings).unwrap();
        let content = std::fs::read_to_string(&settings).unwrap();
        let val: serde_json::Value = serde_json::from_str(&content).unwrap();
        let allow = val["permissions"]["allow"].as_array().unwrap();
        assert!(allow.iter().any(|v| v.as_str() == Some("mcp__illu__*")));

        // Idempotent: second call doesn't duplicate the entry
        ensure_tools_allowed(&settings).unwrap();
        let content2 = std::fs::read_to_string(&settings).unwrap();
        let val2: serde_json::Value = serde_json::from_str(&content2).unwrap();
        let allow2 = val2["permissions"]["allow"].as_array().unwrap();
        assert_eq!(
            allow2
                .iter()
                .filter(|v| v.as_str() == Some("mcp__illu__*"))
                .count(),
            1,
            "mcp__illu__* must not be duplicated"
        );

        // Preserves existing entries
        let settings2 = dir.path().join("settings2.json");
        std::fs::write(
            &settings2,
            r#"{"permissions":{"allow":["Bash"],"deny":["rm"]}}"#,
        )
        .unwrap();
        ensure_tools_allowed(&settings2).unwrap();
        let content3 = std::fs::read_to_string(&settings2).unwrap();
        let val3: serde_json::Value = serde_json::from_str(&content3).unwrap();
        let allow3 = val3["permissions"]["allow"].as_array().unwrap();
        assert!(allow3.iter().any(|v| v.as_str() == Some("Bash")));
        assert!(allow3.iter().any(|v| v.as_str() == Some("mcp__illu__*")));
        let deny3 = val3["permissions"]["deny"].as_array().unwrap();
        assert!(deny3.iter().any(|v| v.as_str() == Some("rm")));
    }
}
