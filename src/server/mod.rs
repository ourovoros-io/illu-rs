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
use std::sync::{Mutex, MutexGuard};

#[derive(Clone)]
pub struct IlluServer {
    db: std::sync::Arc<Mutex<Database>>,
    config: std::sync::Arc<IndexConfig>,
    tool_router: ToolRouter<Self>,
}

impl IlluServer {
    #[must_use]
    pub fn new(db: Database, config: IndexConfig) -> Self {
        Self {
            db: std::sync::Arc::new(Mutex::new(db)),
            config: std::sync::Arc::new(config),
            tool_router: Self::tool_router(),
        }
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

    fn refresh(&self) -> Result<(), McpError> {
        tracing::debug!("Refresh: checking for changed files");
        let pending_docs = {
            let db = self.lock_db()?;
            let refreshed = crate::indexer::refresh_index(&db, &self.config).map_err(to_mcp_err)?;
            if refreshed > 0 {
                tracing::info!(count = refreshed, "Refreshed changed files");
            }
            crate::indexer::docs::pending_docs(&db).map_err(to_mcp_err)?
        }; // lock dropped

        // Fetch docs in background — don't block tool responses
        if !pending_docs.is_empty() {
            let db = self.db.clone();
            let repo_path = self.config.repo_path.clone();
            tokio::spawn(async move {
                let total = pending_docs.len();
                tracing::info!(count = total, "Fetching docs in background");
                crate::status::set(&format!("fetching docs ▸ 0/{total}"));
                let fetched = crate::indexer::docs::fetch_docs(&pending_docs, &repo_path).await;
                if !fetched.is_empty() {
                    let Ok(db) = db.lock() else { return };
                    tracing::info!(count = fetched.len(), "Storing fetched docs");
                    let _ = crate::indexer::docs::store_fetched_docs(&db, &fetched);
                }
                crate::status::set(crate::status::READY);
            });
        }
        Ok(())
    }
}

#[derive(Deserialize, JsonSchema)]
struct QueryParams {
    query: String,
    scope: Option<String>,
    kind: Option<String>,
    /// Filter by attribute/derive (e.g. "test", "derive(Serialize)")
    attribute: Option<String>,
    /// Filter by signature pattern (e.g. "&Database", "-> Result")
    signature: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct ContextParams {
    symbol_name: String,
    /// Return full untruncated source body (default: false)
    full_body: Option<bool>,
    /// Filter results to a specific file path (e.g. "src/db.rs")
    file: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct ImpactParams {
    symbol_name: String,
    /// Max recursion depth (default: 5). Use 1 for direct callers only.
    depth: Option<i64>,
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
}

#[derive(Deserialize, JsonSchema)]
struct TreeParams {
    path: String,
}

#[derive(Deserialize, JsonSchema)]
struct DiffImpactParams {
    /// Git ref range (e.g. "HEAD~3..HEAD", "main"). Omit for unstaged changes.
    git_ref: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct CallpathParams {
    /// Source symbol name
    from: String,
    /// Target symbol name
    to: String,
    /// Max search depth (default: 10)
    max_depth: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
struct BatchContextParams {
    /// List of symbol names to get context for
    symbols: Vec<String>,
    /// Return full untruncated source bodies (default: false)
    full_body: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct UnusedParams {
    /// Filter to files under this path prefix (e.g. "src/server/")
    path: Option<String>,
    /// Filter by symbol kind: function, struct, enum, trait, etc.
    kind: Option<String>,
    /// Include private symbols (default: false, shows only pub/pub(crate))
    include_private: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct CrateGraphParams {}

#[derive(Deserialize, JsonSchema)]
struct FreshnessParams {}

fn to_mcp_err(e: impl std::fmt::Display) -> McpError {
    McpError::internal_error(e.to_string(), None)
}

fn text_result(text: String) -> CallToolResult {
    CallToolResult::success(vec![Content::text(text)])
}

#[tool_router]
impl IlluServer {
    #[tool(
        name = "query",
        description = "Search the codebase for symbols, documentation, or files. Scope: symbols, docs, files, or all (default). Kind: function, struct, enum, enum_variant, trait, impl, const, static, type_alias, macro (filters symbol results)."
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
        self.refresh()?;
        let db = self.lock_db()?;
        let result = tools::query::handle_query(
            &db,
            &params.query,
            params.scope.as_deref(),
            params.kind.as_deref(),
            params.attribute.as_deref(),
            params.signature.as_deref(),
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
        self.refresh()?;
        let db = self.lock_db()?;
        let full_body = params.full_body.unwrap_or(false);
        let result = tools::context::handle_context(
            &db,
            &params.symbol_name,
            full_body,
            params.file.as_deref(),
        )
        .map_err(to_mcp_err)?;
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
        self.refresh()?;
        let db = self.lock_db()?;
        let result = tools::impact::handle_impact(&db, &params.symbol_name, params.depth)
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
        self.refresh()?;
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
        self.refresh()?;
        let db = self.lock_db()?;
        let result = tools::overview::handle_overview(&db, &params.path, params.include_private.unwrap_or(false)).map_err(to_mcp_err)?;
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
        self.refresh()?;
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
        self.refresh()?;
        let db = self.lock_db()?;
        let repo_path = &self.config.repo_path;
        let result =
            tools::diff_impact::handle_diff_impact(&db, repo_path, params.git_ref.as_deref())
                .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "callpath",
        description = "Find the shortest call path between two symbols. Shows how function A reaches function B through the call graph."
    )]
    async fn callpath(
        &self,
        Parameters(params): Parameters<CallpathParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(from = %params.from, to = %params.to, "Tool call: callpath");
        let _guard = crate::status::StatusGuard::new(
            &format!("callpath ▸ {} → {}", params.from, params.to),
        );
        self.refresh()?;
        let db = self.lock_db()?;
        let result = tools::callpath::handle_callpath(
            &db, &params.from, &params.to, params.max_depth,
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
        let db = self.lock_db()?;
        let repo_path = &self.config.repo_path;
        let result = tools::freshness::handle_freshness(&db, repo_path)
            .map_err(to_mcp_err)?;
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
        self.refresh()?;
        let db = self.lock_db()?;
        let full_body = params.full_body.unwrap_or(false);
        let result = tools::batch_context::handle_batch_context(
            &db, &params.symbols, full_body,
        )
        .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "unused",
        description = "Find potentially unused symbols (no incoming references). Excludes entry points like main and #[test]. Useful for dead code detection."
    )]
    async fn unused(
        &self,
        Parameters(params): Parameters<UnusedParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(path = ?params.path, kind = ?params.kind, "Tool call: unused");
        let _guard = crate::status::StatusGuard::new("unused");
        self.refresh()?;
        let db = self.lock_db()?;
        let result = tools::unused::handle_unused(
            &db,
            params.path.as_deref(),
            params.kind.as_deref(),
            params.include_private.unwrap_or(false),
        )
        .map_err(to_mcp_err)?;
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
        self.refresh()?;
        let db = self.lock_db()?;
        let result = tools::crate_graph::handle_crate_graph(&db)
            .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }
}

#[tool_handler]
impl ServerHandler for IlluServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
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
                 'crate_graph' for workspace dependency visualization."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}
