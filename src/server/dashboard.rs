use crate::db::Database;
use crate::registry::{Registry, RepoEntry};
use async_process::Command;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{delete, get},
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Deserialize)]
pub struct DeleteRepoRequest {
    pub path: String,
}

#[derive(Serialize, Default)]
pub struct RepoStatus {
    pub name: String,
    pub path: String,
    pub error: Option<String>,
    pub git_remote: Option<String>,
    pub last_indexed: String,
    pub index_size_bytes: u64,
    pub file_count: i64,
    pub symbol_count: i64,
    pub crate_count: i64,
    pub dep_count: i64,
    pub ref_count: i64,
    pub health: HealthInfo,
    pub indexed_commit: Option<String>,
    pub current_head: Option<String>,
    pub index_version: Option<String>,
    pub binary_version: String,
    pub is_stale: bool,
}

#[derive(Serialize, Default)]
pub struct HealthInfo {
    pub total_refs: i64,
    pub confidence_distribution: Vec<(String, i64)>,
    pub truncated_signatures: i64,
    pub total_functions: i64,
}

struct RepoIndexStats {
    index_size_bytes: u64,
    file_count: i64,
    symbol_count: i64,
    crate_count: i64,
    dep_count: i64,
    ref_count: i64,
    health: HealthInfo,
    indexed_commit: Option<String>,
    index_version: Option<String>,
}

#[derive(Serialize)]
pub struct DashboardData {
    pub repos: Vec<RepoStatus>,
    pub system_info: SystemInfo,
}

#[derive(Serialize)]
pub struct SystemInfo {
    pub os: String,
    pub arch: String,
    pub illu_version: String,
}

async fn get_dashboard_data(State(state): State<Arc<DashboardState>>) -> Json<DashboardData> {
    let registry_repos = {
        let registry = state.registry.read().await;
        registry.repos.clone()
    };

    // Process repositories with limited concurrency (max 10 at once)
    let repos = futures::stream::iter(registry_repos)
        .map(|entry| tokio::spawn(async move { get_repo_status_async(entry).await }))
        .buffer_unordered(10)
        .filter_map(|res| async move {
            match res {
                Ok(status) => Some(status),
                Err(err) => {
                    tracing::error!("Internal task error gathering repo status: {err}");
                    None
                }
            }
        })
        .collect::<Vec<_>>()
        .await;

    let system_info = SystemInfo {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        illu_version: env!("CARGO_PKG_VERSION").to_string(),
    };

    Json(DashboardData { repos, system_info })
}

async fn get_repo_status_async(entry: RepoEntry) -> RepoStatus {
    let binary_version = crate::indexer::INDEX_VERSION.to_string();
    let current_head = read_git_head(&entry.path).await;

    let entry_clone = entry.clone();
    let db_res = tokio::task::spawn_blocking(move || read_index_stats(&entry_clone.path)).await;

    match db_res {
        Ok(Ok(stats)) => build_ok_status(entry, binary_version, current_head, stats),
        Ok(Err(e)) => build_error_status(entry, binary_version, current_head, e),
        Err(_) => build_error_status(
            entry,
            binary_version,
            current_head,
            "Internal error: task panicked".to_string(),
        ),
    }
}

async fn read_git_head(path: &std::path::Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(path)
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn read_index_stats(repo_path: &std::path::Path) -> Result<RepoIndexStats, String> {
    let db_path = repo_path.join(".illu/index.db");
    if !db_path.exists() {
        return Err(
            "Index database not found. Run `illu serve` to index this repository.".to_string(),
        );
    }

    let index_size_bytes = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);
    let db = Database::open_readonly(&db_path).map_err(|e| format!("Failed to open index: {e}"))?;

    let confidence_distribution = db.count_refs_by_confidence().unwrap_or_default();
    let total_refs: i64 = confidence_distribution.iter().map(|(_, c)| c).sum();
    let repo_str = repo_path.to_string_lossy();

    Ok(RepoIndexStats {
        index_size_bytes,
        file_count: db.file_count().unwrap_or(0),
        symbol_count: db.symbol_count().unwrap_or(0),
        crate_count: db.get_crate_count().unwrap_or(0),
        dep_count: db.dep_count().unwrap_or(0),
        ref_count: db.ref_count().unwrap_or(0),
        health: HealthInfo {
            total_refs,
            confidence_distribution,
            truncated_signatures: db.count_truncated_signatures().unwrap_or(0),
            total_functions: db.count_functions().unwrap_or(0),
        },
        indexed_commit: db.get_commit_hash(&repo_str).unwrap_or(None),
        index_version: db.get_index_version(&repo_str).unwrap_or(None),
    })
}

fn build_ok_status(
    entry: RepoEntry,
    binary_version: String,
    current_head: Option<String>,
    stats: RepoIndexStats,
) -> RepoStatus {
    let is_stale = stats.index_version.as_deref() != Some(crate::indexer::INDEX_VERSION)
        || stats.indexed_commit != current_head;

    RepoStatus {
        name: entry.name,
        path: entry.path.to_string_lossy().to_string(),
        error: None,
        git_remote: entry.git_remote,
        last_indexed: entry.last_indexed,
        index_size_bytes: stats.index_size_bytes,
        file_count: stats.file_count,
        symbol_count: stats.symbol_count,
        crate_count: stats.crate_count,
        dep_count: stats.dep_count,
        ref_count: stats.ref_count,
        health: stats.health,
        indexed_commit: stats.indexed_commit,
        current_head,
        index_version: stats.index_version,
        binary_version,
        is_stale,
    }
}

fn build_error_status(
    entry: RepoEntry,
    binary_version: String,
    current_head: Option<String>,
    error: String,
) -> RepoStatus {
    RepoStatus {
        name: entry.name,
        path: entry.path.to_string_lossy().to_string(),
        error: Some(error),
        git_remote: entry.git_remote,
        last_indexed: entry.last_indexed,
        binary_version,
        current_head,
        is_stale: true,
        ..Default::default()
    }
}

async fn dashboard_html() -> Html<&'static str> {
    Html(include_str!("dashboard.html"))
}

async fn delete_repo(
    State(state): State<Arc<DashboardState>>,
    Json(payload): Json<DeleteRepoRequest>,
) -> impl IntoResponse {
    let path = std::path::PathBuf::from(&payload.path);
    let mut registry_lock = state.registry.write().await;
    registry_lock.remove(&path);

    // Create a temporary registry to save it in a blocking task
    // Registry::save doesn't need to be in the lock once we have the data
    // but we need to ensure we don't block the async runtime.
    let registry_to_save = Registry {
        file_path: registry_lock.file_path.clone(),
        repos: registry_lock.repos.clone(),
    };

    let save_res = tokio::task::spawn_blocking(move || registry_to_save.save()).await;

    match save_res {
        Ok(Ok(())) => StatusCode::OK.into_response(),
        Ok(Err(e)) => {
            tracing::error!(
                "Failed to save registry after removing {}: {e}",
                payload.path
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to save registry: {e}"),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("Join error saving registry: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error saving registry".to_string(),
            )
                .into_response()
        }
    }
}

pub struct DashboardState {
    pub registry: Arc<RwLock<Registry>>,
}

pub async fn start_dashboard(
    registry: Registry,
    port: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    let state = Arc::new(DashboardState {
        registry: Arc::new(RwLock::new(registry)),
    });

    let app = Router::new()
        .route("/", get(dashboard_html))
        .route("/api/data", get(get_dashboard_data))
        .route("/api/repos", delete(delete_repo))
        .with_state(state);

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let url = format!("http://{addr}");
    tracing::info!("Dashboard server listening on {url}");

    #[expect(
        clippy::print_stdout,
        reason = "user-facing CLI command announces its URL on stdout"
    )]
    {
        println!("Illu Dashboard is running at {url}");
        println!("Press Ctrl+C to stop.");
    }

    // Automatically open the browser
    if let Err(e) = opener::open(&url) {
        tracing::warn!("Failed to open browser automatically: {e}");
    }

    axum::serve(listener, app).await?;
    Ok(())
}
