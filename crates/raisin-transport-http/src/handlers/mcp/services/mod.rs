// SPDX-License-Identifier: BSL-1.1

//! Concrete implementations of the `raisin-mcp` service traits.
//!
//! The MCP engine is transport-agnostic and depends on three narrow service
//! traits ([`FunctionInvoker`](raisin_mcp::FunctionInvoker),
//! [`SearchProvider`](raisin_mcp::SearchProvider),
//! [`EventSource`](raisin_mcp::EventSource)). This module backs each of them with
//! the real RaisinDB services held in [`AppState`](crate::state::AppState):
//!
//! - [`HttpFunctionInvoker`] resolves and runs a `raisin:Function` through the
//!   same resolve → load → [`FunctionExecutor::execute`] pipeline the functions
//!   HTTP handler uses, scoped to the calling MCP identity.
//! - [`HttpSearchProvider`] backs full-text search with the Tantivy engine and
//!   semantic search with the HNSW engine, offloading the synchronous index work
//!   to a blocking pool.
//! - [`BusEventSource`] bridges the in-process event bus onto the engine's
//!   [`NodeChange`](raisin_mcp::NodeChange) stream for `resources/subscribe`.

#[cfg(feature = "storage-rocksdb")]
mod assets;
#[cfg(feature = "storage-rocksdb")]
mod events;
#[cfg(feature = "storage-rocksdb")]
mod invoker;
#[cfg(feature = "storage-rocksdb")]
mod search;

#[cfg(feature = "storage-rocksdb")]
pub(super) use assets::HttpAssetReader;
#[cfg(feature = "storage-rocksdb")]
pub(super) use events::BusEventSource;
#[cfg(feature = "storage-rocksdb")]
pub(super) use invoker::HttpFunctionInvoker;
#[cfg(feature = "storage-rocksdb")]
pub(super) use search::HttpSearchProvider;
