#[cfg(feature = "dashboard")]
pub mod dashboard;
pub mod tools;

use crate::db::Database;
use crate::indexer::IndexConfig;
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content, ServerCapabilities, ServerInfo};
use rmcp::schemars;
use rmcp::{ErrorData as McpError, ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use std::fmt::Write;
use std::sync::{Mutex, MutexGuard};

const REFRESH_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(5);

/// Deserialize a `Vec<String>` leniently: accepts a real JSON array, a
/// JSON-encoded string containing an array (e.g. `"[\"a\", \"b\"]"`), or a
/// bare string (treated as a single-element array).
///
/// Some MCP callers stringify array parameters when generating tool calls.
/// Handling both forms server-side avoids surfacing those mistakes as errors.
fn lenient_vec_string<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    parse_vec_string(value).map_err(serde::de::Error::custom)
}

/// Option variant of [`lenient_vec_string`]. Missing/null yields `None`.
fn lenient_opt_vec_string<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    if value.is_null() {
        return Ok(None);
    }
    parse_vec_string(value)
        .map(Some)
        .map_err(serde::de::Error::custom)
}

/// Maximum unwrap depth when a caller nests stringified arrays inside each
/// other (`"\"[\\\"x\\\"]\""`). Bounds the loop below so a pathological
/// input cannot spin indefinitely. Four is generous — real callers produce
/// at most one level of stringification.
const MAX_STRING_UNWRAP_DEPTH: usize = 4;

/// Coerce a JSON value into `Vec<String>`, accepting the shapes MCP callers
/// produce in practice:
///
/// - A real array of strings → used as-is.
/// - A JSON-encoded string containing an array (e.g. `"[\"a\", \"b\"]"`) →
///   re-parsed. If re-parsing fails (malformed JSON, or a value that merely
///   starts with `[` like the symbol name `"[u8]::len"`), the raw string is
///   kept as a single element rather than erroring.
/// - Any other bare string → single-element vec. Ambiguous inputs such as
///   `"foo,bar"` become a one-element lookup and surface as a downstream
///   "symbol not found" rather than a deserialization error.
fn parse_vec_string(mut value: serde_json::Value) -> Result<Vec<String>, String> {
    for _ in 0..MAX_STRING_UNWRAP_DEPTH {
        match value {
            serde_json::Value::Array(arr) => {
                return arr
                    .into_iter()
                    .map(|v| match v {
                        serde_json::Value::String(s) => Ok(s),
                        other => Err(format!("expected string in array, got {other}")),
                    })
                    .collect();
            }
            serde_json::Value::String(s) => {
                let trimmed = s.trim();
                if trimmed.starts_with('[') {
                    match serde_json::from_str::<serde_json::Value>(trimmed) {
                        Ok(parsed) => {
                            value = parsed;
                            continue;
                        }
                        // Not actually a stringified array — treat as a literal.
                        Err(_) => return Ok(vec![s]),
                    }
                }
                return Ok(vec![s]);
            }
            other => {
                return Err(format!(
                    "expected array or JSON-encoded array string, got {other}"
                ));
            }
        }
    }
    Err(format!(
        "stringified array nesting exceeds max depth of {MAX_STRING_UNWRAP_DEPTH}"
    ))
}

#[derive(Clone)]
pub struct IlluServer {
    db: std::sync::Arc<Mutex<Database>>,
    config: std::sync::Arc<IndexConfig>,
    registry: std::sync::Arc<crate::registry::Registry>,
    tool_router: ToolRouter<Self>,
    last_refresh: std::sync::Arc<Mutex<std::time::Instant>>,
    ra: Option<std::sync::Arc<crate::ra::RaClient>>,
}

impl IlluServer {
    #[must_use]
    pub fn new(db: Database, config: IndexConfig, registry: crate::registry::Registry) -> Self {
        Self {
            db: std::sync::Arc::new(Mutex::new(db)),
            config: std::sync::Arc::new(config),
            registry: std::sync::Arc::new(registry),
            tool_router: Self::tool_router(),
            last_refresh: std::sync::Arc::new(Mutex::new(
                std::time::Instant::now()
                    .checked_sub(REFRESH_COOLDOWN)
                    .unwrap_or(std::time::Instant::now()),
            )),
            ra: None,
        }
    }

    /// Set the rust-analyzer client for LSP-powered tools.
    #[must_use]
    pub fn with_ra(mut self, ra: std::sync::Arc<crate::ra::RaClient>) -> Self {
        self.ra = Some(ra);
        self
    }

    /// Get a reference to the RA client, if available.
    fn ra(&self) -> Result<&crate::ra::RaClient, McpError> {
        self.ra
            .as_deref()
            .ok_or_else(|| McpError::internal_error(
                "rust-analyzer not available — start illu with RA enabled or ensure rust-analyzer is installed",
                None,
            ))
    }

    /// Get RA client and verify it's ready (indexing complete).
    /// Use this for tools that modify code (rename, SSR) where incomplete indexing
    /// could produce wrong results.
    fn require_ra_ready(&self) -> Result<&crate::ra::RaClient, McpError> {
        let ra = self.ra()?;
        if !ra.is_ready() {
            return Err(McpError::internal_error(
                "rust-analyzer is still indexing — please wait and try again",
                None,
            ));
        }
        Ok(ra)
    }

    #[must_use]
    pub fn db(&self) -> std::sync::Arc<Mutex<Database>> {
        self.db.clone()
    }

    fn lock_db(&self) -> Result<MutexGuard<'_, Database>, McpError> {
        self.db
            .lock()
            .map_err(|e| McpError::internal_error(e.to_string(), None))
    }

    /// Run a sync tool handler on the tokio blocking pool so its work
    /// (subprocess invocations, blocking file I/O, long SQL) does not stall
    /// the reactor. The DB lock is acquired inside the blocking task, so no
    /// guard is held while the reactor schedules the work.
    ///
    /// `Box<dyn Error>` is not `Send`, so the error is flattened to a
    /// caused-by chain via `format_error_chain` inside the blocking
    /// closure — preserving information that plain `Display` drops.
    ///
    /// **Trade-off:** every call pays a tokio blocking-pool dispatch
    /// even when the handler only does in-memory SQL. Only migrate a
    /// handler to this path when it shells out to git, touches `std::fs`,
    /// or does genuinely long-running work. Purely-in-memory handlers
    /// (`query`, `tree`, `impact`, …) are intentionally left on the
    /// reactor — the cost of the thread hop outweighs the contention
    /// win. If a handler does unpredictable long SQL (`neighborhood`
    /// with huge graphs, etc.), pick `run_blocking`.
    async fn run_blocking<F>(&self, f: F) -> Result<String, McpError>
    where
        F: FnOnce(&Database, &IndexConfig) -> Result<String, Box<dyn std::error::Error>>
            + Send
            + 'static,
    {
        let db = std::sync::Arc::clone(&self.db);
        let config = std::sync::Arc::clone(&self.config);
        tokio::task::spawn_blocking(move || -> Result<String, String> {
            let guard = db.lock().map_err(|e| e.to_string())?;
            f(&guard, &config).map_err(|e| format_error_chain(&*e))
        })
        .await
        .map_err(|e| McpError::internal_error(e.to_string(), None))?
        .map_err(|e| McpError::internal_error(e, None))
    }

    /// Refresh the index if the cooldown has elapsed. Heavy work
    /// (git-changed-file detection, SQL writes) runs on the blocking
    /// pool so the reactor isn't stalled. The docs fetch is still kicked
    /// off as a normal `tokio::spawn` task because that path is already
    /// async (HTTP) and only touches the DB inside its own
    /// `spawn_blocking`.
    async fn refresh(&self) -> Result<(), McpError> {
        {
            let last = self
                .last_refresh
                .lock()
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            if last.elapsed() < REFRESH_COOLDOWN {
                tracing::debug!("Refresh: skipping (within cooldown)");
                return Ok(());
            }
        }
        tracing::debug!("Refresh: checking for changed files");

        let db = std::sync::Arc::clone(&self.db);
        let config = std::sync::Arc::clone(&self.config);
        let pending_docs = tokio::task::spawn_blocking(move || -> Result<Vec<_>, String> {
            let db = db.lock().map_err(|e| e.to_string())?;
            let refreshed =
                crate::indexer::refresh_index(&db, &config).map_err(|e| format_error_chain(&*e))?;
            if refreshed > 0 {
                tracing::info!(count = refreshed, "Refreshed changed files");
            }
            crate::indexer::docs::pending_docs(&db).map_err(|e| format_error_chain(&*e))
        })
        .await
        .map_err(|e| McpError::internal_error(e.to_string(), None))?
        .map_err(|e| McpError::internal_error(e, None))?;

        if let Ok(mut last) = self.last_refresh.lock() {
            *last = std::time::Instant::now();
        }

        // Fetch docs in background — don't block tool responses.
        if !pending_docs.is_empty() {
            let db = self.db.clone();
            let repo_path = self.config.repo_path.clone();
            tokio::spawn(async move {
                let total = pending_docs.len();
                tracing::info!(count = total, "Fetching docs in background");
                crate::status::set(&format!("fetching docs ▸ 0/{total}"));
                let fetched = crate::indexer::docs::fetch_docs(&pending_docs, &repo_path).await;
                if !fetched.is_empty() {
                    let fetched_count = fetched.len();
                    // Acquire the std Mutex on the blocking pool so the
                    // async reactor isn't parked on a lock held by a
                    // sibling `spawn_blocking` task.
                    let _ = tokio::task::spawn_blocking(move || {
                        let Ok(db) = db.lock() else {
                            tracing::warn!("docs store: failed to acquire DB lock");
                            return;
                        };
                        tracing::info!(count = fetched_count, "Storing fetched docs");
                        let _ = crate::indexer::docs::store_fetched_docs(&db, &fetched);
                    })
                    .await;
                }
                crate::status::set(crate::status::READY);
            });
        }
        Ok(())
    }
}

#[derive(Deserialize, JsonSchema)]
struct QueryParams {
    /// Search term. Use `*` to match all names when filtering by signature, path, or attribute.
    query: String,
    /// Search scope: symbols (default), docs, files, all, `doc_comments`, bodies, strings
    scope: Option<String>,
    kind: Option<String>,
    /// Filter by attribute/derive (e.g. "test", "derive(Serialize)")
    attribute: Option<String>,
    /// Filter by signature pattern (e.g. "&Database", "-> Result")
    signature: Option<String>,
    /// Filter results to files under this path prefix (e.g. "src/db.rs", "src/server/")
    path: Option<String>,
    /// Max number of results to return (default: 50)
    limit: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
struct ContextParams {
    #[serde(alias = "symbol", alias = "name")]
    symbol_name: String,
    /// Return full untruncated source body (default: false)
    full_body: Option<bool>,
    /// Filter results to a specific file path (e.g. "src/db.rs")
    file: Option<String>,
    /// Select specific sections to include: `source`, `callers`, `callees`,
    /// `tested_by`, `traits`, `related`, `docs`. Omit for all sections.
    /// Example: `["source", "docs"]`.
    #[serde(default, deserialize_with = "lenient_opt_vec_string")]
    sections: Option<Vec<String>>,
    /// Filter callers and callees to this path prefix (e.g. "src/" to exclude test callers)
    callers_path: Option<String>,
    /// Exclude test functions from callers/callees (default: false)
    exclude_tests: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct ImpactParams {
    #[serde(alias = "symbol", alias = "name")]
    symbol_name: String,
    /// Max recursion depth (default: 5). Use 1 for direct callers only.
    depth: Option<i64>,
    /// Summarize deep levels by file instead of listing every symbol (default: true).
    /// Set to false for full verbose output at all depths.
    summary: Option<bool>,
    /// Exclude test functions from impact results (default: false)
    exclude_tests: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct DocsParams {
    dependency: String,
    topic: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct OverviewParams {
    path: String,
    /// Include private symbols (default: false, shows only public/pub(crate))
    include_private: Option<bool>,
    /// Max symbols to show (default: all)
    limit: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
struct TreeParams {
    path: String,
}

#[derive(Deserialize, JsonSchema)]
struct DiffImpactParams {
    /// Git ref range (e.g. "HEAD~3..HEAD", "main"). Omit for unstaged changes.
    git_ref: Option<String>,
    /// Only list changed symbols, skip downstream impact analysis (default: false)
    changes_only: Option<bool>,
    /// Skip downstream impact but still show untested changes and related tests (default: false)
    compact: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct CallpathParams {
    /// Source symbol name
    from: String,
    /// Target symbol name
    to: String,
    /// Max search depth (default: 10)
    max_depth: Option<i64>,
    /// Find all paths instead of just the shortest (default: false)
    all_paths: Option<bool>,
    /// Max number of paths when `all_paths=true` (default: 5)
    max_paths: Option<i64>,
    /// Exclude test functions from paths (default: false)
    exclude_tests: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct BatchContextParams {
    /// List of symbol names to get context for. Example: `["foo", "Bar::baz"]`.
    #[serde(alias = "symbol_names", deserialize_with = "lenient_vec_string")]
    symbols: Vec<String>,
    /// Return full untruncated source bodies (default: false)
    full_body: Option<bool>,
    /// Select specific sections: `source`, `callers`, `callees`,
    /// `tested_by`, `traits`, `related`, `docs`. Omit for all.
    /// Example: `["source", "docs"]`.
    #[serde(default, deserialize_with = "lenient_opt_vec_string")]
    sections: Option<Vec<String>>,
}

#[derive(Deserialize, JsonSchema)]
struct UnusedParams {
    /// Filter to files under this path prefix (e.g. "src/server/")
    path: Option<String>,
    /// Filter by symbol kind: function, struct, enum, trait, etc.
    kind: Option<String>,
    /// Include private symbols (default: false, shows only pub/pub(crate))
    include_private: Option<bool>,
    /// Find symbols with no test coverage instead of unused symbols (default: false)
    untested: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct ImplementsParams {
    /// Trait name to find implementors of
    trait_name: Option<String>,
    /// Type name to find trait implementations for
    type_name: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct TypeUsageParams {
    /// Type name to find usages of
    type_name: String,
    /// Filter to files under this path prefix
    path: Option<String>,
    /// Group results by file with counts instead of listing every entry (default: false)
    compact: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct NeighborhoodParams {
    /// Symbol to explore around
    #[serde(alias = "symbol", alias = "name")]
    symbol_name: String,
    /// Max hops in each direction (default: 2)
    depth: Option<i64>,
    /// Direction: "both" (default), "down" (callees only), "up" (callers only)
    direction: Option<String>,
    /// Format: "list" (default flat), "tree" (hierarchical indented)
    format: Option<String>,
    /// Exclude test functions from results (default: false)
    exclude_tests: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct FileGraphParams {
    /// Path prefix to scope the graph (e.g. "src/server/")
    path: String,
}

#[derive(Deserialize, JsonSchema)]
struct SymbolsAtParams {
    /// File path (e.g. "src/db.rs")
    file: String,
    /// Line number to look up
    line: i64,
}

#[derive(Deserialize, JsonSchema)]
struct StatsParams {
    /// Filter to files under this path prefix (default: all)
    path: Option<String>,
    /// Exclude test function references from "Most Referenced" counts (default: false)
    exclude_tests: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct HotspotsParams {
    /// Filter to files under this path prefix
    path: Option<String>,
    /// Max entries per section (default: 10)
    limit: Option<i64>,
    /// Exclude test function references from "Most Referenced" counts (default: false)
    exclude_tests: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct RenamePlanParams {
    /// Symbol name to plan a rename for (supports `Type::method` syntax)
    #[serde(alias = "symbol", alias = "name")]
    symbol_name: String,
}

#[derive(Deserialize, JsonSchema)]
struct SimilarParams {
    /// Symbol to find similar symbols for
    #[serde(alias = "symbol", alias = "name")]
    symbol_name: String,
    /// Filter to files under this path prefix
    path: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct BlameParams {
    /// Symbol name to blame (supports `Type::method` syntax)
    #[serde(alias = "symbol", alias = "name")]
    symbol_name: String,
}

#[derive(Deserialize, JsonSchema)]
struct HistoryParams {
    /// Symbol name (supports `Type::method` syntax)
    #[serde(alias = "symbol", alias = "name")]
    symbol_name: String,
    /// Max commits to show (default: 10)
    max_commits: Option<i64>,
    /// Show code diffs for each commit (default: false)
    show_diff: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct ReferencesParams {
    /// Symbol name to find all references for (supports `Type::method` syntax)
    #[serde(alias = "symbol", alias = "name")]
    symbol_name: String,
    /// Filter results to files under this path prefix
    path: Option<String>,
    /// Exclude test functions from call sites (default: false)
    exclude_tests: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct DocCoverageParams {
    /// Filter to files under this path prefix (default: all)
    path: Option<String>,
    /// Filter by symbol kind: function, struct, enum, trait, etc.
    kind: Option<String>,
    /// Include private symbols (default: false, shows only pub/pub(crate))
    include_private: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct TestImpactParams {
    /// Symbol name to find test coverage for (supports `Type::method` syntax)
    #[serde(alias = "symbol", alias = "name")]
    symbol_name: String,
    /// Max call graph depth to search for tests (default: 5).
    /// Use 1 for direct test callers only, 2-3 for focused results.
    depth: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
struct OrphanedParams {
    /// Filter to files under this path prefix (e.g. "src/server/")
    path: Option<String>,
    /// Filter by symbol kind: function, struct, enum, trait, etc.
    kind: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct BoundaryParams {
    /// Path prefix defining the module boundary (e.g. "src/server/tools/")
    path: String,
}

#[derive(Deserialize, JsonSchema)]
struct HealthParams {}

#[derive(Deserialize, JsonSchema)]
struct CrateGraphParams {}

#[derive(Deserialize, JsonSchema)]
struct CrateImpactParams {
    /// Symbol name to analyze crate-level impact for (supports `Type::method` syntax)
    #[serde(alias = "symbol", alias = "name")]
    symbol_name: String,
}

#[derive(Deserialize, JsonSchema)]
struct GraphExportParams {
    /// Symbol name for call graph export (provide this or `path`, not both)
    #[serde(default, alias = "symbol", alias = "name")]
    symbol_name: Option<String>,
    /// Path prefix for file dependency graph export (provide this or `symbol_name`, not both)
    path: Option<String>,
    /// Max traversal depth for symbol graphs (default: 2)
    depth: Option<i64>,
    /// Direction for symbol graph: "down" (callees only), "up" (callers only),
    /// "both" (default). Only applies to symbol graphs, not file graphs.
    direction: Option<String>,
    /// Output format: "dot" (Graphviz, default), "edges" (compact edge list for AI),
    /// "summary" (node/edge counts, roots, leaves).
    format: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct FreshnessParams {}

#[derive(Deserialize, JsonSchema)]
struct ReposParams {}

#[derive(Deserialize, JsonSchema)]
struct CrossQueryParams {
    /// Search term
    query: String,
    scope: Option<String>,
    kind: Option<String>,
    attribute: Option<String>,
    signature: Option<String>,
    path: Option<String>,
    limit: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
struct CrossImpactParams {
    /// Symbol name (supports `Type::method` syntax)
    #[serde(alias = "symbol", alias = "name")]
    symbol_name: String,

    /// Optional namespace/module filter to reduce cross-crate noise (e.g., "`illu_rs::`")
    filter: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct CrossDepsParams {}

#[derive(Deserialize, JsonSchema)]
struct CrossCallpathParams {
    /// Source symbol name (in current repo)
    from: String,
    /// Target symbol name (in another repo)
    to: String,
    /// Target repo name (optional — searches all if omitted)
    target_repo: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct AxiomsParams {
    /// Search term for axioms
    query: String,
}

// ── RA (rust-analyzer) tool parameter structs ──────────────────────────

#[derive(Deserialize, JsonSchema)]
struct RaPositionParams {
    /// Position as `file:line:col` (1-indexed)
    position: String,
}

#[derive(Deserialize, JsonSchema)]
struct RaCallHierarchyParams {
    /// Position as `file:line:col` (1-indexed)
    position: String,
    /// Direction: "in" (callers), "out" (callees), or "both" (default)
    direction: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct RaRenameParams {
    /// Position as `file:line:col` (1-indexed)
    position: String,
    /// The new name for the symbol
    new_name: String,
}

#[derive(Deserialize, JsonSchema)]
struct RaSafeRenameParams {
    /// Position as `file:line:col` (1-indexed)
    position: String,
    /// The new name for the symbol
    new_name: String,
}

#[derive(Deserialize, JsonSchema)]
struct RaCodeActionsParams {
    /// Position as `file:line:col` (1-indexed)
    position: String,
    /// Filter by action kind (e.g. "quickfix", "refactor")
    kind: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct RaSsrParams {
    /// SSR pattern, e.g. "foo($a) ==>> bar($a)"
    pattern: String,
    /// If true, preview changes without applying (default: false)
    dry_run: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct RaDiagnosticsParams {
    /// File path (optional — shows all diagnostics if omitted)
    file: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct RaFileParams {
    /// File path (relative to workspace root)
    file: String,
}

fn to_mcp_err(e: impl std::fmt::Display) -> McpError {
    McpError::internal_error(e.to_string(), None)
}

/// Parse a `"file:line:col"` string into a [`PositionSpec`], mapping
/// any failure to `McpError` so RA tool handlers can use `?` directly.
fn parse_position(s: &str) -> Result<crate::ra::PositionSpec, McpError> {
    s.parse::<crate::ra::PositionSpec>()
        .map_err(|e: crate::ra::RaError| to_mcp_err(e))
}

/// Build an MCP tool error result from any displayable error. Used by
/// RA tool handlers that prefer to return a tool-error response rather
/// than propagating via `?` (which would surface as a protocol error).
fn ra_error(e: impl std::fmt::Display) -> CallToolResult {
    CallToolResult::error(vec![Content::text(e.to_string())])
}

/// Flatten an error and its `source()` chain to a multi-line string.
/// Used when an error must cross a thread boundary (via `spawn_blocking`)
/// where `Box<dyn Error>` is not `Send` — plain `Display` would drop the
/// causal chain, so we walk it ourselves before serialising.
fn format_error_chain(e: &(dyn std::error::Error + 'static)) -> String {
    let mut out = e.to_string();
    let mut source = e.source();
    while let Some(inner) = source {
        out.push_str("\n  caused by: ");
        out.push_str(&inner.to_string());
        source = inner.source();
    }
    out
}

fn text_result(text: String) -> CallToolResult {
    CallToolResult::success(vec![Content::text(text)])
}

#[tool_router]
impl IlluServer {
    #[tool(
        name = "axioms",
        description = "Query the Rust Axioms database for core rules, safety constraints, and best practices. Use this when unsure about Rust architecture or idioms."
    )]
    async fn axioms(
        &self,
        Parameters(params): Parameters<AxiomsParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(query = %params.query, "Tool call: axioms");
        let _guard = crate::status::StatusGuard::new(&format!("axioms ▸ {}", params.query));
        let result = tools::axioms::handle_axioms(&params.query).map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "query",
        description = "Search the codebase for symbols, documentation, or files. Scope: symbols (default), docs, files, all, doc_comments, bodies, strings. Kind: function, struct, enum, enum_variant, trait, impl, const, static, type_alias, macro (filters symbol results). Use query='*' with signature/path/attribute filters to search without a name."
    )]
    async fn query(
        &self,
        Parameters(params): Parameters<QueryParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(
            query = %params.query,
            scope = ?params.scope,
            kind = ?params.kind,
            "Tool call: query"
        );
        let _guard = crate::status::StatusGuard::new(&format!("query ▸ {}", params.query));
        self.refresh().await?;
        let db = self.lock_db()?;
        let result = tools::query::handle_query(
            &db,
            &params.query,
            params.scope.as_deref(),
            params.kind.as_deref(),
            params.attribute.as_deref(),
            params.signature.as_deref(),
            params.path.as_deref(),
            params.limit,
        )
        .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "context",
        description = "Get full context for a symbol: definition, signature, file location, and related documentation. Supports Type::method syntax (e.g. 'Database::new') and optional file filter."
    )]
    async fn context(
        &self,
        Parameters(params): Parameters<ContextParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(symbol = %params.symbol_name, file = ?params.file, "Tool call: context");
        let _guard = crate::status::StatusGuard::new(&format!("context ▸ {}", params.symbol_name));
        self.refresh().await?;
        let sym = params.symbol_name.clone();
        let file = params.file.clone();
        let sections = params.sections.clone();
        let callers_path = params.callers_path.clone();
        let full_body = params.full_body.unwrap_or(false);
        let exclude_tests = params.exclude_tests.unwrap_or(false);
        let result = self
            .run_blocking(move |db, _cfg| {
                let sections_slice: Option<Vec<&str>> = sections
                    .as_ref()
                    .map(|v| v.iter().map(String::as_str).collect());
                tools::context::handle_context(
                    db,
                    &sym,
                    full_body,
                    file.as_deref(),
                    sections_slice.as_deref(),
                    callers_path.as_deref(),
                    exclude_tests,
                )
            })
            .await?;
        Ok(text_result(result))
    }

    #[tool(
        name = "impact",
        description = "Analyze the impact of changing a symbol by finding all transitive dependents. Use depth=1 for direct callers only."
    )]
    async fn impact(
        &self,
        Parameters(params): Parameters<ImpactParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(symbol = %params.symbol_name, "Tool call: impact");
        let _guard = crate::status::StatusGuard::new(&format!("impact ▸ {}", params.symbol_name));
        self.refresh().await?;
        let db = self.lock_db()?;
        let summary = params.summary.unwrap_or(true);
        let exclude_tests = params.exclude_tests.unwrap_or(false);
        let result = tools::impact::handle_impact(
            &db,
            &params.symbol_name,
            params.depth,
            summary,
            exclude_tests,
        )
        .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "docs",
        description = "Get documentation for a dependency, optionally filtered by topic."
    )]
    async fn docs(
        &self,
        Parameters(params): Parameters<DocsParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(
            dependency = %params.dependency,
            topic = ?params.topic,
            "Tool call: docs"
        );
        let _guard = crate::status::StatusGuard::new(&format!("docs ▸ {}", params.dependency));
        self.refresh().await?;
        let db = self.lock_db()?;
        let result = tools::docs::handle_docs(&db, &params.dependency, params.topic.as_deref())
            .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "overview",
        description = "List public symbols under a file path prefix, grouped by file. Shows name, kind, signature, and first line of doc comment. Set include_private to see all symbols."
    )]
    async fn overview(
        &self,
        Parameters(params): Parameters<OverviewParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(path = %params.path, "Tool call: overview");
        let _guard = crate::status::StatusGuard::new(&format!("overview ▸ {}", params.path));
        self.refresh().await?;
        let db = self.lock_db()?;
        let result = tools::overview::handle_overview(
            &db,
            &params.path,
            params.include_private.unwrap_or(false),
            params.limit,
        )
        .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "tree",
        description = "Show the file/module tree under a path prefix with public symbol counts per file."
    )]
    async fn tree(
        &self,
        Parameters(params): Parameters<TreeParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(path = %params.path, "Tool call: tree");
        let _guard = crate::status::StatusGuard::new(&format!("tree ▸ {}", params.path));
        self.refresh().await?;
        let db = self.lock_db()?;
        let result = tools::tree::handle_tree(&db, &params.path).map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "diff_impact",
        description = "Analyze impact of code changes from a git diff. Shows which symbols were modified and their downstream dependents."
    )]
    async fn diff_impact(
        &self,
        Parameters(params): Parameters<DiffImpactParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(git_ref = ?params.git_ref, "Tool call: diff_impact");
        let _guard = crate::status::StatusGuard::new("diff_impact");
        self.refresh().await?;
        let git_ref = params.git_ref.clone();
        let changes_only = params.changes_only.unwrap_or(false);
        let compact = params.compact.unwrap_or(false);
        let result = self
            .run_blocking(move |db, cfg| {
                tools::diff_impact::handle_diff_impact(
                    db,
                    &cfg.repo_path,
                    git_ref.as_deref(),
                    changes_only,
                    compact,
                )
            })
            .await?;
        Ok(text_result(result))
    }

    #[tool(
        name = "callpath",
        description = "Find call paths between two symbols. By default finds the shortest path. Set all_paths=true to find up to max_paths (default 5) distinct paths via DFS."
    )]
    async fn callpath(
        &self,
        Parameters(params): Parameters<CallpathParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(from = %params.from, to = %params.to, all_paths = ?params.all_paths, "Tool call: callpath");
        let _guard =
            crate::status::StatusGuard::new(&format!("callpath ▸ {} → {}", params.from, params.to));
        self.refresh().await?;
        let db = self.lock_db()?;
        let result = tools::callpath::handle_callpath(
            &db,
            &params.from,
            &params.to,
            params.max_depth,
            params.all_paths.unwrap_or(false),
            params.max_paths,
            params.exclude_tests.unwrap_or(false),
        )
        .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "freshness",
        description = "Check if the index is up to date with the current git HEAD. Shows indexed commit, current HEAD, and any changed files."
    )]
    async fn freshness(
        &self,
        Parameters(_params): Parameters<FreshnessParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!("Tool call: freshness");
        let _guard = crate::status::StatusGuard::new("freshness");
        self.refresh().await?;
        let result = self
            .run_blocking(move |db, cfg| tools::freshness::handle_freshness(db, &cfg.repo_path))
            .await?;
        Ok(text_result(result))
    }

    #[tool(
        name = "batch_context",
        description = "Get full context for multiple symbols in one call. Returns definition, signature, callers, callees, and docs for each symbol."
    )]
    async fn batch_context(
        &self,
        Parameters(params): Parameters<BatchContextParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(symbols = ?params.symbols, "Tool call: batch_context");
        let _guard = crate::status::StatusGuard::new("batch_context");
        self.refresh().await?;
        let db = self.lock_db()?;
        let full_body = params.full_body.unwrap_or(false);
        let sections: Option<Vec<&str>> = params
            .sections
            .as_ref()
            .map(|v| v.iter().map(String::as_str).collect());
        let result = tools::batch_context::handle_batch_context(
            &db,
            &params.symbols,
            full_body,
            sections.as_deref(),
        )
        .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "unused",
        description = "Find potentially unused symbols (no incoming references) or untested symbols (no test coverage). Excludes entry points like main and #[test]. Set untested=true to find symbols with no tests."
    )]
    async fn unused(
        &self,
        Parameters(params): Parameters<UnusedParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(path = ?params.path, kind = ?params.kind, untested = ?params.untested, "Tool call: unused");
        let _guard = crate::status::StatusGuard::new("unused");
        self.refresh().await?;
        let db = self.lock_db()?;
        let result = tools::unused::handle_unused(
            &db,
            params.path.as_deref(),
            params.kind.as_deref(),
            params.include_private.unwrap_or(false),
            params.untested.unwrap_or(false),
        )
        .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "implements",
        description = "Query trait/type relationships. Use trait_name to find all types implementing a trait, type_name to find all traits a type implements, or both to check a specific implementation."
    )]
    async fn implements(
        &self,
        Parameters(params): Parameters<ImplementsParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(trait_name = ?params.trait_name, type_name = ?params.type_name, "Tool call: implements");
        let _guard = crate::status::StatusGuard::new("implements");
        self.refresh().await?;
        let db = self.lock_db()?;
        let result = tools::implements::handle_implements(
            &db,
            params.trait_name.as_deref(),
            params.type_name.as_deref(),
        )
        .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "neighborhood",
        description = "Explore the local call graph around a symbol. Shows callers (upstream) and callees (downstream) within N hops. Only follows function calls (excludes type refs). Use for understanding a symbol's role in the architecture."
    )]
    async fn neighborhood(
        &self,
        Parameters(params): Parameters<NeighborhoodParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(symbol = %params.symbol_name, depth = ?params.depth, "Tool call: neighborhood");
        let _guard =
            crate::status::StatusGuard::new(&format!("neighborhood ▸ {}", params.symbol_name));
        self.refresh().await?;
        let db = self.lock_db()?;
        let result = tools::neighborhood::handle_neighborhood(
            &db,
            &params.symbol_name,
            params.depth,
            params.direction.as_deref(),
            params.format.as_deref(),
            params.exclude_tests.unwrap_or(false),
        )
        .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "type_usage",
        description = "Find where a type is used: as function parameters, return types, and struct fields. Best-effort text search on signatures and struct details."
    )]
    async fn type_usage(
        &self,
        Parameters(params): Parameters<TypeUsageParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(type_name = %params.type_name, "Tool call: type_usage");
        let _guard = crate::status::StatusGuard::new(&format!("type_usage ▸ {}", params.type_name));
        self.refresh().await?;
        let db = self.lock_db()?;
        let result = tools::type_usage::handle_type_usage(
            &db,
            &params.type_name,
            params.path.as_deref(),
            params.compact.unwrap_or(false),
        )
        .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "file_graph",
        description = "Show the file-level dependency graph under a path prefix. Derived from symbol references — shows which files depend on which other files."
    )]
    async fn file_graph(
        &self,
        Parameters(params): Parameters<FileGraphParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(path = %params.path, "Tool call: file_graph");
        let _guard =
            crate::status::StatusGuard::new(&format!("file_graph \u{25b8} {}", params.path));
        self.refresh().await?;
        let db = self.lock_db()?;
        let result = tools::file_graph::handle_file_graph(&db, &params.path).map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "symbols_at",
        description = "Look up which symbol(s) exist at a given file path and line number. Use when navigating from compiler errors, stack traces, or git blame output."
    )]
    async fn symbols_at(
        &self,
        Parameters(params): Parameters<SymbolsAtParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(file = %params.file, line = params.line, "Tool call: symbols_at");
        let _guard = crate::status::StatusGuard::new(&format!(
            "symbols_at \u{25b8} {}:{}",
            params.file, params.line
        ));
        self.refresh().await?;
        let db = self.lock_db()?;
        let result = tools::symbols_at::handle_symbols_at(&db, &params.file, params.line)
            .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "hotspots",
        description = "Identify complexity and coupling hotspots: most-referenced symbols (fragile), most-referencing symbols (complex), and largest functions."
    )]
    async fn hotspots(
        &self,
        Parameters(params): Parameters<HotspotsParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(path = ?params.path, "Tool call: hotspots");
        let _guard = crate::status::StatusGuard::new("hotspots");
        self.refresh().await?;
        let db = self.lock_db()?;
        let result = tools::hotspots::handle_hotspots(
            &db,
            params.path.as_deref(),
            params.limit,
            params.exclude_tests.unwrap_or(false),
        )
        .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "stats",
        description = "Show codebase statistics: file and symbol counts, test coverage ratio, most-referenced symbols, and largest files."
    )]
    async fn stats(
        &self,
        Parameters(params): Parameters<StatsParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(path = ?params.path, "Tool call: stats");
        let _guard = crate::status::StatusGuard::new("stats");
        self.refresh().await?;
        let db = self.lock_db()?;
        let result = tools::stats::handle_stats(
            &db,
            params.path.as_deref(),
            params.exclude_tests.unwrap_or(false),
        )
        .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "rename_plan",
        description = "Preview all locations that would need updating when renaming a symbol. Shows call sites, type usage in signatures, struct fields, trait implementations, and doc comments."
    )]
    async fn rename_plan(
        &self,
        Parameters(params): Parameters<RenamePlanParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(symbol = %params.symbol_name, "Tool call: rename_plan");
        let _guard = crate::status::StatusGuard::new(&format!(
            "rename_plan \u{25b8} {}",
            params.symbol_name
        ));
        self.refresh().await?;
        let db = self.lock_db()?;
        let result =
            tools::rename_plan::handle_rename_plan(&db, &params.symbol_name).map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "similar",
        description = "Find symbols with similar signatures and call patterns. Useful for discovering duplicates, finding patterns to follow, or identifying refactoring candidates."
    )]
    async fn similar(
        &self,
        Parameters(params): Parameters<SimilarParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(symbol = %params.symbol_name, "Tool call: similar");
        let _guard =
            crate::status::StatusGuard::new(&format!("similar \u{25b8} {}", params.symbol_name));
        self.refresh().await?;
        let db = self.lock_db()?;
        let result =
            tools::similar::handle_similar(&db, &params.symbol_name, params.path.as_deref())
                .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "blame",
        description = "Show git blame for a symbol: who last modified it, when, and the commit message. Summarizes authorship across the symbol's line range."
    )]
    async fn blame(
        &self,
        Parameters(params): Parameters<BlameParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(symbol = %params.symbol_name, "Tool call: blame");
        let _guard =
            crate::status::StatusGuard::new(&format!("blame \u{25b8} {}", params.symbol_name));
        self.refresh().await?;
        let sym = params.symbol_name.clone();
        let result = self
            .run_blocking(move |db, cfg| tools::blame::handle_blame(db, &cfg.repo_path, &sym))
            .await?;
        Ok(text_result(result))
    }

    #[tool(
        name = "history",
        description = "Show git commit history for a symbol's line range. Shows who changed it, when, and why — useful for understanding evolution and recent modifications."
    )]
    async fn history(
        &self,
        Parameters(params): Parameters<HistoryParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(symbol = %params.symbol_name, "Tool call: history");
        let _guard =
            crate::status::StatusGuard::new(&format!("history \u{25b8} {}", params.symbol_name));
        self.refresh().await?;
        let sym = params.symbol_name.clone();
        let max_commits = params.max_commits;
        let show_diff = params.show_diff.unwrap_or(false);
        let result = self
            .run_blocking(move |db, cfg| {
                tools::history::handle_history(db, &cfg.repo_path, &sym, max_commits, show_diff)
            })
            .await?;
        Ok(text_result(result))
    }

    #[tool(
        name = "references",
        description = "Unified view of all references to a symbol: call sites, type usage in signatures, and trait implementations. Use for comprehensive impact understanding before renaming or modifying a symbol."
    )]
    async fn references(
        &self,
        Parameters(params): Parameters<ReferencesParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(symbol = %params.symbol_name, "Tool call: references");
        let _guard =
            crate::status::StatusGuard::new(&format!("references \u{25b8} {}", params.symbol_name));
        self.refresh().await?;
        let db = self.lock_db()?;
        let result = tools::references::handle_references(
            &db,
            &params.symbol_name,
            params.path.as_deref(),
            params.exclude_tests.unwrap_or(false),
        )
        .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "doc_coverage",
        description = "Find symbols missing doc comments. Shows coverage percentage and lists undocumented symbols grouped by file. Filter by path, kind, or visibility."
    )]
    async fn doc_coverage(
        &self,
        Parameters(params): Parameters<DocCoverageParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(path = ?params.path, kind = ?params.kind, "Tool call: doc_coverage");
        let _guard = crate::status::StatusGuard::new("doc_coverage");
        self.refresh().await?;
        let db = self.lock_db()?;
        let result = tools::doc_coverage::handle_doc_coverage(
            &db,
            params.path.as_deref(),
            params.kind.as_deref(),
            params.include_private.unwrap_or(false),
        )
        .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "boundary",
        description = "Analyze module boundaries: which symbols are used by code outside the given path (public API) vs only used internally (safe to refactor)."
    )]
    async fn boundary(
        &self,
        Parameters(params): Parameters<BoundaryParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(path = %params.path, "Tool call: boundary");
        let _guard = crate::status::StatusGuard::new(&format!("boundary \u{25b8} {}", params.path));
        self.refresh().await?;
        let db = self.lock_db()?;
        let result = tools::boundary::handle_boundary(&db, &params.path).map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "health",
        description = "Report index quality: ref confidence distribution, signature completeness, noise sources, and coverage metrics."
    )]
    async fn health(
        &self,
        Parameters(_params): Parameters<HealthParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!("Tool call: health");
        let _guard = crate::status::StatusGuard::new("health");
        self.refresh().await?;
        let db = self.lock_db()?;
        let result = tools::health::handle_health(&db).map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "crate_graph",
        description = "Show the workspace crate dependency graph. Lists all crates and their inter-crate dependencies."
    )]
    async fn crate_graph(
        &self,
        Parameters(_params): Parameters<CrateGraphParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!("Tool call: crate_graph");
        let _guard = crate::status::StatusGuard::new("crate_graph");
        self.refresh().await?;
        let db = self.lock_db()?;
        let result = tools::crate_graph::handle_crate_graph(&db).map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "crate_impact",
        description = "Show which workspace crates are affected by changing a symbol. Requires a multi-crate workspace. Shows the defining crate, transitive crate dependents, and symbol-level impact grouped by module."
    )]
    async fn crate_impact(
        &self,
        Parameters(params): Parameters<CrateImpactParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(symbol = %params.symbol_name, "Tool call: crate_impact");
        let _guard = crate::status::StatusGuard::new(&format!(
            "crate_impact \u{25b8} {}",
            params.symbol_name
        ));
        self.refresh().await?;
        let db = self.lock_db()?;
        let result = tools::crate_impact::handle_crate_impact(&db, &params.symbol_name)
            .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "graph_export",
        description = "Export a call graph or file dependency graph. Provide `symbol_name` for a symbol call graph, or `path` for a file dependency graph. Format: \"dot\" (Graphviz, default), \"edges\" (compact A -> B lines for AI), \"summary\" (node/edge counts with roots and leaves)."
    )]
    async fn graph_export(
        &self,
        Parameters(params): Parameters<GraphExportParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(symbol = ?params.symbol_name, path = ?params.path, format = ?params.format, "Tool call: graph_export");
        let _guard = crate::status::StatusGuard::new("graph_export");
        self.refresh().await?;
        let db = self.lock_db()?;
        let result = tools::graph_export::handle_graph_export(
            &db,
            params.symbol_name.as_deref(),
            params.path.as_deref(),
            params.depth,
            params.direction.as_deref(),
            params.format.as_deref(),
        )
        .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "test_impact",
        description = "Show which tests break when changing a symbol. Combines impact analysis with test discovery. Returns test names, locations, and a suggested cargo test command."
    )]
    async fn test_impact(
        &self,
        Parameters(params): Parameters<TestImpactParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(symbol = %params.symbol_name, "Tool call: test_impact");
        let _guard = crate::status::StatusGuard::new(&format!(
            "test_impact \u{25b8} {}",
            params.symbol_name
        ));
        self.refresh().await?;
        let db = self.lock_db()?;
        let result = tools::test_impact::handle_test_impact(&db, &params.symbol_name, params.depth)
            .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "orphaned",
        description = "Find symbols with no callers AND no test coverage — truly dead, untested code. These are safe to remove or should have tests added."
    )]
    async fn orphaned(
        &self,
        Parameters(params): Parameters<OrphanedParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(path = ?params.path, kind = ?params.kind, "Tool call: orphaned");
        let _guard = crate::status::StatusGuard::new("orphaned");
        self.refresh().await?;
        let db = self.lock_db()?;
        let result =
            tools::orphaned::handle_orphaned(&db, params.path.as_deref(), params.kind.as_deref())
                .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "repos",
        description = "List all registered repos with status, symbol counts, and which is the active session repo."
    )]
    async fn repos(
        &self,
        Parameters(_params): Parameters<ReposParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!("Tool call: repos");
        let _guard = crate::status::StatusGuard::new("repos");
        let result = tools::repos::handle_repos(&self.registry, &self.config.repo_path)
            .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "cross_query",
        description = "Search symbols across other registered repos. Same parameters as `query` but searches all repos except the current one."
    )]
    async fn cross_query(
        &self,
        Parameters(params): Parameters<CrossQueryParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(query = %params.query, "Tool call: cross_query");
        let _guard =
            crate::status::StatusGuard::new(&format!("cross_query \u{25b8} {}", params.query));
        let opts = tools::cross_query::CrossQueryOpts {
            query: &params.query,
            scope: params.scope.as_deref(),
            kind: params.kind.as_deref(),
            attribute: params.attribute.as_deref(),
            signature: params.signature.as_deref(),
            path: params.path.as_deref(),
            limit: params.limit,
        };
        let result =
            tools::cross_query::handle_cross_query(&self.registry, &self.config.repo_path, &opts)
                .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "cross_impact",
        description = "Find references to a symbol in other registered repos. Name-based matching across repo boundaries."
    )]
    async fn cross_impact(
        &self,
        Parameters(params): Parameters<CrossImpactParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(symbol = %params.symbol_name, "Tool call: cross_impact");
        let _guard = crate::status::StatusGuard::new(&format!(
            "cross_impact \u{25b8} {}",
            params.symbol_name
        ));
        let result = tools::cross_impact::handle_cross_impact(
            &self.registry,
            &self.config.repo_path,
            &params.symbol_name,
            params.filter.as_deref(),
        )
        .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "cross_deps",
        description = "Show inter-repo dependencies: path deps between registered repos and shared crate dependencies."
    )]
    async fn cross_deps(
        &self,
        Parameters(_params): Parameters<CrossDepsParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!("Tool call: cross_deps");
        let _guard = crate::status::StatusGuard::new("cross_deps");
        let registry = std::sync::Arc::clone(&self.registry);
        let result = tokio::task::spawn_blocking(move || -> Result<String, String> {
            tools::cross_deps::handle_cross_deps(&registry).map_err(|e| format_error_chain(&*e))
        })
        .await
        .map_err(|e| McpError::internal_error(e.to_string(), None))?
        .map_err(|e| McpError::internal_error(e, None))?;
        Ok(text_result(result))
    }

    #[tool(
        name = "cross_callpath",
        description = "Find call chains spanning repos via bridge symbols. Identifies shared symbols between the current repo and target repos that could form a call path."
    )]
    async fn cross_callpath(
        &self,
        Parameters(params): Parameters<CrossCallpathParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(from = %params.from, to = %params.to, "Tool call: cross_callpath");
        let _guard = crate::status::StatusGuard::new(&format!(
            "cross_callpath \u{25b8} {} \u{2192} {}",
            params.from, params.to
        ));
        self.refresh().await?;
        let db = self.lock_db()?;
        let result = tools::cross_callpath::handle_cross_callpath(
            &db,
            &self.registry,
            &self.config.repo_path,
            &params.from,
            &params.to,
            params.target_repo.as_deref(),
        )
        .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    // ── RA (rust-analyzer) powered tools ────────────────────────────────

    #[tool(
        name = "ra_definition",
        description = "Go to compiler-accurate definition of the symbol at file:line:col. Powered by rust-analyzer — resolves through macros, trait impls, and generics."
    )]
    async fn ra_definition(
        &self,
        Parameters(params): Parameters<RaPositionParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(position = %params.position, "Tool call: ra_definition");
        let ra = self.ra()?;
        let spec = parse_position(&params.position)?;
        match ra.definition(&spec).await {
            Ok(locs) => {
                let mut out = String::new();
                if locs.is_empty() {
                    out.push_str("No definition found.");
                } else {
                    for loc in &locs {
                        let rl = crate::ra::types::RichLocation::from_location(loc);
                        std::fmt::Write::write_fmt(
                            &mut out,
                            format_args!("- `{}:{}:{}`\n", rl.file, rl.line, rl.col),
                        )
                        .map_err(to_mcp_err)?;
                    }
                }
                Ok(text_result(out))
            }
            Err(e) => Ok(ra_error(e)),
        }
    }

    #[tool(
        name = "ra_hover",
        description = "Show type information and documentation for the symbol at file:line:col. Powered by rust-analyzer."
    )]
    async fn ra_hover(
        &self,
        Parameters(params): Parameters<RaPositionParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(position = %params.position, "Tool call: ra_hover");
        let ra = self.ra()?;
        let spec = parse_position(&params.position)?;
        match ra.hover(&spec).await {
            Ok(Some(text)) => Ok(text_result(text)),
            Ok(None) => Ok(text_result("No hover information available.".to_string())),
            Err(e) => Ok(ra_error(e)),
        }
    }

    #[tool(
        name = "ra_diagnostics",
        description = "Show compilation errors and warnings from rust-analyzer. Optionally filter to a specific file."
    )]
    async fn ra_diagnostics(
        &self,
        Parameters(params): Parameters<RaDiagnosticsParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(file = ?params.file, "Tool call: ra_diagnostics");
        let ra = self.ra()?;
        if let Some(path) = params.file {
            let path = std::path::PathBuf::from(&path);
            match ra.get_diagnostics(&path) {
                Ok(diags) => {
                    let json = serde_json::to_string_pretty(&diags).unwrap_or_default();
                    Ok(text_result(format!("```json\n{json}\n```")))
                }
                Err(e) => Ok(ra_error(e)),
            }
        } else {
            let all = ra.get_all_diagnostics();
            let json = serde_json::to_string_pretty(&all).unwrap_or_default();
            Ok(text_result(format!("```json\n{json}\n```")))
        }
    }

    #[tool(
        name = "ra_call_hierarchy",
        description = "Show callers and/or callees for the symbol at file:line:col. Direction: 'in' (callers), 'out' (callees), or 'both' (default). Powered by rust-analyzer."
    )]
    async fn ra_call_hierarchy(
        &self,
        Parameters(params): Parameters<RaCallHierarchyParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(position = %params.position, "Tool call: ra_call_hierarchy");
        let ra = self.ra()?;
        let spec = parse_position(&params.position)?;
        let items = match ra.prepare_call_hierarchy(&spec).await {
            Ok(items) => items,
            Err(e) => return Ok(ra_error(e)),
        };
        if items.is_empty() {
            return Ok(text_result("No call hierarchy item found.".to_string()));
        }
        let Some(item) = items.into_iter().next() else {
            return Ok(text_result("No call hierarchy item found.".to_string()));
        };
        let dir = params.direction.unwrap_or_else(|| "both".to_string());
        let mut out = String::new();

        if dir == "in" || dir == "both" {
            match ra.incoming_calls(item.clone()).await {
                Ok(calls) => {
                    out.push_str("## Incoming Calls (Callers)\n");
                    for call in &calls {
                        std::fmt::Write::write_fmt(
                            &mut out,
                            format_args!(
                                "- `{}` ({}) at `{}:{}`\n",
                                call.name, call.kind, call.file, call.line
                            ),
                        )
                        .map_err(to_mcp_err)?;
                    }
                    out.push('\n');
                }
                Err(e) => return Ok(ra_error(e)),
            }
        }
        if dir == "out" || dir == "both" {
            match ra.outgoing_calls(item).await {
                Ok(calls) => {
                    out.push_str("## Outgoing Calls (Callees)\n");
                    for call in &calls {
                        std::fmt::Write::write_fmt(
                            &mut out,
                            format_args!(
                                "- `{}` ({}) at `{}:{}`\n",
                                call.name, call.kind, call.file, call.line
                            ),
                        )
                        .map_err(to_mcp_err)?;
                    }
                    out.push('\n');
                }
                Err(e) => return Ok(ra_error(e)),
            }
        }

        Ok(text_result(out))
    }

    #[tool(
        name = "ra_type_hierarchy",
        description = "Show supertypes (traits implemented) and subtypes for the type at file:line:col. Powered by rust-analyzer."
    )]
    async fn ra_type_hierarchy(
        &self,
        Parameters(params): Parameters<RaPositionParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(position = %params.position, "Tool call: ra_type_hierarchy");
        let ra = self.ra()?;
        let spec = parse_position(&params.position)?;
        let items = match ra.prepare_type_hierarchy(&spec).await {
            Ok(items) => items,
            Err(e) => return Ok(ra_error(e)),
        };
        if items.is_empty() {
            return Ok(text_result("No type hierarchy item found.".to_string()));
        }
        let Some(item) = items.into_iter().next() else {
            return Ok(text_result("No type hierarchy item found.".to_string()));
        };
        let mut out = String::new();

        match ra.supertypes(item.clone()).await {
            Ok(supertypes) if !supertypes.is_empty() => {
                out.push_str("## Supertypes\n");
                for sym in &supertypes {
                    std::fmt::Write::write_fmt(
                        &mut out,
                        format_args!(
                            "- **{}** `{}` ({}:{})\n",
                            sym.kind, sym.name, sym.file, sym.line
                        ),
                    )
                    .map_err(to_mcp_err)?;
                }
                out.push('\n');
            }
            Err(e) => return Ok(ra_error(e)),
            _ => {}
        }

        match ra.subtypes(item).await {
            Ok(subtypes) if !subtypes.is_empty() => {
                out.push_str("## Subtypes\n");
                for sym in &subtypes {
                    std::fmt::Write::write_fmt(
                        &mut out,
                        format_args!(
                            "- **{}** `{}` ({}:{})\n",
                            sym.kind, sym.name, sym.file, sym.line
                        ),
                    )
                    .map_err(to_mcp_err)?;
                }
                out.push('\n');
            }
            Err(e) => return Ok(ra_error(e)),
            _ => {}
        }

        if out.is_empty() {
            out.push_str("No supertypes or subtypes found.");
        }
        Ok(text_result(out))
    }

    #[tool(
        name = "ra_rename",
        description = "Preview the impact of renaming a symbol. Shows affected files and reference counts. Use ra_safe_rename to actually apply the rename."
    )]
    async fn ra_rename(
        &self,
        Parameters(params): Parameters<RaRenameParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(position = %params.position, new_name = %params.new_name, "Tool call: ra_rename");
        let ra = self.require_ra_ready()?;
        let spec = parse_position(&params.position)?;
        match ra.rename_preview(&spec, &params.new_name).await {
            Ok(impact) => {
                let mut out = format!(
                    "## Rename Preview: `{}` → `{}`\n\n**Total references:** {}\n**Files affected:** {}\n\n",
                    impact.old_name,
                    impact.new_name,
                    impact.total_references,
                    impact.files_affected.len()
                );
                for file_ref in &impact.references_by_file {
                    std::fmt::Write::write_fmt(
                        &mut out,
                        format_args!("- `{}` ({} references)\n", file_ref.file, file_ref.count),
                    )
                    .map_err(to_mcp_err)?;
                }
                Ok(text_result(out))
            }
            Err(e) => Ok(ra_error(e)),
        }
    }

    #[tool(
        name = "ra_safe_rename",
        description = "Safe rename: previews impact, applies the rename, and checks for new compilation errors. Powered by rust-analyzer."
    )]
    async fn ra_safe_rename(
        &self,
        Parameters(params): Parameters<RaSafeRenameParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(position = %params.position, new_name = %params.new_name, "Tool call: ra_safe_rename");
        let ra = self.require_ra_ready()?;
        let spec = parse_position(&params.position)?;
        match ra.safe_rename(&spec, &params.new_name).await {
            Ok(result) => {
                let mut out = format!(
                    "## Rename Complete: `{}` → `{}`\n\n**Edits applied:** {}\n**Files changed:** {}\n\n",
                    result.old_name,
                    result.new_name,
                    result.total_edits,
                    result.files_changed.len()
                );
                for file in &result.files_changed {
                    let _ = writeln!(out, "- `{file}`");
                }
                if !result.new_diagnostics.is_empty() {
                    out.push_str("\n### New Diagnostics\n");
                    for diag in &result.new_diagnostics {
                        std::fmt::Write::write_fmt(
                            &mut out,
                            format_args!(
                                "- **{}** `{}:{}`: {}\n",
                                diag.severity, diag.file, diag.line, diag.message
                            ),
                        )
                        .map_err(to_mcp_err)?;
                    }
                }
                Ok(text_result(out))
            }
            Err(e) => Ok(ra_error(e)),
        }
    }

    #[tool(
        name = "ra_code_actions",
        description = "List available code actions (quick fixes, refactors) at file:line:col. Optionally filter by kind (e.g. 'quickfix', 'refactor'). Powered by rust-analyzer."
    )]
    async fn ra_code_actions(
        &self,
        Parameters(params): Parameters<RaCodeActionsParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(position = %params.position, "Tool call: ra_code_actions");
        let ra = self.ra()?;
        let spec = parse_position(&params.position)?;
        match ra.code_actions(&spec, params.kind.as_deref()).await {
            Ok(actions) => {
                let mut out = String::new();
                if actions.is_empty() {
                    out.push_str("No code actions available.");
                } else {
                    for action in &actions {
                        match action {
                            lsp_types::CodeActionOrCommand::CodeAction(ca) => {
                                let kind = ca.kind.as_ref().map_or("", |k| k.as_str());
                                std::fmt::Write::write_fmt(
                                    &mut out,
                                    format_args!("- **{}** [{}]\n", ca.title, kind),
                                )
                                .map_err(to_mcp_err)?;
                            }
                            lsp_types::CodeActionOrCommand::Command(cmd) => {
                                std::fmt::Write::write_fmt(
                                    &mut out,
                                    format_args!("- {}\n", cmd.title),
                                )
                                .map_err(to_mcp_err)?;
                            }
                        }
                    }
                }
                Ok(text_result(out))
            }
            Err(e) => Ok(ra_error(e)),
        }
    }

    #[tool(
        name = "ra_expand_macro",
        description = "Expand the macro at file:line:col, showing the generated Rust code. Powered by rust-analyzer."
    )]
    async fn ra_expand_macro(
        &self,
        Parameters(params): Parameters<RaPositionParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(position = %params.position, "Tool call: ra_expand_macro");
        let ra = self.ra()?;
        let spec = parse_position(&params.position)?;
        match ra.expand_macro(&spec).await {
            Ok(Some(expanded)) => {
                let out = format!(
                    "## Macro: {}\n```rust\n{}\n```",
                    expanded.name, expanded.expansion
                );
                Ok(text_result(out))
            }
            Ok(None) => Ok(text_result("No macro at this position.".to_string())),
            Err(e) => Ok(ra_error(e)),
        }
    }

    #[tool(
        name = "ra_ssr",
        description = "Structural search and replace using rust-analyzer's SSR engine. Pattern format: 'foo($a) ==>> bar($a)'. Set dry_run=true to preview."
    )]
    async fn ra_ssr(
        &self,
        Parameters(params): Parameters<RaSsrParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(pattern = %params.pattern, "Tool call: ra_ssr");
        let ra = self.require_ra_ready()?;
        match ra.ssr(&params.pattern).await {
            Ok(edit) => {
                if params.dry_run.unwrap_or(false) {
                    let json = serde_json::to_string_pretty(&edit).unwrap_or_default();
                    Ok(text_result(format!("```json\n{json}\n```")))
                } else {
                    match crate::ra::ops::apply_workspace_edit(&edit).await {
                        Ok(changed) => {
                            let mut out = String::from("## SSR Applied\nFiles changed:\n");
                            for f in &changed {
                                let _ = writeln!(out, "  - {f}");
                            }
                            Ok(text_result(out))
                        }
                        Err(e) => Ok(ra_error(e)),
                    }
                }
            }
            Err(e) => Ok(ra_error(e)),
        }
    }

    #[tool(
        name = "ra_context",
        description = "Get full compiler-accurate context for a symbol at file:line:col: definition, hover, callers, callees, implementations, and related tests. Powered by rust-analyzer."
    )]
    async fn ra_context(
        &self,
        Parameters(params): Parameters<RaPositionParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(position = %params.position, "Tool call: ra_context");
        let ra = self.ra()?;
        let spec = parse_position(&params.position)?;
        match ra.symbol_context(&spec).await {
            Ok(ctx) => {
                let mut out = format!("## {}\n\n", ctx.name);
                if let Some(hover) = &ctx.hover {
                    let _ = writeln!(out, "{hover}\n");
                }
                let _ = writeln!(
                    out,
                    "**Definition:** `{}:{}:{}`",
                    ctx.definition.file, ctx.definition.line, ctx.definition.col
                );
                if !ctx.definition.text.is_empty() {
                    let _ = writeln!(out, "```rust\n{}\n```\n", ctx.definition.text);
                }
                let _ = writeln!(out, "**References:** {}\n", ctx.reference_count);
                if !ctx.incoming_calls.is_empty() {
                    out.push_str("### Callers\n");
                    for call in &ctx.incoming_calls {
                        let _ = writeln!(
                            out,
                            "- `{}` ({}) at `{}:{}`",
                            call.name, call.kind, call.file, call.line
                        );
                    }
                    out.push('\n');
                }
                if !ctx.outgoing_calls.is_empty() {
                    out.push_str("### Callees\n");
                    for call in &ctx.outgoing_calls {
                        let _ = writeln!(
                            out,
                            "- `{}` ({}) at `{}:{}`",
                            call.name, call.kind, call.file, call.line
                        );
                    }
                    out.push('\n');
                }
                if !ctx.implementations.is_empty() {
                    out.push_str("### Implementations\n");
                    for loc in &ctx.implementations {
                        let _ = writeln!(out, "- `{}:{}:{}`", loc.file, loc.line, loc.col);
                    }
                    out.push('\n');
                }
                if !ctx.related_tests.is_empty() {
                    out.push_str("### Related Tests\n");
                    for test in &ctx.related_tests {
                        let _ = writeln!(out, "- `{test}`");
                    }
                    out.push('\n');
                }
                Ok(text_result(out))
            }
            Err(e) => Ok(ra_error(e)),
        }
    }

    #[tool(
        name = "ra_syntax_tree",
        description = "Show the syntax tree for a file. Useful for debugging macro expansion and understanding parse structure. Powered by rust-analyzer."
    )]
    async fn ra_syntax_tree(
        &self,
        Parameters(params): Parameters<RaFileParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(file = %params.file, "Tool call: ra_syntax_tree");
        let ra = self.ra()?;
        let path = std::path::PathBuf::from(&params.file);
        match ra.syntax_tree(&path).await {
            Ok(tree) => Ok(text_result(tree)),
            Err(e) => Ok(ra_error(e)),
        }
    }

    #[tool(
        name = "ra_related_tests",
        description = "Find tests related to the symbol at file:line:col. Uses rust-analyzer's test discovery — more accurate than text-based matching."
    )]
    async fn ra_related_tests(
        &self,
        Parameters(params): Parameters<RaPositionParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(position = %params.position, "Tool call: ra_related_tests");
        let ra = self.ra()?;
        let spec = parse_position(&params.position)?;
        match ra.related_tests(&spec).await {
            Ok(tests) => {
                if tests.is_empty() {
                    Ok(text_result("No related tests found.".to_string()))
                } else {
                    let mut out = String::new();
                    for test in &tests {
                        std::fmt::Write::write_fmt(
                            &mut out,
                            format_args!("- {}\n", test.runnable.label),
                        )
                        .map_err(to_mcp_err)?;
                    }
                    Ok(text_result(out))
                }
            }
            Err(e) => Ok(ra_error(e)),
        }
    }
}

#[tool_handler]
impl ServerHandler for IlluServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "illu-rs: Code intelligence server for Rust projects. \
             Use 'query' to search (supports attribute and signature filters), \
             'context' for symbol details (includes source body, doc comments, \
             struct fields, trait impls, and callees), \
             'batch_context' for multiple symbols at once, \
             'impact' for single-symbol change analysis, \
             'diff_impact' for git diff-based batch impact analysis, \
             'callpath' to find shortest call chain between two symbols, \
             'unused' to find potentially dead code, \
             'freshness' to check index staleness, \
             'docs' for dependency docs, \
             'overview' for structural maps, \
             'tree' for file/module tree, \
             'implements' for trait/type relationships, \
             'neighborhood' for bidirectional call graph exploration, \
             'type_usage' for finding type usage in signatures and fields, \
             'file_graph' for file-level dependency visualization, \
             'symbols_at' for file:line symbol lookup, \
             'hotspots' for complexity and coupling analysis, \
             'stats' for codebase statistics and health metrics, \
             'rename_plan' for rename impact preview, \
             'similar' for finding structurally similar symbols, \
             'blame' for git blame on symbols, \
             'history' for git commit history on symbols, \
             'references' for unified view of all symbol references, \
             'doc_coverage' for finding undocumented symbols, \
             'boundary' for module API boundary analysis, \
             'health' for index quality diagnosis, \
             'crate_graph' for workspace dependency visualization, \
             'crate_impact' for cross-crate symbol impact in workspaces, \
             'graph_export' for DOT/Graphviz export of call or file graphs, \
             'test_impact' for finding which tests break when changing a symbol, \
             'orphaned' for finding symbols with no callers and no test coverage, \
             'repos' for listing registered repos with status, \
             'cross_query' for searching symbols across all registered repos, \
             'cross_impact' for finding cross-repo references to a symbol, \
             'cross_deps' for showing inter-repo dependency relationships, \
             'cross_callpath' for finding call chains spanning repo boundaries. \
             When rust-analyzer is available, additional 'ra_*' tools provide compiler-accurate intelligence: \
             'ra_definition' for precise go-to-definition, \
             'ra_hover' for type info and docs, \
             'ra_diagnostics' for compilation errors, \
             'ra_call_hierarchy' for compiler-accurate callers/callees, \
             'ra_type_hierarchy' for supertypes/subtypes, \
             'ra_rename' for safe workspace-wide rename, \
             'ra_safe_rename' for rename with diagnostic verification, \
             'ra_code_actions' for quick fixes and refactors, \
             'ra_expand_macro' for macro expansion, \
             'ra_ssr' for structural search and replace, \
             'ra_context' for full compiler-accurate symbol context, \
             'ra_syntax_tree' for debugging parse structure, \
             'ra_related_tests' for compiler-accurate test discovery.",
        )
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod lenient_deser_tests {
    use super::{BatchContextParams, ContextParams};

    #[test]
    fn batch_context_accepts_real_arrays() {
        let params: BatchContextParams = serde_json::from_value(serde_json::json!({
            "symbols": ["foo", "Bar::baz"],
            "sections": ["source", "docs"],
        }))
        .unwrap();
        assert_eq!(params.symbols, vec!["foo", "Bar::baz"]);
        assert_eq!(
            params.sections.unwrap(),
            vec!["source".to_string(), "docs".to_string()]
        );
    }

    #[test]
    fn batch_context_accepts_stringified_arrays() {
        // Caller double-encoded both fields as JSON strings.
        let params: BatchContextParams = serde_json::from_value(serde_json::json!({
            "symbols": "[\"foo\", \"Bar::baz\"]",
            "sections": "[\"source\", \"docs\"]",
        }))
        .unwrap();
        assert_eq!(params.symbols, vec!["foo", "Bar::baz"]);
        assert_eq!(
            params.sections.unwrap(),
            vec!["source".to_string(), "docs".to_string()]
        );
    }

    #[test]
    fn batch_context_accepts_symbol_names_alias() {
        let params: BatchContextParams = serde_json::from_value(serde_json::json!({
            "symbol_names": ["foo", "bar"],
        }))
        .unwrap();
        assert_eq!(params.symbols, vec!["foo", "bar"]);
        assert!(params.sections.is_none());
    }

    #[test]
    fn batch_context_accepts_bare_string_as_single_element() {
        let params: BatchContextParams = serde_json::from_value(serde_json::json!({
            "symbols": "foo",
            "sections": "source",
        }))
        .unwrap();
        assert_eq!(params.symbols, vec!["foo"]);
        assert_eq!(params.sections.unwrap(), vec!["source".to_string()]);
    }

    #[test]
    fn context_sections_accepts_stringified_array() {
        let params: ContextParams = serde_json::from_value(serde_json::json!({
            "symbol_name": "foo",
            "sections": "[\"source\"]",
        }))
        .unwrap();
        assert_eq!(params.sections.unwrap(), vec!["source".to_string()]);
    }

    #[test]
    fn context_sections_null_is_none() {
        let params: ContextParams = serde_json::from_value(serde_json::json!({
            "symbol_name": "foo",
            "sections": serde_json::Value::Null,
        }))
        .unwrap();
        assert!(params.sections.is_none());
    }

    #[test]
    fn batch_context_rejects_non_string_array_elements() {
        let result = serde_json::from_value::<BatchContextParams>(serde_json::json!({
            "symbols": [1, 2],
        }));
        let Err(err) = result else {
            unreachable!("expected error for non-string array elements")
        };
        assert!(
            err.to_string().contains("expected string in array"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn bare_string_starting_with_bracket_is_single_element() {
        // A legitimate symbol name like `[u8]::len` starts with `[` but is
        // not a stringified array. It should fall back to a single element
        // rather than erroring.
        let params: BatchContextParams = serde_json::from_value(serde_json::json!({
            "symbols": "[u8]::len",
        }))
        .unwrap();
        assert_eq!(params.symbols, vec!["[u8]::len"]);
    }

    #[test]
    fn empty_stringified_array() {
        let params: BatchContextParams = serde_json::from_value(serde_json::json!({
            "symbols": "[]",
        }))
        .unwrap();
        assert!(params.symbols.is_empty());
    }

    #[test]
    fn stringified_array_with_surrounding_whitespace() {
        let params: BatchContextParams = serde_json::from_value(serde_json::json!({
            "symbols": "  [\"foo\", \"bar\"]  ",
        }))
        .unwrap();
        assert_eq!(params.symbols, vec!["foo", "bar"]);
    }

    #[test]
    fn unicode_symbol_name() {
        let params: BatchContextParams = serde_json::from_value(serde_json::json!({
            "symbols": ["λ::μ", "Δέλτα"],
        }))
        .unwrap();
        assert_eq!(params.symbols, vec!["λ::μ", "Δέλτα"]);
    }

    #[test]
    fn context_params_accepts_symbol_alias() {
        // Callers that guess `symbol` instead of `symbol_name` still work.
        let params: ContextParams = serde_json::from_value(serde_json::json!({
            "symbol": "foo",
        }))
        .unwrap();
        assert_eq!(params.symbol_name, "foo");
    }

    #[test]
    fn context_params_accepts_name_alias() {
        let params: ContextParams = serde_json::from_value(serde_json::json!({
            "name": "foo",
        }))
        .unwrap();
        assert_eq!(params.symbol_name, "foo");
    }
}
