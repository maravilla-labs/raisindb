// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Service abstractions the MCP engine depends on.
//!
//! The engine is transport-agnostic: it never reaches into RocksDB, the HTTP
//! [`AppState`](https://docs.rs), or feature-gated index engines directly.
//! Instead it depends on three narrow traits that the hosting transport (the
//! HTTP layer) implements, backing them with the *real* RaisinDB services:
//!
//! - [`FunctionInvoker`] — resolve + execute a `raisin:Function` by name, as the
//!   calling identity, returning the real
//!   [`raisin_functions::ExecutionResult`].
//! - [`SearchProvider`] — full-text and vector search over a workspace, backed
//!   by the Tantivy / HNSW engines.
//! - [`EventSource`] — a live stream of node-change events, bridged from the
//!   in-process event bus, used to back `resources/subscribe`.
//! - [`AssetReader`] — raw asset bytes + mime for a `(workspace, path)`, used to
//!   serve `raisin:Asset` content byte-for-byte over `resources/read` and to
//!   back `mode: html` MCP-UI widget resources.
//!
//! The node read / write / query / SQL data path is *not* abstracted here: it
//! uses [`raisin_functions::FunctionApi`] directly (see [`crate::data_tools`]),
//! which is the same RLS-scoped backend server-side functions run against.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use raisin_functions::ExecutionResult;

use crate::error::Result;
use crate::identity::McpIdentity;

// ---------------------------------------------------------------------------
// Function invocation
// ---------------------------------------------------------------------------

/// Resolves and executes `raisin:Function` nodes on behalf of the MCP engine.
///
/// Implementors own the resolve → load → execute pipeline (node lookup by name,
/// entry-file code load, [`raisin_functions::FunctionExecutor::execute`]) and
/// must run it as the supplied [`McpIdentity`] so the function body is RLS-scoped
/// to the caller. The returned [`ExecutionResult`] is the real executor output:
/// `success: false` with `error: Some(_)` is a function-level failure, whereas an
/// `Err` is an infrastructure failure.
pub trait FunctionInvoker: Send + Sync {
    /// Invoke the function named `name` with `input`, as `identity`.
    fn invoke<'a>(
        &'a self,
        identity: &'a McpIdentity,
        name: &'a str,
        input: Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ExecutionResult>> + Send + 'a>>;
}

/// Shared handle to a [`FunctionInvoker`].
pub type SharedFunctionInvoker = Arc<dyn FunctionInvoker>;

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

/// Mode of a [`SearchProvider`] query.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    /// Full-text (lexical) search.
    #[default]
    Fulltext,
    /// Vector (semantic) similarity search.
    Vector,
}

/// A normalized search query, independent of the underlying engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    /// Workspace to search within.
    pub workspace: String,
    /// Branch to search within.
    pub branch: String,
    /// Free-text query string.
    pub query: String,
    /// Search mode (lexical vs semantic).
    #[serde(default)]
    pub mode: SearchMode,
    /// Optional exact `ns:TypeName` filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_type: Option<String>,
    /// Maximum number of hits to return.
    pub limit: usize,
    /// What one hit is: `node` (default) or `chunk`.
    ///
    /// With `chunk`, `limit` counts PASSAGES and several hits may share a
    /// `node_id` — which is what an agent filling a context window wants. The
    /// default keeps `limit` meaning documents, as every existing caller
    /// expects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granularity: Option<String>,
}

/// A single search hit, normalized across engines.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    /// Node id of the hit.
    pub node_id: String,
    /// Workspace the hit belongs to.
    pub workspace: String,
    /// Relevance score (higher is better); for vector search this is similarity.
    pub score: f32,
    /// Node path, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Node type, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_type: Option<String>,
    /// Which chunk of the document answered, when a vector leg matched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk_index: Option<i64>,
    /// The TEXT of that chunk — the passage an agent should quote or reason
    /// over, rather than re-reading and re-chunking the document itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk_text: Option<String>,
    /// Where `chunk_text` came from: `exact` (the passage), `excerpt` (a
    /// 200-character preview — NOT the whole passage) or `unavailable`. An
    /// agent that treats an excerpt as the full passage will quote a truncated
    /// source, so this is reported rather than left to be inferred.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk_text_source: Option<String>,
    /// Vector distance, when a vector leg matched. Lower is closer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector_distance: Option<f32>,
}

/// Lexical and semantic search over RaisinDB content.
///
/// The HTTP layer backs this with the real `TantivyIndexingEngine` (full-text)
/// and `HnswIndexingEngine` (vector); the engines are sync/CPU-bound, so
/// implementors are expected to offload to a blocking pool internally.
pub trait SearchProvider: Send + Sync {
    /// Run `query` for `identity` and return the ranked hits.
    fn search<'a>(
        &'a self,
        identity: &'a McpIdentity,
        query: SearchQuery,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<SearchHit>>> + Send + 'a>>;
}

/// Shared handle to a [`SearchProvider`].
pub type SharedSearchProvider = Arc<dyn SearchProvider>;

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// A normalized node-change event delivered to resource subscribers.
///
/// This is the engine-facing projection of the core
/// `raisin_events::NodeEvent`; the HTTP layer maps the real bus event onto it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeChange {
    /// Workspace the changed node lives in.
    pub workspace: String,
    /// Branch the change occurred on.
    pub branch: String,
    /// Id of the changed node.
    pub node_id: String,
    /// Path of the changed node, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Node type, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_type: Option<String>,
    /// Change kind string, e.g. `"node:created"` / `"node:updated"` /
    /// `"node:deleted"`.
    pub kind: String,
}

/// A live stream of [`NodeChange`] events.
pub type NodeChangeStream =
    std::pin::Pin<Box<dyn futures::Stream<Item = NodeChange> + Send + 'static>>;

/// Source of live node-change events, bridged from the in-process event bus.
///
/// Implementors subscribe to the real bus and forward node events as
/// [`NodeChange`]s. The engine applies workspace / branch / path filtering on
/// the returned stream (see [`crate::resources`]).
pub trait EventSource: Send + Sync {
    /// Subscribe to node changes for `identity`'s tenant scope.
    ///
    /// The returned stream yields every node change visible to the bridge; the
    /// caller narrows it by workspace, branch, and path.
    fn subscribe(&self, identity: &McpIdentity) -> NodeChangeStream;
}

/// Shared handle to an [`EventSource`].
pub type SharedEventSource = Arc<dyn EventSource>;

// ---------------------------------------------------------------------------
// Asset bytes
// ---------------------------------------------------------------------------

/// Raw bytes of a `raisin:Asset` plus its resolved MIME type.
///
/// The owned result an [`AssetReader`] hands back: the file's bytes and the
/// content type to serve them with (stored `mime_type`, else guessed from the
/// path).
#[derive(Debug, Clone)]
pub struct AssetBytes {
    /// The asset's raw file bytes.
    pub bytes: Vec<u8>,
    /// MIME type to serve `bytes` with.
    pub mime_type: String,
}

/// Reads raw `raisin:Asset` bytes on behalf of the MCP engine.
///
/// Implementors resolve the node at `(workspace, path)` through the RLS-scoped
/// node service — so the read is denied when the caller may not see it — and
/// only then fetch the underlying bytes from binary storage. The repository and
/// branch come from the calling [`McpIdentity`]. This keeps the engine's "never
/// touches storage directly" rule: the hosting transport owns the byte fetch.
pub trait AssetReader: Send + Sync {
    /// Read the asset at `workspace`/`path` as `identity`.
    fn read_asset<'a>(
        &'a self,
        identity: &'a McpIdentity,
        workspace: &'a str,
        path: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<AssetBytes>> + Send + 'a>>;
}

/// Shared handle to an [`AssetReader`].
pub type SharedAssetReader = Arc<dyn AssetReader>;
