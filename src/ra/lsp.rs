//! Typed wrappers around LSP protocol methods.

use std::collections::HashMap;
use std::path::Path;

use async_lsp::LanguageServer;
use lsp_types::request::GotoImplementationParams;
use lsp_types::{
    CallHierarchyIncomingCallsParams, CallHierarchyItem, CallHierarchyOutgoingCallsParams,
    CallHierarchyPrepareParams, CodeAction, CodeActionContext, CodeActionKind, CodeActionOrCommand,
    CodeActionParams, CodeActionTriggerKind, DocumentSymbol, DocumentSymbolParams,
    DocumentSymbolResponse, GotoDefinitionParams, GotoDefinitionResponse, HoverContents,
    HoverParams, Location, MarkedString, OneOf, PartialResultParams, PrepareRenameResponse, Range,
    ReferenceContext, ReferenceParams, TextDocumentIdentifier, TextDocumentPositionParams,
    TypeHierarchyItem, TypeHierarchyPrepareParams, TypeHierarchySubtypesParams,
    TypeHierarchySupertypesParams, Url, WorkDoneProgressParams, WorkspaceEdit,
    WorkspaceSymbolParams, WorkspaceSymbolResponse,
};

use super::client::RaClient;
use super::error::{RaError, Result};
use super::retry::with_retry;
use super::types::{
    CallInfo, CallSite, DiagnosticInfo, LspSymbolInfo, PositionSpec, symbol_kind_name,
    url_to_path_string,
};

// ── Navigation ──────────────────────────────────────────────────────────

impl RaClient {
    /// Go to definition at the given position.
    pub async fn definition(&self, spec: &PositionSpec) -> Result<Vec<Location>> {
        let uri = self.ensure_open(&spec.file).await?;
        let pos = spec.to_lsp_position();
        let server = self.server().clone();

        let params = GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: pos,
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        };

        let response = with_retry(|| {
            let mut s = server.clone();
            let p = params.clone();
            async move { s.definition(p).await }
        })
        .await?;

        Ok(goto_response_to_locations(response))
    }

    /// Go to implementation at the given position.
    pub async fn implementation(&self, spec: &PositionSpec) -> Result<Vec<Location>> {
        let uri = self.ensure_open(&spec.file).await?;
        let pos = spec.to_lsp_position();
        let server = self.server().clone();

        let params: GotoImplementationParams = GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: pos,
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        };

        let response = with_retry(|| {
            let mut s = server.clone();
            let p = params.clone();
            async move { s.implementation(p).await }
        })
        .await?;

        Ok(goto_response_to_locations(response))
    }

    /// Find all references at the given position.
    pub async fn references(
        &self,
        spec: &PositionSpec,
        include_declaration: bool,
    ) -> Result<Vec<Location>> {
        let uri = self.ensure_open(&spec.file).await?;
        let pos = spec.to_lsp_position();
        let server = self.server().clone();

        let params = ReferenceParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: pos,
            },
            context: ReferenceContext {
                include_declaration,
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        };

        let response = with_retry(|| {
            let mut s = server.clone();
            let p = params.clone();
            async move { s.references(p).await }
        })
        .await?;

        Ok(response.unwrap_or_default())
    }
}

fn goto_response_to_locations(response: Option<GotoDefinitionResponse>) -> Vec<Location> {
    match response {
        Some(GotoDefinitionResponse::Scalar(loc)) => vec![loc],
        Some(GotoDefinitionResponse::Array(locs)) => locs,
        Some(GotoDefinitionResponse::Link(links)) => links
            .into_iter()
            .map(|link| Location {
                uri: link.target_uri,
                range: link.target_selection_range,
            })
            .collect(),
        None => vec![],
    }
}

// ── Hover ───────────────────────────────────────────────────────────────

impl RaClient {
    pub async fn hover(&self, spec: &PositionSpec) -> Result<Option<String>> {
        let uri = self.ensure_open(&spec.file).await?;
        let pos = spec.to_lsp_position();
        let server = self.server().clone();

        let params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: pos,
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
        };

        let response = with_retry(|| {
            let mut s = server.clone();
            let p = params.clone();
            async move { s.hover(p).await }
        })
        .await?;

        Ok(response.map(|hover| extract_hover_text(&hover.contents)))
    }
}

fn extract_hover_text(contents: &HoverContents) -> String {
    match contents {
        HoverContents::Scalar(value) => extract_marked_string(value),
        HoverContents::Array(values) => values
            .iter()
            .map(extract_marked_string)
            .collect::<Vec<_>>()
            .join("\n\n"),
        HoverContents::Markup(markup) => markup.value.clone(),
    }
}

fn extract_marked_string(ms: &MarkedString) -> String {
    match ms {
        MarkedString::String(s) => s.clone(),
        MarkedString::LanguageString(ls) => format!("```{}\n{}\n```", ls.language, ls.value),
    }
}

// ── Symbols ─────────────────────────────────────────────────────────────

impl RaClient {
    pub async fn document_symbols(&self, path: &Path) -> Result<Vec<LspSymbolInfo>> {
        let uri = self.ensure_open(path).await?;
        let server = self.server().clone();

        let params = DocumentSymbolParams {
            text_document: TextDocumentIdentifier { uri },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        };

        let response = with_retry(|| {
            let mut s = server.clone();
            let p = params.clone();
            async move { s.document_symbol(p).await }
        })
        .await?;

        Ok(match response {
            Some(DocumentSymbolResponse::Nested(symbols)) => {
                symbols.into_iter().map(convert_document_symbol).collect()
            }
            Some(DocumentSymbolResponse::Flat(symbols)) => symbols
                .into_iter()
                .map(|s| LspSymbolInfo {
                    name: s.name,
                    kind: symbol_kind_name(s.kind).to_string(),
                    detail: None,
                    file: url_to_path_string(&s.location.uri),
                    line: s.location.range.start.line + 1,
                    col: s.location.range.start.character + 1,
                    children: vec![],
                })
                .collect(),
            None => vec![],
        })
    }

    pub async fn workspace_symbols(&self, query: &str) -> Result<Vec<LspSymbolInfo>> {
        let server = self.server().clone();

        let params = WorkspaceSymbolParams {
            query: query.to_string(),
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        };

        let response = with_retry(|| {
            let mut s = server.clone();
            let p = params.clone();
            async move { s.symbol(p).await }
        })
        .await?;

        Ok(match response {
            Some(WorkspaceSymbolResponse::Flat(symbols)) => symbols
                .into_iter()
                .map(|s| LspSymbolInfo {
                    name: s.name,
                    kind: symbol_kind_name(s.kind).to_string(),
                    detail: s.container_name,
                    file: url_to_path_string(&s.location.uri),
                    line: s.location.range.start.line + 1,
                    col: s.location.range.start.character + 1,
                    children: vec![],
                })
                .collect(),
            Some(WorkspaceSymbolResponse::Nested(symbols)) => symbols
                .into_iter()
                .map(|s| {
                    let loc = match s.location {
                        OneOf::Left(loc) => loc,
                        OneOf::Right(wloc) => Location {
                            uri: wloc.uri,
                            range: Range::default(),
                        },
                    };
                    LspSymbolInfo {
                        name: s.name,
                        kind: symbol_kind_name(s.kind).to_string(),
                        detail: s.container_name,
                        file: url_to_path_string(&loc.uri),
                        line: loc.range.start.line + 1,
                        col: loc.range.start.character + 1,
                        children: vec![],
                    }
                })
                .collect(),
            None => vec![],
        })
    }
}

fn convert_document_symbol(sym: DocumentSymbol) -> LspSymbolInfo {
    LspSymbolInfo {
        name: sym.name,
        kind: symbol_kind_name(sym.kind).to_string(),
        detail: sym.detail,
        file: String::new(),
        line: sym.selection_range.start.line + 1,
        col: sym.selection_range.start.character + 1,
        children: sym
            .children
            .unwrap_or_default()
            .into_iter()
            .map(convert_document_symbol)
            .collect(),
    }
}

// ── Call Hierarchy ───────────────────────────────────────────────────────

impl RaClient {
    pub async fn prepare_call_hierarchy(
        &self,
        spec: &PositionSpec,
    ) -> Result<Vec<CallHierarchyItem>> {
        let uri = self.ensure_open(&spec.file).await?;
        let pos = spec.to_lsp_position();
        let server = self.server().clone();

        let params = CallHierarchyPrepareParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: pos,
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
        };

        let response = with_retry(|| {
            let mut s = server.clone();
            let p = params.clone();
            async move { s.prepare_call_hierarchy(p).await }
        })
        .await?;

        Ok(response.unwrap_or_default())
    }

    pub async fn incoming_calls(&self, item: CallHierarchyItem) -> Result<Vec<CallInfo>> {
        let server = self.server().clone();

        let params = CallHierarchyIncomingCallsParams {
            item,
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        };

        let response = with_retry(|| {
            let mut s = server.clone();
            let p = params.clone();
            async move { s.incoming_calls(p).await }
        })
        .await?;

        Ok(response
            .unwrap_or_default()
            .into_iter()
            .map(|call| CallInfo {
                name: call.from.name,
                kind: symbol_kind_name(call.from.kind).to_string(),
                file: url_to_path_string(&call.from.uri),
                line: call.from.selection_range.start.line + 1,
                col: call.from.selection_range.start.character + 1,
                detail: call.from.detail,
                call_sites: call
                    .from_ranges
                    .into_iter()
                    .map(|r| CallSite {
                        file: String::new(),
                        line: r.start.line + 1,
                        col: r.start.character + 1,
                    })
                    .collect(),
            })
            .collect())
    }

    pub async fn outgoing_calls(&self, item: CallHierarchyItem) -> Result<Vec<CallInfo>> {
        let server = self.server().clone();

        let params = CallHierarchyOutgoingCallsParams {
            item,
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        };

        let response = with_retry(|| {
            let mut s = server.clone();
            let p = params.clone();
            async move { s.outgoing_calls(p).await }
        })
        .await?;

        Ok(response
            .unwrap_or_default()
            .into_iter()
            .map(|call| CallInfo {
                name: call.to.name,
                kind: symbol_kind_name(call.to.kind).to_string(),
                file: url_to_path_string(&call.to.uri),
                line: call.to.selection_range.start.line + 1,
                col: call.to.selection_range.start.character + 1,
                detail: call.to.detail,
                call_sites: call
                    .from_ranges
                    .into_iter()
                    .map(|r| CallSite {
                        file: String::new(),
                        line: r.start.line + 1,
                        col: r.start.character + 1,
                    })
                    .collect(),
            })
            .collect())
    }
}

// ── Type Hierarchy ──────────────────────────────────────────────────────

impl RaClient {
    pub async fn prepare_type_hierarchy(
        &self,
        spec: &PositionSpec,
    ) -> Result<Vec<TypeHierarchyItem>> {
        let uri = self.ensure_open(&spec.file).await?;
        let pos = spec.to_lsp_position();
        let server = self.server().clone();

        let params = TypeHierarchyPrepareParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: pos,
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
        };

        let response = with_retry(|| {
            let mut s = server.clone();
            let p = params.clone();
            async move { s.prepare_type_hierarchy(p).await }
        })
        .await?;

        Ok(response.unwrap_or_default())
    }

    pub async fn supertypes(&self, item: TypeHierarchyItem) -> Result<Vec<LspSymbolInfo>> {
        let server = self.server().clone();

        let params = TypeHierarchySupertypesParams {
            item,
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        };

        let response = with_retry(|| {
            let mut s = server.clone();
            let p = params.clone();
            async move { s.supertypes(p).await }
        })
        .await?;

        Ok(type_hierarchy_items_to_symbols(
            response.unwrap_or_default(),
        ))
    }

    pub async fn subtypes(&self, item: TypeHierarchyItem) -> Result<Vec<LspSymbolInfo>> {
        let server = self.server().clone();

        let params = TypeHierarchySubtypesParams {
            item,
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        };

        let response = with_retry(|| {
            let mut s = server.clone();
            let p = params.clone();
            async move { s.subtypes(p).await }
        })
        .await?;

        Ok(type_hierarchy_items_to_symbols(
            response.unwrap_or_default(),
        ))
    }
}

fn type_hierarchy_items_to_symbols(items: Vec<TypeHierarchyItem>) -> Vec<LspSymbolInfo> {
    items
        .into_iter()
        .map(|item| LspSymbolInfo {
            name: item.name,
            kind: symbol_kind_name(item.kind).to_string(),
            detail: item.detail,
            file: url_to_path_string(&item.uri),
            line: item.selection_range.start.line + 1,
            col: item.selection_range.start.character + 1,
            children: vec![],
        })
        .collect()
}

// ── Rename ──────────────────────────────────────────────────────────────

impl RaClient {
    pub async fn prepare_rename(
        &self,
        spec: &PositionSpec,
    ) -> Result<Option<PrepareRenameResponse>> {
        let uri = self.ensure_open(&spec.file).await?;
        let pos = spec.to_lsp_position();
        let server = self.server().clone();

        let params = TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri },
            position: pos,
        };

        let response = with_retry(|| {
            let mut s = server.clone();
            let p = params.clone();
            async move { s.prepare_rename(p).await }
        })
        .await?;

        Ok(response)
    }

    pub async fn rename(
        &self,
        spec: &PositionSpec,
        new_name: &str,
    ) -> Result<Option<WorkspaceEdit>> {
        let uri = self.ensure_open(&spec.file).await?;
        let pos = spec.to_lsp_position();
        let server = self.server().clone();

        let params = lsp_types::RenameParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: pos,
            },
            new_name: new_name.to_string(),
            work_done_progress_params: WorkDoneProgressParams::default(),
        };

        let response = with_retry(|| {
            let mut s = server.clone();
            let p = params.clone();
            async move { s.rename(p).await }
        })
        .await?;

        Ok(response)
    }
}

// ── Code Actions ────────────────────────────────────────────────────────

impl RaClient {
    pub async fn code_actions(
        &self,
        spec: &PositionSpec,
        kind_filter: Option<&str>,
    ) -> Result<Vec<CodeActionOrCommand>> {
        let uri = self.ensure_open(&spec.file).await?;
        let pos = spec.to_lsp_position();
        let server = self.server().clone();

        let only = kind_filter.map(|k| vec![CodeActionKind::from(k.to_string())]);

        let params = CodeActionParams {
            text_document: TextDocumentIdentifier { uri },
            range: Range {
                start: pos,
                end: pos,
            },
            context: CodeActionContext {
                diagnostics: vec![],
                only,
                trigger_kind: Some(CodeActionTriggerKind::INVOKED),
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        };

        let response = with_retry(|| {
            let mut s = server.clone();
            let p = params.clone();
            async move { s.code_action(p).await }
        })
        .await?;

        Ok(response.unwrap_or_default())
    }

    pub async fn resolve_code_action(&self, action: CodeAction) -> Result<CodeAction> {
        let server = self.server().clone();

        let response = with_retry(|| {
            let mut s = server.clone();
            let a = action.clone();
            async move { s.code_action_resolve(a).await }
        })
        .await?;

        Ok(response)
    }
}

// ── Diagnostics ─────────────────────────────────────────────────────────

impl RaClient {
    /// Get current diagnostics for a specific file.
    pub fn get_diagnostics(&self, path: &Path) -> Result<Vec<DiagnosticInfo>> {
        let abs_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };

        let uri = Url::from_file_path(&abs_path).map_err(|()| {
            RaError::InvalidPosition(format!("cannot convert to URL: {}", abs_path.display()))
        })?;

        Ok(self.server_state().get_diagnostics(uri.as_ref()))
    }

    /// Get all current diagnostics.
    #[must_use]
    pub fn get_all_diagnostics(&self) -> HashMap<String, Vec<DiagnosticInfo>> {
        self.server_state().all_diagnostics()
    }
}
