use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use async_lsp::concurrency::ConcurrencyLayer;
use async_lsp::panic::CatchUnwindLayer;
use async_lsp::tracing::TracingLayer;
use async_lsp::{LanguageServer, MainLoop, ServerSocket};
use lsp_types::{
    ClientCapabilities, CodeActionCapabilityResolveSupport, CodeActionClientCapabilities,
    CodeActionKindLiteralSupport, CodeActionLiteralSupport, CompletionClientCapabilities,
    CompletionItemCapability, DocumentSymbolClientCapabilities,
    DynamicRegistrationClientCapabilities, GeneralClientCapabilities, GotoCapability,
    HoverClientCapabilities, InitializeParams, InitializedParams, MarkupKind, PositionEncodingKind,
    PublishDiagnosticsClientCapabilities, RenameClientCapabilities, TextDocumentClientCapabilities,
    TypeHierarchyClientCapabilities, Url, WindowClientCapabilities, WorkspaceFolder,
};
use tower::ServiceBuilder;
use tracing::{debug, info};

use super::document::DocumentTracker;
use super::error::{RaError, Result};
use super::transport::{ServerState, Stop, build_router};

/// The main rust-analyzer client.
pub struct RaClient {
    server: ServerSocket,
    server_state: ServerState,
    documents: DocumentTracker,
    root_path: PathBuf,
    mainloop_handle: tokio::task::JoinHandle<()>,
}

impl RaClient {
    /// Start rust-analyzer for the given workspace root.
    #[expect(
        clippy::too_many_lines,
        reason = "LSP initialization requires many capability fields"
    )]
    pub async fn start(root_path: &Path) -> Result<Self> {
        let root_path = if root_path.is_absolute() {
            root_path.to_path_buf()
        } else {
            std::env::current_dir()?.join(root_path)
        };

        info!("starting rust-analyzer for {}", root_path.display());

        // Pre-flight: verify rust-analyzer is actually functional.
        // Rustup installs a proxy binary that exists in PATH even when the
        // rust-analyzer component is missing, so `spawn()` alone succeeds but
        // the process exits immediately, causing opaque LSP handshake failures.
        let preflight = async_process::Command::new("rust-analyzer")
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        match preflight {
            Ok(child) => {
                let output = child.output().await.map_err(|e| {
                    RaError::InitializationFailed(format!(
                        "rust-analyzer --version failed: {e}"
                    ))
                })?;
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(RaError::InitializationFailed(format!(
                        "rust-analyzer is not functional (exit {}): {}",
                        output.status,
                        stderr.trim()
                    )));
                }
            }
            Err(e) => {
                return Err(RaError::InitializationFailed(format!(
                    "failed to run rust-analyzer: {e}. Is it installed and in PATH?"
                )));
            }
        }

        let server_state = ServerState::new();
        let documents = DocumentTracker::new();
        let router = build_router(server_state.clone());

        let (mainloop, mut server) = MainLoop::new_client(|_server| {
            ServiceBuilder::new()
                .layer(TracingLayer::default())
                .layer(CatchUnwindLayer::default())
                .layer(ConcurrencyLayer::default())
                .service(router)
        });

        let child = async_process::Command::new("rust-analyzer")
            .current_dir(&root_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                RaError::InitializationFailed(format!(
                    "failed to spawn rust-analyzer: {e}. Is it installed and in PATH?"
                ))
            })?;

        let mut child = child;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| RaError::InitializationFailed("no stdout from RA".into()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| RaError::InitializationFailed("no stdin from RA".into()))?;

        let mainloop_handle = tokio::spawn(async move {
            let _child = child;
            if let Err(e) = mainloop.run_buffered(stdout, stdin).await {
                debug!("mainloop exited with error: {e}");
            }
        });

        let root_uri = Url::from_file_path(&root_path).map_err(|()| {
            RaError::InitializationFailed(format!("invalid root path: {}", root_path.display()))
        })?;

        #[expect(
            deprecated,
            reason = "LSP InitializeParams requires deprecated root_uri field"
        )]
        let init_params = InitializeParams {
            process_id: Some(std::process::id()),
            root_uri: Some(root_uri.clone()),
            workspace_folders: Some(vec![WorkspaceFolder {
                uri: root_uri,
                name: root_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
            }]),
            capabilities: ClientCapabilities {
                text_document: Some(TextDocumentClientCapabilities {
                    hover: Some(HoverClientCapabilities {
                        content_format: Some(vec![MarkupKind::Markdown, MarkupKind::PlainText]),
                        ..Default::default()
                    }),
                    definition: Some(GotoCapability {
                        link_support: Some(false),
                        ..Default::default()
                    }),
                    references: Some(DynamicRegistrationClientCapabilities {
                        dynamic_registration: Some(false),
                    }),
                    document_symbol: Some(DocumentSymbolClientCapabilities {
                        hierarchical_document_symbol_support: Some(true),
                        ..Default::default()
                    }),
                    rename: Some(RenameClientCapabilities {
                        prepare_support: Some(true),
                        ..Default::default()
                    }),
                    publish_diagnostics: Some(PublishDiagnosticsClientCapabilities {
                        related_information: Some(true),
                        ..Default::default()
                    }),
                    completion: Some(CompletionClientCapabilities {
                        completion_item: Some(CompletionItemCapability {
                            documentation_format: Some(vec![
                                MarkupKind::Markdown,
                                MarkupKind::PlainText,
                            ]),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    code_action: Some(CodeActionClientCapabilities {
                        code_action_literal_support: Some(CodeActionLiteralSupport {
                            code_action_kind: CodeActionKindLiteralSupport {
                                value_set: vec![
                                    "quickfix".to_string(),
                                    "refactor".to_string(),
                                    "refactor.extract".to_string(),
                                    "refactor.inline".to_string(),
                                    "refactor.rewrite".to_string(),
                                ],
                            },
                        }),
                        resolve_support: Some(CodeActionCapabilityResolveSupport {
                            properties: vec!["edit".to_string()],
                        }),
                        ..Default::default()
                    }),
                    call_hierarchy: Some(DynamicRegistrationClientCapabilities {
                        dynamic_registration: Some(false),
                    }),
                    type_hierarchy: Some(TypeHierarchyClientCapabilities {
                        dynamic_registration: Some(false),
                    }),
                    ..Default::default()
                }),
                window: Some(WindowClientCapabilities {
                    work_done_progress: Some(true),
                    ..Default::default()
                }),
                general: Some(GeneralClientCapabilities {
                    position_encodings: Some(vec![PositionEncodingKind::UTF16]),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };

        let result = server
            .initialize(init_params)
            .await
            .map_err(|e| RaError::InitializationFailed(format!("initialize failed: {e}")))?;

        info!(
            "server initialized: {}",
            result
                .server_info
                .as_ref()
                .map(|i| format!("{} {}", i.name, i.version.as_deref().unwrap_or("")))
                .unwrap_or_default()
        );

        let _ = server.initialized(InitializedParams {});

        Ok(Self {
            server,
            server_state,
            documents,
            root_path,
            mainloop_handle,
        })
    }

    /// Wait for rust-analyzer to finish indexing. Polls with timeout.
    pub async fn wait_for_ready(&self, timeout: Duration) -> Result<()> {
        let start = std::time::Instant::now();
        let poll_interval = Duration::from_millis(200);

        tokio::time::sleep(Duration::from_secs(1)).await;

        loop {
            if self.server_state.is_ready() {
                info!("rust-analyzer is ready (token match)");
                return Ok(());
            }

            if self.server_state.received_progress()
                && self.server_state.active_progress() == 0
                && start.elapsed() > Duration::from_secs(2)
            {
                info!("rust-analyzer is ready (all progress completed)");
                self.server_state.set_ready(true);
                return Ok(());
            }

            if start.elapsed() > timeout {
                return Err(RaError::Timeout(timeout));
            }

            tokio::time::sleep(poll_interval).await;
        }
    }

    /// Get a reference to the server socket for sending requests.
    #[must_use]
    pub fn server(&self) -> &ServerSocket {
        &self.server
    }

    /// Get the server state.
    #[must_use]
    pub fn server_state(&self) -> &ServerState {
        &self.server_state
    }

    /// Get the workspace root path.
    #[must_use]
    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    /// Ensure a file is open and return its URL.
    pub async fn ensure_open(&self, path: &Path) -> Result<Url> {
        self.documents.ensure_open(&self.server, path).await
    }

    /// Check whether rust-analyzer has finished indexing.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.server_state.is_ready()
    }

    /// Shutdown the server gracefully.
    pub async fn shutdown(mut self) -> Result<()> {
        info!("shutting down rust-analyzer");
        let _ = self.server.shutdown(()).await;
        let _ = self.server.exit(());
        let _ = self.server.emit(Stop);
        let _ = self.mainloop_handle.await;
        Ok(())
    }
}
