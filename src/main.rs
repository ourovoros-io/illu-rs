use illu_rs::db::Database;
use illu_rs::indexer::{IndexConfig, index_repo};
use illu_rs::server::IlluServer;
use rmcp::ServiceExt;
use rmcp::transport::stdio;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let repo_path = match std::env::args().nth(1) {
        Some(arg) => PathBuf::from(arg),
        None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };

    let db_dir = repo_path.join(".illu");
    std::fs::create_dir_all(&db_dir)?;
    let db_path = db_dir.join("index.db");
    let db = Database::open(&db_path)?;

    tracing::info!("Indexing {}", repo_path.display());
    let config = IndexConfig {
        repo_path: repo_path.clone(),
        skip_doc_fetch: true,
    };
    index_repo(&db, &config)?;
    tracing::info!("Indexing complete");

    let server = IlluServer::new(db);
    let transport = stdio();
    let service = server.serve(transport).await?;
    service.waiting().await?;

    Ok(())
}
