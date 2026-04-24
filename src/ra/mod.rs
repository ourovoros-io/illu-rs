//! Optional rust-analyzer integration for compiler-accurate code intelligence.
//!
//! This module provides an LSP client that connects to rust-analyzer,
//! exposing semantic operations (go-to-definition, hover, rename, etc.)
//! that are powered by the full Rust compiler.

#![allow(dead_code, missing_docs, unreachable_pub)]

pub(crate) mod client;
pub(crate) mod document;
pub(crate) mod error;
pub(crate) mod extensions;
pub(crate) mod lsp;
pub(crate) mod ops;
pub(crate) mod retry;
pub(crate) mod transport;
pub(crate) mod types;

pub use client::RaClient;
pub use error::RaError;
pub use types::PositionSpec;
