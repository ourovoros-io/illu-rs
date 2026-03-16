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
use std::sync::Mutex;

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
    pub fn db_handle(&self) -> std::sync::Arc<Mutex<Database>> {
        std::sync::Arc::clone(&self.db)
    }

    async fn refresh(&self) -> Result<(), McpError> {
        // Phase 1 (sync): re-index changed files, collect pending docs
        let pending_docs = {
            let db = self
                .db
                .lock()
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            crate::indexer::refresh_index(&db, &self.config)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            crate::indexer::docs::pending_docs(&db)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?
        }; // lock dropped

        if pending_docs.is_empty() {
            return Ok(());
        }

        // Phase 2 (async): fetch docs over network — no lock held
        let fetched = crate::indexer::docs::fetch_docs(&pending_docs).await;

        // Phase 3 (sync): store fetched docs — re-acquire lock
        if !fetched.is_empty() {
            let db = self
                .db
                .lock()
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            crate::indexer::docs::store_fetched_docs(&db, &fetched)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        }
        Ok(())
    }
}

#[derive(Deserialize, JsonSchema)]
struct QueryParams {
    query: String,
    scope: Option<String>,
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

#[tool_router]
impl IlluServer {
    #[tool(
        name = "query",
        description = "Search the codebase for symbols, documentation, or files. Scope: symbols, docs, files, or all (default)."
    )]
    async fn query(
        &self,
        Parameters(params): Parameters<QueryParams>,
    ) -> Result<CallToolResult, McpError> {
        self.refresh().await?;
        let db = self
            .db
            .lock()
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let result = tools::query::handle_query(&db, &params.query, params.scope.as_deref())
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(result)]))
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
        let db = self
            .db
            .lock()
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let result = tools::context::handle_context(&db, &params.symbol_name)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(result)]))
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
        let db = self
            .db
            .lock()
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let result = tools::impact::handle_impact(&db, &params.symbol_name)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(result)]))
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
        let db = self
            .db
            .lock()
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let result = tools::docs::handle_docs(&db, &params.dependency, params.topic.as_deref())
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(result)]))
    }
}

#[tool_handler]
impl ServerHandler for IlluServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "illu-rs: Code intelligence server for Rust projects. \
                 Use 'query' to search, 'context' for symbol details, \
                 'impact' for change analysis, 'docs' for dependency docs."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}
