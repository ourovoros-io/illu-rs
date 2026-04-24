use std::future::Future;
use std::time::Duration;

use async_lsp::{Error, ErrorCode};
use tracing::{debug, warn};

use super::error::{RaError, Result};

const MAX_RETRIES: u32 = 5;
const INITIAL_BACKOFF_MS: u64 = 100;

/// Execute an LSP request with retry on `CONTENT_MODIFIED` errors.
/// Uses exponential backoff between retries.
pub(crate) async fn with_retry<F, Fut, T>(operation: F) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: Future<Output = std::result::Result<T, Error>>,
{
    let mut backoff = Duration::from_millis(INITIAL_BACKOFF_MS);

    for attempt in 0..=MAX_RETRIES {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(Error::Response(resp)) if resp.code == ErrorCode::CONTENT_MODIFIED => {
                if attempt < MAX_RETRIES {
                    debug!(
                        "content modified (attempt {}/{}), retrying in {:?}",
                        attempt + 1,
                        MAX_RETRIES,
                        backoff
                    );
                    tokio::time::sleep(backoff).await;
                    backoff *= 2;
                    continue;
                }
                warn!("content modified after {MAX_RETRIES} retries, giving up");
                return Err(RaError::ContentModified);
            }
            Err(Error::Response(resp)) if resp.code == ErrorCode::METHOD_NOT_FOUND => {
                return Err(RaError::MethodNotSupported(resp.message.clone()));
            }
            Err(err) => {
                return Err(RaError::RequestFailed(format!("LSP error: {err}")));
            }
        }
    }

    // Safety: the loop always returns via Ok or Err before exceeding MAX_RETRIES
    Err(RaError::ContentModified)
}
