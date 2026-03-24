//! Optional rust-analyzer integration for compiler-accurate code intelligence.
//!
//! This module provides an LSP client that connects to rust-analyzer,
//! exposing semantic operations (go-to-definition, hover, rename, etc.)
//! that are powered by the full Rust compiler.

pub mod client;
pub mod document;
pub mod error;
pub mod extensions;
pub mod lsp;
pub mod ops;
pub mod retry;
pub mod transport;
pub mod types;

pub use client::RaClient;
pub use error::RaError;
pub use types::PositionSpec;
