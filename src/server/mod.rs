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
}

#[derive(Deserialize, JsonSchema)]
struct ContextParams {
    symbol_name: String,
    /// Return full untruncated source body (default: false)
    full_body: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct ImpactParams {
    symbol_name: String,
}

#[derive(Deserialize, JsonSchema)]
struct DocsParams {
    dependency: String,
    topic: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct OverviewParams {
    path: String,
}

#[derive(Deserialize, JsonSchema)]
struct TreeParams {
    path: String,
}

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
        description = "Search the codebase for symbols, documentation, or files. Scope: symbols, docs, files, or all (default). Kind: function, struct, enum, trait, impl, const, static, type_alias, macro (filters symbol results)."
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
        )
        .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "context",
        description = "Get full context for a symbol: definition, signature, file location, and related documentation."
    )]
    async fn context(
        &self,
        Parameters(params): Parameters<ContextParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(symbol = %params.symbol_name, "Tool call: context");
        let _guard = crate::status::StatusGuard::new(&format!("context ▸ {}", params.symbol_name));
        self.refresh()?;
        let db = self.lock_db()?;
        let full_body = params.full_body.unwrap_or(false);
        let result = tools::context::handle_context(&db, &params.symbol_name, full_body)
            .map_err(to_mcp_err)?;
        Ok(text_result(result))
    }

    #[tool(
        name = "impact",
        description = "Analyze the impact of changing a symbol by finding all transitive dependents."
    )]
    async fn impact(
        &self,
        Parameters(params): Parameters<ImpactParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(symbol = %params.symbol_name, "Tool call: impact");
        let _guard = crate::status::StatusGuard::new(&format!("impact ▸ {}", params.symbol_name));
        self.refresh()?;
        let db = self.lock_db()?;
        let result = tools::impact::handle_impact(&db, &params.symbol_name).map_err(to_mcp_err)?;
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
        description = "List public symbols under a file path prefix, grouped by file. Shows name, kind, signature, and first line of doc comment."
    )]
    async fn overview(
        &self,
        Parameters(params): Parameters<OverviewParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::info!(path = %params.path, "Tool call: overview");
        let _guard = crate::status::StatusGuard::new(&format!("overview ▸ {}", params.path));
        self.refresh()?;
        let db = self.lock_db()?;
        let result = tools::overview::handle_overview(&db, &params.path).map_err(to_mcp_err)?;
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
}

#[tool_handler]
impl ServerHandler for IlluServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "illu-rs: Code intelligence server for Rust projects. \
                 Use 'query' to search, 'context' for symbol details \
                 (includes source body, doc comments, struct fields, \
                 trait impls, and callees), 'impact' for change analysis, \
                 'docs' for dependency docs, 'overview' for structural maps."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}
