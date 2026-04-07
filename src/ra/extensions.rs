//! rust-analyzer extension requests (non-standard LSP).

use lsp_types::TextDocumentIdentifier;
use serde::{Deserialize, Serialize};

use super::client::RaClient;
use super::error::{RaError, Result};
use super::types::PositionSpec;

// ── Expand Macro ─────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ExpandMacro {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpandMacroParams {
    pub text_document: TextDocumentIdentifier,
    pub position: lsp_types::Position,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpandedMacro {
    pub name: String,
    pub expansion: String,
}

impl lsp_types::request::Request for ExpandMacro {
    type Params = ExpandMacroParams;
    type Result = Option<ExpandedMacro>;
    const METHOD: &'static str = "rust-analyzer/expandMacro";
}

impl RaClient {
    pub async fn expand_macro(&self, spec: &PositionSpec) -> Result<Option<ExpandedMacro>> {
        let uri = self.ensure_open(&spec.file).await?;
        let pos = spec.to_lsp_position();
        let server = self.server().clone();

        let params = ExpandMacroParams {
            text_document: TextDocumentIdentifier { uri },
            position: pos,
        };

        server
            .request::<ExpandMacro>(params)
            .await
            .map_err(|e| RaError::RequestFailed(format!("expandMacro: {e}")))
    }
}

// ── SSR (Structural Search and Replace) ──────────────────────────────────

#[derive(Debug)]
pub enum Ssr {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SsrParams {
    pub query: String,
    pub parse_only: bool,
    #[serde(flatten)]
    pub position: lsp_types::TextDocumentPositionParams,
    pub selections: Vec<lsp_types::Range>,
}

impl lsp_types::request::Request for Ssr {
    type Params = SsrParams;
    type Result = lsp_types::WorkspaceEdit;
    const METHOD: &'static str = "experimental/ssr";
}

impl RaClient {
    pub async fn ssr(&self, query: &str) -> Result<lsp_types::WorkspaceEdit> {
        let server = self.server().clone();

        // Find a valid file in the workspace to serve as the context position
        let fallback_uri = lsp_types::Url::from_file_path(self.root_path().join("Cargo.toml"))
            .map_err(|()| RaError::RequestFailed("Invalid workspace root path".to_string()))?;
        // Since ssr requires a valid rust file, we try to use Cargo.toml or just root
        let uri = fallback_uri;

        let params = SsrParams {
            query: query.to_string(),
            parse_only: false,
            position: lsp_types::TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: lsp_types::Position::default(),
            },
            selections: vec![],
        };

        server
            .request::<Ssr>(params)
            .await
            .map_err(|e| RaError::RequestFailed(format!("ssr: {e}")))
    }
}

// ── Related Tests ────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum RelatedTests {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedTestsParams {
    pub text_document: TextDocumentIdentifier,
    pub position: lsp_types::Position,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestInfo {
    pub runnable: Runnable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Runnable {
    pub label: String,
    pub location: Option<lsp_types::LocationLink>,
    pub kind: String,
    pub args: RunnableArgs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnableArgs {
    pub workspace_root: Option<String>,
    pub cargo_args: Vec<String>,
    pub executable_args: Vec<String>,
}

impl lsp_types::request::Request for RelatedTests {
    type Params = RelatedTestsParams;
    type Result = Vec<TestInfo>;
    const METHOD: &'static str = "rust-analyzer/relatedTests";
}

impl RaClient {
    pub async fn related_tests(&self, spec: &PositionSpec) -> Result<Vec<TestInfo>> {
        let uri = self.ensure_open(&spec.file).await?;
        let pos = spec.to_lsp_position();
        let server = self.server().clone();

        let params = RelatedTestsParams {
            text_document: TextDocumentIdentifier { uri },
            position: pos,
        };

        server
            .request::<RelatedTests>(params)
            .await
            .map_err(|e| RaError::RequestFailed(format!("relatedTests: {e}")))
    }
}

// ── Syntax Tree ──────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum SyntaxTree {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyntaxTreeParams {
    pub text_document: TextDocumentIdentifier,
}

impl lsp_types::request::Request for SyntaxTree {
    type Params = SyntaxTreeParams;
    type Result = String;
    const METHOD: &'static str = "rust-analyzer/viewSyntaxTree";
}

impl RaClient {
    pub async fn syntax_tree(&self, path: &std::path::Path) -> Result<String> {
        let uri = self.ensure_open(path).await?;
        let server = self.server().clone();

        let params = SyntaxTreeParams {
            text_document: TextDocumentIdentifier { uri },
        };

        server
            .request::<SyntaxTree>(params)
            .await
            .map_err(|e| RaError::RequestFailed(format!("syntaxTree: {e}")))
    }
}
