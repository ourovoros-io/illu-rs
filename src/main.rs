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
}

fn write_mcp_server_config(
    repo_path: &Path,
    config_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let binary = std::env::current_exe()?
        .canonicalize()?
        .to_string_lossy()
        .into_owned();
    let repo = repo_path.canonicalize()?.to_string_lossy().into_owned();

    let illu_entry = serde_json::json!({
        "command": binary,
        "args": ["--repo", repo, "serve"],
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

This repo is indexed by illu. **Use illu tools as your first step** — before reading files, \
before grep, before guessing at code structure.

### When to use illu

- **Starting any task**: `{cmd_prefix} query` the relevant symbols to understand what exists
- **Before modifying a function/struct/trait**: `{cmd_prefix} impact` to see what depends on it
- **Debugging or tracing issues**: `{cmd_prefix} context` to get the full definition and references
- **Using an external crate**: `{cmd_prefix} docs` to check how it's used in this project
- **Before reading files**: query first — illu tells you exactly where things are
- **Finding call paths**: `{cmd_prefix} callpath` to trace how one symbol reaches another
- **Dead code detection**: `{cmd_prefix} unused` to find unreferenced symbols
- **Index health**: `{cmd_prefix} freshness` to check if the index is current

### Commands

| User types | MCP tool | Params |
|------------|----------|--------|
| `{cmd_prefix} query <term>` | `{tool_prefix}query` | `query: \"<term>\"` |
| `{cmd_prefix} query <term> --scope <s>` | `{tool_prefix}query` | `query: \"<term>\", scope: \"<s>\"` |
| `{cmd_prefix} context <symbol>` | `{tool_prefix}context` | `symbol_name: \"<symbol>\"` |
| `{cmd_prefix} context Type::method` | `{tool_prefix}context` | `symbol_name: \"Type::method\"` |
| `{cmd_prefix} context <symbol> --file <f>` | `{tool_prefix}context` | `symbol_name: \"<symbol>\", file: \"<f>\"` |
| `{cmd_prefix} impact <symbol>` | `{tool_prefix}impact` | `symbol_name: \"<symbol>\"` |
| `{cmd_prefix} impact <symbol> --depth 1` | `{tool_prefix}impact` | `symbol_name: \"<symbol>\", depth: 1` |
| `{cmd_prefix} docs <dep>` | `{tool_prefix}docs` | `dependency: \"<dep>\"` |
| `{cmd_prefix} docs <dep> --topic <t>` | `{tool_prefix}docs` | `dependency: \"<dep>\", topic: \"<t>\"` |
| `{cmd_prefix} callpath <from> <to>` | `{tool_prefix}callpath` | `from: \"<from>\", to: \"<to>\"` |
| `{cmd_prefix} batch_context <sym1> <sym2>` | `{tool_prefix}batch_context` | `symbols: [\"<sym1>\", \"<sym2>\"]` |
| `{cmd_prefix} unused` | `{tool_prefix}unused` | |
| `{cmd_prefix} unused --path src/server/` | `{tool_prefix}unused` | `path: \"src/server/\"` |
| `{cmd_prefix} freshness` | `{tool_prefix}freshness` | |
| `{cmd_prefix} crate_graph` | `{tool_prefix}crate_graph` | |

### Workflow rules

1. **Locate before you read**: `{cmd_prefix} query` or `{cmd_prefix} context` to find the right file:line, then Read only what you need
2. **Impact before you change**: always run `{cmd_prefix} impact` before modifying any public symbol
3. **Chain tools**: `{cmd_prefix} query` to find candidates → `{cmd_prefix} context` for the one you need → `{cmd_prefix} impact` before changing it
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

fn ensure_gitignore(repo_path: &Path) -> Result<bool, Box<dyn std::error::Error>> {
    let gitignore_path = repo_path.join(".gitignore");
    let content = std::fs::read_to_string(&gitignore_path).unwrap_or_default();

    if content
        .lines()
        .any(|l| l.trim() == ".illu/" || l.trim() == ".illu")
    {
        return Ok(false);
    }

    let mut new_content = content;
    if !new_content.is_empty() && !new_content.ends_with('\n') {
        new_content.push('\n');
    }
    new_content.push_str(".illu/\n");
    std::fs::write(&gitignore_path, new_content)?;
    Ok(true)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let repo_path = &cli.repo;

    match cli.command {
        None | Some(Command::Serve) => {
            tracing::info!(repo = %repo_path.display(), "Starting illu server");
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

            // Index eagerly at startup so tools are ready immediately
            illu_rs::status::set("indexing");
            let refreshed = illu_rs::indexer::refresh_index(&db, &config)?;
            if refreshed > 0 {
                tracing::info!(count = refreshed, "Refreshed changed files");
            }

            // Check for pending docs before handing DB to the server
            let pending_docs = illu_rs::indexer::docs::pending_docs(&db)?;

            let server = IlluServer::new(db, config.clone());
            let db_arc = server.db();
            let transport = stdio();
            tracing::info!("MCP transport ready, starting handshake");
            illu_rs::status::set(illu_rs::status::READY);
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
            illu_rs::status::clear();
        }
        Some(Command::Query {
            search,
            scope,
            kind,
        }) => {
            let db = open_or_index(repo_path)?;
            let result = handle_query(&db, &search, Some(&scope), kind.as_deref(), None, None, None)?;
            print_result(&result);
        }
        Some(Command::Context { symbol }) => {
            let db = open_or_index(repo_path)?;
            let result = handle_context(&db, &symbol, false, None)?;
            print_result(&result);
        }
        Some(Command::Impact { symbol }) => {
            let db = open_or_index(repo_path)?;
            let result = handle_impact(&db, &symbol, None, false)?;
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
    }

    Ok(())
}
