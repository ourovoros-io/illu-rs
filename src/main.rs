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
    /// Set up illu in a repo (detects/prompts per-repo agents, builds index)
    Init {
        /// Configure a specific agent (repeatable). Example: --agent claude-code
        #[arg(long)]
        agent: Vec<String>,
        /// Configure every supported per-repo agent without prompting.
        #[arg(long)]
        all: bool,
        /// Skip the prompt and accept detected agents.
        #[arg(long, short = 'y')]
        yes: bool,
        /// Print what would be written without touching the filesystem.
        #[arg(long)]
        dry_run: bool,
    },
    /// Install illu globally (detects/prompts global agents)
    Install {
        #[arg(long)]
        agent: Vec<String>,
        #[arg(long)]
        all: bool,
        #[arg(long, short = 'y')]
        yes: bool,
        #[arg(long)]
        dry_run: bool,
    },
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
fn init_repo(
    repo_path: &Path,
    flags: &illu_rs::agents::SetupFlags,
) -> Result<(), Box<dyn std::error::Error>> {
    let repo_path = repo_path.canonicalize()?;

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

    let reports = illu_rs::agents::configure_repo(&repo_path, flags)?;
    for report in &reports {
        if report.skipped {
            continue;
        }
        println!("  configured {}", report.agent_id);
        for path in &report.written_paths {
            println!("    -> {}", path.display());
        }
    }
    if reports.is_empty() || reports.iter().all(|r| r.skipped) {
        if flags.explicit_agents.is_empty() && !flags.all {
            let scoped_ids: Vec<&str> = illu_rs::agents::AGENTS
                .iter()
                .filter(|a| a.repo_config.is_some())
                .map(|a| a.id)
                .collect();
            return Err(format!(
                "no agents detected. Pass --agent <id> to configure one explicitly. \
                 Supported per-repo agents: {}",
                scoped_ids.join(", ")
            )
            .into());
        }
        println!("  no agents configured (nothing selected)");
    }

    if flags.dry_run {
        println!("\n(dry run — no files written)");
        return Ok(());
    }

    println!("  indexing...");
    illu_rs::status::init(&repo_path);
    ensure_indexed(&repo_path)?;
    println!("  index built");

    if ensure_gitignore(&repo_path)? {
        println!("  updated .gitignore with illu entries");
    }

    println!("\nDone.");
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
fn install_global(flags: &illu_rs::agents::SetupFlags) -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::var("HOME").map_err(|_| "HOME environment variable not set")?;
    let home = PathBuf::from(home);

    println!("Installing illu globally...");

    let reports = illu_rs::agents::configure_global(&home, flags)?;
    for report in &reports {
        if report.skipped {
            continue;
        }
        println!("  configured {}", report.agent_id);
        for path in &report.written_paths {
            println!("    -> {}", path.display());
        }
    }
    if reports.is_empty() || reports.iter().all(|r| r.skipped) {
        if flags.explicit_agents.is_empty() && !flags.all {
            let scoped_ids: Vec<&str> = illu_rs::agents::AGENTS
                .iter()
                .filter(|a| a.global_config.is_some())
                .map(|a| a.id)
                .collect();
            return Err(format!(
                "no agents detected. Pass --agent <id> to configure one explicitly. \
                 Supported global agents: {}",
                scoped_ids.join(", ")
            )
            .into());
        }
        println!("  no agents configured (nothing selected)");
    }

    if flags.dry_run {
        println!("\n(dry run — no files written)");
        return Ok(());
    }

    install_statusline(&home)?;
    ensure_global_gitignore(&home)?;

    println!("\nDone.");
    Ok(())
}

/// Poll `git rev-parse HEAD` and refresh the index when HEAD changes.
/// Runs as a background task — detects `git pull`, `git checkout`, etc.
async fn head_watcher(db: std::sync::Arc<std::sync::Mutex<Database>>, config: IndexConfig) {
    const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

    let repo_path = config.repo_path.clone();
    let mut last_head = git_head_async(&repo_path).await;

    loop {
        tokio::time::sleep(POLL_INTERVAL).await;

        let current = git_head_async(&repo_path).await;
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

        last_head = git_head_async(&repo_path).await;
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

/// `git_head` wrapped for async callers: runs the blocking subprocess on
/// tokio's blocking pool so polling HEAD doesn't stall the reactor.
async fn git_head_async(repo_path: &Path) -> Option<String> {
    let repo_path = repo_path.to_path_buf();
    tokio::task::spawn_blocking(move || git_head(&repo_path))
        .await
        .ok()
        .flatten()
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
                .map_or(0, |d| d.as_secs());

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

            let home = std::env::var("HOME").ok().map(PathBuf::from);
            if let Some(home) = &home
                && let Err(e) = illu_rs::agents::self_heal_on_serve(
                    if has_project { Some(repo_path) } else { None },
                    home,
                )
            {
                tracing::warn!("Agent self-heal failed: {e}");
            }

            let (db, config) = if has_project {
                let db_dir = repo_path.join(".illu");
                std::fs::create_dir_all(&db_dir)?;
                illu_rs::status::init(repo_path);
                illu_rs::status::set("starting");

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
                            match ra.wait_for_ready(std::time::Duration::from_mins(2)).await {
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
            if has_project {
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
            if has_project {
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
        Some(Command::Init {
            agent,
            all,
            yes,
            dry_run,
        }) => {
            let flags = illu_rs::agents::SetupFlags {
                explicit_agents: agent,
                all,
                yes,
                dry_run,
            };
            init_repo(repo_path, &flags)?;
        }
        Some(Command::Install {
            agent,
            all,
            yes,
            dry_run,
        }) => {
            let flags = illu_rs::agents::SetupFlags {
                explicit_agents: agent,
                all,
                yes,
                dry_run,
            };
            install_global(&flags)?;
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
}
