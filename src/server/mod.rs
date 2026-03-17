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

    fn lock_db(&self) -> Result<MutexGuard<'_, Database>, McpError> {
        self.db
            .lock()
            .map_err(|e| McpError::internal_error(e.to_string(), None))
    }

    async fn refresh(&self) -> Result<(), McpError> {
        // Phase 1 (sync): re-index changed files, collect pending docs
        let pending_docs = {
            let db = self.lock_db()?;
            crate::indexer::refresh_index(&db, &self.config).map_err(to_mcp_err)?;
            crate::indexer::docs::pending_docs(&db).map_err(to_mcp_err)?
        }; // lock dropped

        if pending_docs.is_empty() {
            return Ok(());
        }

        // Phase 2 (async): fetch docs over network — no lock held
        let fetched = crate::indexer::docs::fetch_docs(&pending_docs).await;

        // Phase 3 (sync): store fetched docs — re-acquire lock
        if !fetched.is_empty() {
            let db = self.lock_db()?;
            crate::indexer::docs::store_fetched_docs(&db, &fetched).map_err(to_mcp_err)?;
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
        self.refresh().await?;
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
        self.refresh().await?;
        let db = self.lock_db()?;
        let result =
            tools::context::handle_context(&db, &params.symbol_name).map_err(to_mcp_err)?;
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
        self.refresh().await?;
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
        self.refresh().await?;
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
        self.refresh().await?;
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
        self.refresh().await?;
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
