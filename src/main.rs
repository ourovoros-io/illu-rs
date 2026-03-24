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
#[command(name = "illu", about = "Rust code intelligence")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Path to the repo
    #[arg(long, short, global = true, default_value = ".")]
    repo: PathBuf,
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
    /// Set up illu in a Rust repo (configures Claude Code + Gemini CLI, builds index)
    Init,
    /// Install illu globally (configures Claude Code + Gemini CLI for all repos)
    Install,
}

fn write_mcp_config_to(
    config_path: &Path,
    args: &[&str],
) -> Result<(), Box<dyn std::error::Error>> {
    let binary = std::env::current_exe()?
        .canonicalize()?
        .to_string_lossy()
        .into_owned();

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

fn write_mcp_server_config(
    repo_path: &Path,
    config_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let repo = repo_path.canonicalize()?.to_string_lossy().into_owned();
    write_mcp_config_to(config_path, &["--repo", &repo, "serve"])
}

fn write_mcp_config(repo_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    write_mcp_server_config(repo_path, &repo_path.join(".mcp.json"))
}

fn write_gemini_config(repo_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    write_mcp_server_config(repo_path, &repo_path.join(".gemini/settings.json"))
}

const ILLU_SECTION_START: &str = "<!-- illu:start -->";
const ILLU_SECTION_END: &str = "<!-- illu:end -->";

fn illu_agent_section(cmd_prefix: &str, tool_prefix: &str) -> String {
    format!(
        "{ILLU_SECTION_START}
## Code Intelligence (illu)

This repo is indexed by illu (36 tools). **Use illu tools as your first step** — before reading files, \
before grep, before guessing at code structure.

### When to use illu

- **Starting any task**: `{cmd_prefix} query` the relevant symbols to understand what exists
- **Before modifying a function/struct/trait**: `{cmd_prefix} impact` to see what depends on it
- **Debugging or tracing issues**: `{cmd_prefix} context` to get the full definition and references
- **Understanding call flow**: `{cmd_prefix} neighborhood` or `{cmd_prefix} callpath` to explore the call graph
- **Before refactoring a module**: `{cmd_prefix} boundary` to see what's public API vs internal
- **Using an external crate**: `{cmd_prefix} docs` to check how it's used in this project
- **Before reading files**: query first — illu tells you exactly where things are
- **Finding which tests to run**: `{cmd_prefix} test_impact` after changing a symbol
- **Dead code detection**: `{cmd_prefix} unused` or `{cmd_prefix} orphaned` to find unreferenced symbols
- **Index health**: `{cmd_prefix} freshness` to check if the index is current
- **Cross-repo analysis**: `{cmd_prefix} cross_query` to find symbols in other repos, `{cmd_prefix} cross_impact` to check cross-repo effects
- **Repo overview**: `{cmd_prefix} repos` to see all registered repos

### Commands

| User types | MCP tool | Params |
|------------|----------|--------|
| `{cmd_prefix} query <term>` | `{tool_prefix}query` | `query: \"<term>\"` |
| `{cmd_prefix} query <term> --scope <s>` | `{tool_prefix}query` | `query: \"<term>\", scope: \"<s>\"` |
| `{cmd_prefix} query * --kind struct` | `{tool_prefix}query` | `query: \"*\", kind: \"struct\"` |
| `{cmd_prefix} query * --sig \"-> Result\"` | `{tool_prefix}query` | `query: \"*\", signature: \"-> Result\"` |
| `{cmd_prefix} context <symbol>` | `{tool_prefix}context` | `symbol_name: \"<symbol>\"` |
| `{cmd_prefix} context Type::method` | `{tool_prefix}context` | `symbol_name: \"Type::method\"` |
| `{cmd_prefix} context <sym> --sections source,callers` | `{tool_prefix}context` | `symbol_name: \"<sym>\", sections: [\"source\", \"callers\"]` |
| `{cmd_prefix} context <sym> --exclude-tests` | `{tool_prefix}context` | `symbol_name: \"<sym>\", exclude_tests: true` |
| `{cmd_prefix} batch_context <sym1> <sym2>` | `{tool_prefix}batch_context` | `symbols: [\"<sym1>\", \"<sym2>\"]` |
| `{cmd_prefix} impact <symbol>` | `{tool_prefix}impact` | `symbol_name: \"<symbol>\"` |
| `{cmd_prefix} impact <symbol> --depth 1` | `{tool_prefix}impact` | `symbol_name: \"<symbol>\", depth: 1` |
| `{cmd_prefix} diff_impact` | `{tool_prefix}diff_impact` | *(unstaged changes)* |
| `{cmd_prefix} diff_impact main` | `{tool_prefix}diff_impact` | `git_ref: \"main\"` |
| `{cmd_prefix} test_impact <symbol>` | `{tool_prefix}test_impact` | `symbol_name: \"<symbol>\"` |
| `{cmd_prefix} callpath <from> <to>` | `{tool_prefix}callpath` | `from: \"<from>\", to: \"<to>\"` |
| `{cmd_prefix} neighborhood <symbol>` | `{tool_prefix}neighborhood` | `symbol_name: \"<symbol>\"` |
| `{cmd_prefix} neighborhood <sym> --format tree` | `{tool_prefix}neighborhood` | `symbol_name: \"<sym>\", format: \"tree\"` |
| `{cmd_prefix} references <symbol>` | `{tool_prefix}references` | `symbol_name: \"<symbol>\"` |
| `{cmd_prefix} boundary src/server/` | `{tool_prefix}boundary` | `path: \"src/server/\"` |
| `{cmd_prefix} unused` | `{tool_prefix}unused` | |
| `{cmd_prefix} unused --path src/server/` | `{tool_prefix}unused` | `path: \"src/server/\"` |
| `{cmd_prefix} orphaned` | `{tool_prefix}orphaned` | |
| `{cmd_prefix} overview src/` | `{tool_prefix}overview` | `path: \"src/\"` |
| `{cmd_prefix} stats` | `{tool_prefix}stats` | |
| `{cmd_prefix} hotspots` | `{tool_prefix}hotspots` | |
| `{cmd_prefix} implements --trait Display` | `{tool_prefix}implements` | `trait_name: \"Display\"` |
| `{cmd_prefix} docs <dep>` | `{tool_prefix}docs` | `dependency: \"<dep>\"` |
| `{cmd_prefix} docs <dep> --topic <t>` | `{tool_prefix}docs` | `dependency: \"<dep>\", topic: \"<t>\"` |
| `{cmd_prefix} freshness` | `{tool_prefix}freshness` | |
| `{cmd_prefix} crate_graph` | `{tool_prefix}crate_graph` | |
| `{cmd_prefix} blame <symbol>` | `{tool_prefix}blame` | `symbol_name: \"<symbol>\"` |
| `{cmd_prefix} history <symbol>` | `{tool_prefix}history` | `symbol_name: \"<symbol>\"` |
| `{cmd_prefix} repos` | `{tool_prefix}repos` | |
| `{cmd_prefix} cross_query <term>` | `{tool_prefix}cross_query` | `query: \"<term>\"` |
| `{cmd_prefix} cross_impact <symbol>` | `{tool_prefix}cross_impact` | `symbol_name: \"<symbol>\"` |
| `{cmd_prefix} cross_deps` | `{tool_prefix}cross_deps` | |
| `{cmd_prefix} cross_callpath <from> <to>` | `{tool_prefix}cross_callpath` | `from: \"<from>\", to: \"<to>\"` |

### Workflow rules

1. **Locate before you read**: `{cmd_prefix} query` or `{cmd_prefix} context` to find the right file:line, then Read only what you need
2. **Impact before you change**: always run `{cmd_prefix} impact` before modifying any public symbol
3. **Chain tools**: `{cmd_prefix} query` to find candidates → `{cmd_prefix} context` for the one you need → `{cmd_prefix} impact` before changing it
4. **Save tokens**: use `sections: [\"source\", \"callers\"]` on context/batch_context to fetch only what you need
5. **Production focus**: use `exclude_tests: true` on context/neighborhood/callpath to filter out test functions

### Cross-repo workflow

**NEVER navigate to or read files from other repositories directly.** Use cross-repo tools instead — they query other repos' indexes without leaving this repo.

1. `{cmd_prefix} repos` — confirm the other repo is indexed and available
2. `{cmd_prefix} cross_query <term>` — search symbols across all indexed repos
3. `{cmd_prefix} cross_impact <symbol>` — find which code in other repos references a symbol
4. `{cmd_prefix} cross_deps` — show inter-repo dependency relationships
5. `{cmd_prefix} cross_callpath <from> <to>` — find call chains spanning repo boundaries

Cross-repo tools open other repos' indexes read-only. They work as long as the other repo has been indexed by illu (check with `{cmd_prefix} repos`). \
If a repo is not indexed, ask the user to run illu on it first.
{ILLU_SECTION_END}"
    )
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
    let section = illu_agent_section("illu", "mcp__illu__");
    write_md_section(repo_path, "CLAUDE.md", "# CLAUDE.md", &section)
}

fn write_gemini_md_section(repo_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let section = illu_agent_section("@illu", "mcp_illu_");
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

    // Verify it's a Rust project
    let cargo_toml = repo_path.join("Cargo.toml");
    if !cargo_toml.exists() {
        return Err(format!("No Cargo.toml found in {}", repo_path.display()).into());
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

    // 3. Build initial index
    println!("  indexing...");
    illu_rs::status::init(&repo_path);
    ensure_indexed(&repo_path)?;
    println!("  index built");

    // 4. Add .illu/ to .gitignore if not already there
    if ensure_gitignore(&repo_path)? {
        println!("  added .illu/ to .gitignore");
    }

    println!("\nDone. Start Claude Code or Gemini CLI in this repo — illu will run automatically.");
    Ok(())
}

fn append_gitignore_entry(path: &Path) -> Result<bool, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    if content
        .lines()
        .any(|l| l.trim() == ".illu/" || l.trim() == ".illu")
    {
        return Ok(false);
    }
    let mut out = content;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(".illu/\n");
    std::fs::write(path, out)?;
    Ok(true)
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
    println!("  wrote {}", claude_settings.display());

    let gemini_settings = home.join(".gemini/settings.json");
    write_global_mcp_config(&gemini_settings)?;
    println!("  wrote {}", gemini_settings.display());

    write_md_section(
        &home,
        ".claude/CLAUDE.md",
        "# CLAUDE.md",
        &illu_agent_section("illu", "mcp__illu__"),
    )?;
    println!("  updated {}", home.join(".claude/CLAUDE.md").display());

    write_md_section(
        &home,
        ".gemini/GEMINI.md",
        "# GEMINI.md",
        &illu_agent_section("@illu", "mcp_illu_"),
    )?;
    println!("  updated {}", home.join(".gemini/GEMINI.md").display());

    install_statusline(&home)?;

    ensure_global_gitignore(&home)?;

    println!("\nDone. illu will auto-start in any Rust repo.");
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
    let repo_path = if cli.repo == Path::new(".") {
        let cwd = std::env::current_dir()?;
        match illu_rs::git::detect_repo_root(&cwd) {
            Ok(git_root) => illu_rs::git::detect_cargo_root(&cwd, &git_root),
            Err(_) => cwd,
        }
    } else {
        cli.repo.clone()
    };
    let repo_path = &repo_path;

    match cli.command {
        None | Some(Command::Serve) => {
            tracing::info!(repo = %repo_path.display(), "Starting illu server");
            let has_cargo = repo_path.join("Cargo.toml").exists();

            let (db, config) = if has_cargo {
                let db_dir = repo_path.join(".illu");
                std::fs::create_dir_all(&db_dir)?;
                illu_rs::status::init(repo_path);
                illu_rs::status::set("starting");
                write_mcp_config(repo_path)?;
                write_claude_md_section(repo_path)?;
                write_gemini_config(repo_path)?;
                write_gemini_md_section(repo_path)?;

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
                tracing::warn!("No Cargo.toml — starting with empty index (cross-repo tools only)");
                let db = Database::open_in_memory()?;
                let config = IndexConfig {
                    repo_path: repo_path.clone(),
                };
                (db, config)
            };

            if has_cargo {
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

            let server = IlluServer::new(db, config.clone(), registry);
            let db_arc = server.db();
            let transport = stdio();
            tracing::info!("MCP transport ready, starting handshake");
            if has_cargo {
                illu_rs::status::set(illu_rs::status::READY);
            }
            let service = server.serve(transport).await?;
            tracing::info!("MCP server initialized, waiting for requests");

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

            service.waiting().await?;
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
        Some(Command::Init) => {
            init_repo(repo_path)?;
        }
        Some(Command::Install) => {
            install_global()?;
        }
    }

    Ok(())
}
