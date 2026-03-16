pub mod tools;

use crate::db::Database;
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, Content, ServerCapabilities, ServerInfo,
};
use rmcp::schemars;
use schemars::JsonSchema;
use rmcp::{
    ErrorData as McpError, ServerHandler, tool, tool_handler,
    tool_router,
};
use serde::Deserialize;
use std::sync::Mutex;

#[derive(Clone)]
pub struct IlluServer {
    db: std::sync::Arc<Mutex<Database>>,
    tool_router: ToolRouter<Self>,
}

impl IlluServer {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self {
            db: std::sync::Arc::new(Mutex::new(db)),
            tool_router: Self::tool_router(),
        }
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
        let db = self.db.lock().map_err(|e| {
            McpError::internal_error(e.to_string(), None)
        })?;
        let result = tools::query::handle_query(
            &db,
            &params.query,
            params.scope.as_deref(),
        )
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
        let db = self.db.lock().map_err(|e| {
            McpError::internal_error(e.to_string(), None)
        })?;
        let result = tools::context::handle_context(
            &db,
            &params.symbol_name,
        )
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
        let db = self.db.lock().map_err(|e| {
            McpError::internal_error(e.to_string(), None)
        })?;
        let result = tools::impact::handle_impact(
            &db,
            &params.symbol_name,
        )
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
        let db = self.db.lock().map_err(|e| {
            McpError::internal_error(e.to_string(), None)
        })?;
        let result = tools::docs::handle_docs(
            &db,
            &params.dependency,
            params.topic.as_deref(),
        )
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
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .build(),
            ..Default::default()
        }
    }
}
