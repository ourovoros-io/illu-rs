use crate::db::Database;
use crate::registry::{Registry, RepoEntry};
use axum::{
    Json, Router,
    extract::{Request, State},
    http::StatusCode,
    middleware::{Next, from_fn_with_state},
    response::{Html, IntoResponse, Response},
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
    pub last_indexed: String,
    pub index_size_bytes: u64,
    pub file_count: i64,
    pub symbol_count: i64,
    pub crate_count: i64,
    pub dep_count: i64,
    pub health: HealthInfo,
    pub indexed_commit: Option<String>,
    pub current_head: Option<String>,
    pub index_version: Option<String>,
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
    let current_head = read_git_head(&entry.path).await;

    let entry_clone = entry.clone();
    let db_res = tokio::task::spawn_blocking(move || read_index_stats(&entry_clone.path)).await;

    match db_res {
        Ok(Ok(stats)) => build_ok_status(entry, current_head, stats),
        Ok(Err(e)) => build_error_status(entry, current_head, e),
        Err(_) => build_error_status(entry, current_head, "Internal error: task panicked".into()),
    }
}

async fn read_git_head(path: &std::path::Path) -> Option<String> {
    // `rev-parse HEAD` does not execute hooks, but we still block system
    // and global git config so a crafted repo cannot influence behavior
    // via `include.path` or environment tricks.
    let output = tokio::process::Command::new("git")
        .arg("rev-parse")
        .arg("HEAD")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
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

    // The existence check above guarantees metadata is present; treat a
    // metadata failure as non-fatal (size is best-effort).
    let index_size_bytes = std::fs::metadata(&db_path).map_or(0, |m| m.len());
    let db = Database::open_readonly(&db_path).map_err(|e| format!("Failed to open index: {e}"))?;

    let confidence_distribution = db
        .count_refs_by_confidence()
        .map_err(|e| format!("count_refs_by_confidence: {e}"))?;
    let total_refs: i64 = confidence_distribution.iter().map(|(_, c)| c).sum();
    let repo_str = repo_path.to_string_lossy();

    Ok(RepoIndexStats {
        index_size_bytes,
        file_count: db.file_count().map_err(|e| format!("file_count: {e}"))?,
        symbol_count: db
            .symbol_count()
            .map_err(|e| format!("symbol_count: {e}"))?,
        crate_count: db
            .get_crate_count()
            .map_err(|e| format!("crate_count: {e}"))?,
        dep_count: db.dep_count().map_err(|e| format!("dep_count: {e}"))?,
        health: HealthInfo {
            total_refs,
            confidence_distribution,
            truncated_signatures: db
                .count_truncated_signatures()
                .map_err(|e| format!("truncated_signatures: {e}"))?,
            total_functions: db
                .count_functions()
                .map_err(|e| format!("total_functions: {e}"))?,
        },
        indexed_commit: db
            .get_commit_hash(&repo_str)
            .map_err(|e| format!("commit_hash: {e}"))?,
        index_version: db
            .get_index_version(&repo_str)
            .map_err(|e| format!("index_version: {e}"))?,
    })
}

fn build_ok_status(
    entry: RepoEntry,
    current_head: Option<String>,
    stats: RepoIndexStats,
) -> RepoStatus {
    let is_stale = crate::indexer::is_index_stale(
        stats.index_version.as_deref(),
        stats.indexed_commit.as_deref(),
        current_head.as_deref(),
    );

    RepoStatus {
        name: entry.name,
        path: entry.path.to_string_lossy().to_string(),
        error: None,
        last_indexed: entry.last_indexed,
        index_size_bytes: stats.index_size_bytes,
        file_count: stats.file_count,
        symbol_count: stats.symbol_count,
        crate_count: stats.crate_count,
        dep_count: stats.dep_count,
        health: stats.health,
        indexed_commit: stats.indexed_commit,
        current_head,
        index_version: stats.index_version,
        is_stale,
    }
}

fn build_error_status(entry: RepoEntry, current_head: Option<String>, error: String) -> RepoStatus {
    RepoStatus {
        name: entry.name,
        path: entry.path.to_string_lossy().to_string(),
        error: Some(error),
        last_indexed: entry.last_indexed,
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

    // Mutate the registry under the lock, then snapshot a clone for the
    // blocking save. Dropping the guard before `.await` keeps the lock
    // out of the await point.
    let (removed, snapshot) = {
        let mut guard = state.registry.write().await;
        let removed = guard.remove(&path);
        (removed, guard.clone())
    };

    if !removed {
        return (
            StatusCode::NOT_FOUND,
            format!("No registry entry found for path: {}", payload.path),
        )
            .into_response();
    }

    let save_res = tokio::task::spawn_blocking(move || snapshot.save()).await;
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
    /// The port the server is listening on. Used by the Host-header
    /// middleware to reject DNS-rebinding attempts.
    pub port: u16,
}

/// Rejects requests whose `Host` header does not match the expected
/// loopback `host:port`. Defends against DNS-rebinding attacks: a
/// malicious page served from `attacker.example` that rebinds its DNS
/// to `127.0.0.1` would still present `attacker.example:<port>` as the
/// Host header, which this middleware refuses. Requests with no Host
/// header are also rejected.
async fn validate_host(
    State(state): State<Arc<DashboardState>>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let host = req
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let expected_v4 = format!("127.0.0.1:{}", state.port);
    let expected_lh = format!("localhost:{}", state.port);
    if host == expected_v4 || host == expected_lh {
        Ok(next.run(req).await)
    } else {
        tracing::warn!("Rejected request with Host header: {host:?}");
        Err(StatusCode::FORBIDDEN)
    }
}

pub async fn start_dashboard(
    registry: Registry,
    port: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    let state = Arc::new(DashboardState {
        registry: Arc::new(RwLock::new(registry)),
        port,
    });

    let app = Router::new()
        .route("/", get(dashboard_html))
        .route("/api/data", get(get_dashboard_data))
        .route("/api/repos", delete(delete_repo))
        .route_layer(from_fn_with_state(Arc::clone(&state), validate_host))
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

    if let Err(e) = opener::open(&url) {
        tracing::warn!("Failed to open browser automatically: {e}");
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
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
    tracing::info!("Dashboard received shutdown signal");
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::registry::RepoEntry;
    use std::path::PathBuf;

    fn entry() -> RepoEntry {
        RepoEntry {
            name: "demo".into(),
            path: PathBuf::from("/tmp/demo"),
            git_remote: None,
            git_common_dir: PathBuf::from("/tmp/demo/.git"),
            last_indexed: "2026-04-20T00:00:00Z".into(),
        }
    }

    fn stats() -> RepoIndexStats {
        RepoIndexStats {
            index_size_bytes: 1024,
            file_count: 5,
            symbol_count: 10,
            crate_count: 1,
            dep_count: 2,
            health: HealthInfo::default(),
            indexed_commit: Some("abc".into()),
            index_version: Some(crate::indexer::INDEX_VERSION.to_string()),
        }
    }

    #[test]
    fn is_stale_when_version_mismatches() {
        assert!(crate::indexer::is_index_stale(
            Some("0.0.0"),
            Some("abc"),
            Some("abc")
        ));
    }

    #[test]
    fn is_stale_when_head_moved() {
        assert!(crate::indexer::is_index_stale(
            Some(crate::indexer::INDEX_VERSION),
            Some("abc"),
            Some("def")
        ));
    }

    #[test]
    fn is_fresh_when_version_and_head_match() {
        assert!(!crate::indexer::is_index_stale(
            Some(crate::indexer::INDEX_VERSION),
            Some("abc"),
            Some("abc")
        ));
    }

    #[test]
    fn build_ok_status_marks_fresh_when_everything_matches() {
        let status = build_ok_status(entry(), Some("abc".into()), stats());
        assert!(!status.is_stale);
        assert!(status.error.is_none());
        assert_eq!(status.file_count, 5);
    }

    #[test]
    fn build_ok_status_marks_stale_when_head_diverges() {
        let status = build_ok_status(entry(), Some("xyz".into()), stats());
        assert!(status.is_stale);
    }

    #[test]
    fn build_error_status_is_stale_and_preserves_error() {
        let status = build_error_status(entry(), None, "boom".into());
        assert!(status.is_stale);
        assert_eq!(status.error.as_deref(), Some("boom"));
    }

    #[test]
    fn read_index_stats_returns_error_for_missing_db() {
        let dir = tempfile::tempdir().unwrap();
        let res = read_index_stats(dir.path());
        assert!(res.is_err(), "missing db must error");
        let err = res.err().unwrap();
        assert!(err.contains("Index database not found"), "got: {err}");
    }
}
