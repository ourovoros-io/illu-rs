use std::collections::HashMap;
use std::ops::ControlFlow;
use std::sync::{Arc, Mutex};

use async_lsp::router::Router;
use lsp_types::notification::{Progress, PublishDiagnostics, ShowMessage};
use lsp_types::request::{RegisterCapability, WorkDoneProgressCreate};
use lsp_types::{DiagnosticSeverity, NumberOrString, ProgressParamsValue, WorkDoneProgress};
use tokio::sync::Notify;
use tracing::{debug, info, warn};

use super::types::DiagnosticInfo;

/// Event to signal the mainloop to stop.
pub(crate) struct Stop;

/// Token names for rust-analyzer indexing progress.
const RA_INDEXING_TOKENS: &[&str] = &["rustAnalyzer/Indexing", "rustAnalyzer/cachePriming"];

/// Shared state updated by LSP notifications.
#[derive(Debug, Clone)]
pub struct ServerState {
    inner: Arc<Mutex<ServerStateInner>>,
    /// Signalled whenever a state change could make `wait_for_ready`
    /// resolve — specifically `set_ready(true)` and `end_progress`.
    /// Lets callers await readiness instead of polling.
    readiness: Arc<Notify>,
}

#[derive(Debug, Default)]
struct ServerStateInner {
    ready: bool,
    received_progress: bool,
    active_progress: usize,
    diagnostics: HashMap<String, Vec<DiagnosticInfo>>,
}

impl Default for ServerState {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ServerStateInner::default())),
            readiness: Arc::new(Notify::new()),
        }
    }

    /// Return the readiness notifier so callers can register their
    /// interest with `notify.notified()` *before* checking state. Using
    /// `readiness_changed().await` directly is race-prone: a notification
    /// fired between the state check and the await would be missed
    /// (`notify_waiters` wakes only currently-registered waiters, it does
    /// not leave a permit).
    #[must_use]
    pub fn readiness_notifier(&self) -> Arc<Notify> {
        Arc::clone(&self.readiness)
    }

    /// Acquire the inner mutex, transparently recovering from poisoning.
    /// Centralises the `unwrap_or_else(PoisonError::into_inner)` idiom.
    fn lock(&self) -> std::sync::MutexGuard<'_, ServerStateInner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.lock().ready
    }

    pub fn set_ready(&self, ready: bool) {
        self.lock().ready = ready;
        if ready {
            self.readiness.notify_waiters();
        }
    }

    #[must_use]
    pub fn received_progress(&self) -> bool {
        self.lock().received_progress
    }

    #[must_use]
    pub fn active_progress(&self) -> usize {
        self.lock().active_progress
    }

    pub fn begin_progress(&self) {
        let mut inner = self.lock();
        inner.received_progress = true;
        inner.active_progress += 1;
    }

    pub fn end_progress(&self) {
        let went_idle = {
            let mut inner = self.lock();
            inner.active_progress = inner.active_progress.saturating_sub(1);
            inner.received_progress && inner.active_progress == 0
        };
        if went_idle {
            self.readiness.notify_waiters();
        }
    }

    pub fn set_diagnostics(&self, uri: String, diags: Vec<DiagnosticInfo>) {
        self.lock().diagnostics.insert(uri, diags);
    }

    #[must_use]
    pub fn get_diagnostics(&self, uri: &str) -> Vec<DiagnosticInfo> {
        self.lock()
            .diagnostics
            .get(uri)
            .cloned()
            .unwrap_or_default()
    }

    #[must_use]
    pub fn all_diagnostics(&self) -> HashMap<String, Vec<DiagnosticInfo>> {
        self.lock().diagnostics.clone()
    }
}

/// Build the notification router for the LSP client.
#[must_use]
pub(crate) fn build_router(state: ServerState) -> Router<()> {
    let progress_state = state.clone();
    let diag_state = state;

    let mut router = Router::new(());

    router
        .request::<WorkDoneProgressCreate, _>(|(), _params| std::future::ready(Ok(())))
        .request::<RegisterCapability, _>(|(), _params| std::future::ready(Ok(())))
        .event(|(), _: Stop| ControlFlow::Break(Ok(())));

    router
        .notification::<Progress>(move |(), prog| {
            let token = match &prog.token {
                NumberOrString::String(s) => s.clone(),
                NumberOrString::Number(n) => n.to_string(),
            };

            match prog.value {
                ProgressParamsValue::WorkDone(progress) => match progress {
                    WorkDoneProgress::Begin(begin) => {
                        debug!("progress begin [{token}]: {}", begin.title);
                        progress_state.begin_progress();
                    }
                    WorkDoneProgress::Report(report) => {
                        if let Some(msg) = &report.message {
                            debug!("progress report [{token}]: {msg}");
                        }
                    }
                    WorkDoneProgress::End(_) => {
                        debug!("progress end [{token}]");
                        progress_state.end_progress();
                        if RA_INDEXING_TOKENS.iter().any(|t| token.contains(t)) {
                            info!("rust-analyzer indexing complete (token match)");
                            progress_state.set_ready(true);
                        }
                    }
                },
            }
            ControlFlow::Continue(())
        })
        .notification::<PublishDiagnostics>(move |(), params| {
            let uri = params.uri.to_string();
            let diags: Vec<DiagnosticInfo> = params
                .diagnostics
                .iter()
                .map(|d| DiagnosticInfo {
                    file: uri.clone(),
                    line: d.range.start.line + 1,
                    severity: match d.severity {
                        Some(DiagnosticSeverity::ERROR) => "error".to_string(),
                        Some(DiagnosticSeverity::WARNING) => "warning".to_string(),
                        Some(DiagnosticSeverity::INFORMATION) => "info".to_string(),
                        Some(DiagnosticSeverity::HINT) => "hint".to_string(),
                        _ => "unknown".to_string(),
                    },
                    message: d.message.clone(),
                })
                .collect();
            diag_state.set_diagnostics(uri, diags);
            ControlFlow::Continue(())
        })
        .notification::<ShowMessage>(|(), params| {
            warn!("rust-analyzer message: {}", params.message);
            ControlFlow::Continue(())
        });

    router
}
