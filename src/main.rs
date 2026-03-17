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
}

fn write_mcp_config(repo_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
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

    let mcp_path = repo_path.join(".mcp.json");
    let mut config: serde_json::Value = std::fs::read_to_string(&mcp_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({"mcpServers": {}}));

    config["mcpServers"]["illu"] = illu_entry;

    std::fs::write(&mcp_path, serde_json::to_string_pretty(&config)?)?;
    tracing::info!("Wrote MCP config to {}", mcp_path.display());
    Ok(())
}

const ILLU_SECTION_START: &str = "<!-- illu:start -->";
const ILLU_SECTION_END: &str = "<!-- illu:end -->";

fn illu_claude_section() -> String {
    format!(
        "{ILLU_SECTION_START}
## Code Intelligence (illu)

This repo is indexed by illu. **Use illu tools as your first step** — before reading files, \
before grep, before guessing at code structure.

### When to use illu

- **Starting any task**: `illu query` the relevant symbols to understand what exists
- **Before modifying a function/struct/trait**: `illu impact` to see what depends on it
- **Debugging or tracing issues**: `illu context` to get the full definition and references
- **Using an external crate**: `illu docs` to check how it's used in this project
- **Before reading files**: query first — illu tells you exactly where things are

### Commands

| User types | MCP tool | Params |
|------------|----------|--------|
| `illu query <term>` | `mcp__illu__query` | `query: \"<term>\"` |
| `illu query <term> --scope <s>` | `mcp__illu__query` | `query: \"<term>\", scope: \"<s>\"` |
| `illu context <symbol>` | `mcp__illu__context` | `symbol_name: \"<symbol>\"` |
| `illu impact <symbol>` | `mcp__illu__impact` | `symbol_name: \"<symbol>\"` |
| `illu docs <dep>` | `mcp__illu__docs` | `dependency: \"<dep>\"` |
| `illu docs <dep> --topic <t>` | `mcp__illu__docs` | `dependency: \"<dep>\", topic: \"<t>\"` |

### Workflow rules

1. **Locate before you read**: `illu query` or `illu context` to find the right file:line, then Read only what you need
2. **Impact before you change**: always run `illu impact` before modifying any public symbol
3. **Chain tools**: `illu query` to find candidates → `illu context` for the one you need → `illu impact` before changing it
{ILLU_SECTION_END}"
    )
}

fn write_claude_md_section(repo_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let claude_md_path = repo_path.join("CLAUDE.md");
    let section = illu_claude_section();

    let content = std::fs::read_to_string(&claude_md_path).unwrap_or_default();

    let new_content = if let Some(start) = content.find(ILLU_SECTION_START) {
        if let Some(end) = content.find(ILLU_SECTION_END) {
            let end = end + ILLU_SECTION_END.len();
            format!("{}{section}{}", &content[..start], &content[end..])
        } else {
            format!("{}{section}{}", &content[..start], &content[start..])
        }
    } else if content.is_empty() {
        format!("# CLAUDE.md\n\n{section}\n")
    } else {
        format!("{content}\n{section}\n")
    };

    std::fs::write(&claude_md_path, new_content)?;
    tracing::info!("Updated CLAUDE.md with illu section");
    Ok(())
}

fn open_or_index(repo_path: &Path) -> Result<Database, Box<dyn std::error::Error>> {
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
            let db_dir = repo_path.join(".illu");
            std::fs::create_dir_all(&db_dir)?;
            write_mcp_config(repo_path)?;
            write_claude_md_section(repo_path)?;

            let config = IndexConfig {
                repo_path: repo_path.clone(),
            };

            let db_path = db_dir.join("index.db");
            let db = Database::open(&db_path)?;

            // If already indexed, fetch any missing docs (fast, cached)
            // If not indexed, skip — first tool call will trigger full index
            let has_index = db
                .get_all_file_paths()
                .map(|f| !f.is_empty())
                .unwrap_or(false);
            if has_index {
                illu_rs::indexer::docs::fetch_dependency_docs(&db).await?;
            }

            let server = IlluServer::new(db, config);
            let transport = stdio();
            let service = server.serve(transport).await?;
            service.waiting().await?;
        }
        Some(Command::Query {
            search,
            scope,
            kind,
        }) => {
            let db = open_or_index(repo_path)?;
            let result = handle_query(&db, &search, Some(&scope), kind.as_deref())?;
            print_result(&result);
        }
        Some(Command::Context { symbol }) => {
            let db = open_or_index(repo_path)?;
            let result = handle_context(&db, &symbol)?;
            print_result(&result);
        }
        Some(Command::Impact { symbol }) => {
            let db = open_or_index(repo_path)?;
            let result = handle_impact(&db, &symbol)?;
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
    }

    Ok(())
}
