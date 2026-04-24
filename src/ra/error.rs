use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RaError {
    #[error("server not ready: {0}")]
    ServerNotReady(String),

    #[error("server initialization failed: {0}")]
    InitializationFailed(String),

    #[error("request timed out after {0:?}")]
    Timeout(std::time::Duration),

    #[error("LSP request failed: {0}")]
    RequestFailed(String),

    #[error("LSP method not supported by this rust-analyzer: {0}")]
    MethodNotSupported(String),

    #[error("content modified (retryable)")]
    ContentModified,

    #[error("file not found: {0}")]
    FileNotFound(String),

    #[error("invalid position: {0}")]
    InvalidPosition(String),

    #[error("symbol not found: {0}")]
    SymbolNotFound(String),

    #[error("rust-analyzer not available")]
    NotAvailable,

    #[error("server shut down unexpectedly")]
    ServerShutdown,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub(crate) type Result<T> = std::result::Result<T, RaError>;
