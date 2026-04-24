use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_lsp::LanguageServer;
use async_lsp::ServerSocket;
use lsp_types::{
    DidChangeTextDocumentParams, DidOpenTextDocumentParams, TextDocumentContentChangeEvent,
    TextDocumentItem, Url, VersionedTextDocumentIdentifier,
};
use tracing::debug;

use super::error::{RaError, Result};

/// Tracks which documents are currently open in the LSP session.
#[derive(Debug, Clone)]
pub(crate) struct DocumentTracker {
    inner: Arc<Mutex<TrackerInner>>,
}

#[derive(Debug, Default)]
struct TrackerInner {
    open_docs: HashMap<String, i32>,
}

impl Default for DocumentTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentTracker {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(TrackerInner::default())),
        }
    }

    /// Ensure a file is open in the LSP server. Returns the file URL.
    pub(crate) async fn ensure_open(&self, socket: &ServerSocket, path: &Path) -> Result<Url> {
        let abs_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };

        if !abs_path.exists() {
            return Err(RaError::FileNotFound(abs_path.display().to_string()));
        }

        let uri = Url::from_file_path(&abs_path).map_err(|()| {
            RaError::InvalidPosition(format!("cannot convert to URL: {}", abs_path.display()))
        })?;

        let content = tokio::fs::read_to_string(&abs_path).await.map_err(|e| {
            RaError::Io(std::io::Error::new(
                e.kind(),
                format!("{}: {e}", abs_path.display()),
            ))
        })?;

        let uri_str = uri.to_string();
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut sock = socket.clone();

        if let Some(version) = inner.open_docs.get_mut(&uri_str) {
            *version += 1;
            let ver = *version;
            drop(inner);

            debug!("re-syncing document: {uri_str} (version {ver})");
            let _ = sock.did_change(DidChangeTextDocumentParams {
                text_document: VersionedTextDocumentIdentifier {
                    uri: uri.clone(),
                    version: ver,
                },
                content_changes: vec![TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: content,
                }],
            });
        } else {
            inner.open_docs.insert(uri_str.clone(), 1);
            drop(inner);

            debug!("opening document: {uri_str}");
            let _ = sock.did_open(DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: uri.clone(),
                    language_id: "rust".to_string(),
                    version: 1,
                    text: content,
                },
            });
        }

        Ok(uri)
    }
}
