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

//! # Raisin MCP
//!
//! A complete, transport-agnostic Model Context Protocol (MCP) engine for
//! RaisinDB. Content authors declare a `raisin:McpServer` node describing an
//! endpoint — its identity, scopes, [`DataPolicy`], and custom function tools —
//! and this crate resolves that node, assembles the live tool set, and dispatches
//! JSON-RPC `initialize` / `tools/*` / `resources/*` calls against it as the
//! authenticated caller.
//!
//! The engine never touches storage directly. It reads and mutates content
//! through [`raisin_functions::FunctionApi`] (the same RLS-scoped backend
//! server-side functions use) and depends on three narrow service traits —
//! [`FunctionInvoker`], [`SearchProvider`], [`EventSource`] — that the hosting
//! transport backs with the real RaisinDB services.
//!
//! ## Layout
//!
//! - [`error`]      — [`McpError`] / [`Result`], folds in [`raisin_error::Error`].
//! - [`protocol`]   — JSON-RPC 2.0 envelopes and the typed MCP method payloads.
//! - [`identity`]   — [`McpIdentity`], scopes, and the RLS auth-context bridge.
//! - [`server`]     — the parsed [`McpServerDescriptor`] domain type.
//! - [`services`]   — service-trait abstractions the transport implements.
//! - [`registry`]   — the [`Tool`] trait, [`ToolRegistry`], and tool assembly.
//! - [`data_tools`] — built-in node / search tools and custom function tools.
//! - [`resources`]  — `raisin://` resource reads and live subscriptions.
//! - [`dispatch`]   — the [`Dispatcher`] engine routing JSON-RPC methods.
//! - [`transport`]  — the [`McpEndpoint`] byte-transport adapter.
//!
//! ## Assembling an endpoint
//!
//! ```no_run
//! use std::sync::Arc;
//! use raisin_mcp::{
//!     AssemblyServices, Dispatcher, McpEndpoint, McpIdentity,
//!     NodeResourceProvider, assemble_for_slug,
//! };
//! use raisin_functions::FunctionApi;
//!
//! # async fn example(backend: Arc<dyn FunctionApi>, discovery: Arc<dyn FunctionApi>) -> raisin_mcp::Result<()> {
//! let services = AssemblyServices { backend: backend.clone(), search: None, functions: None };
//! let identity = McpIdentity::anonymous("my-repo").with_workspace("content");
//!
//! // `discovery` resolves the server declaration (system-context); `services.backend`
//! // runs the data tools as the caller.
//! let (descriptor, registry) = assemble_for_slug(&services, &discovery, "content", "assistant").await?;
//! let dispatcher = Dispatcher::new(descriptor, registry)
//!     .with_resources(NodeResourceProvider::new(backend));
//! let endpoint = McpEndpoint::new(identity, dispatcher);
//! endpoint.run_stdio().await?;
//! # Ok(())
//! # }
//! ```

#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]

pub mod data_tools;
pub mod dispatch;
pub mod error;
pub mod identity;
pub mod protocol;
pub mod registry;
pub mod resources;
pub mod server;
pub mod services;
mod sql;
pub mod transport;

#[cfg(test)]
mod tests;

pub use data_tools::{
    build_data_tools, CreateNodeTool, DataBackend, DeleteNodeTool, FunctionTool, GetNodeTool,
    ListWorkspacesTool, QueryNodesTool, SearchNodesTool, UpdateNodeTool,
};
pub use dispatch::Dispatcher;
pub use error::{McpError, Result};
pub use identity::{McpIdentity, TenantMode, DEFAULT_BRANCH, DEFAULT_WORKSPACE};
pub use protocol::{
    is_legacy_handshake, CallToolParams, CallToolResult, ContentBlock, DiscoverResult,
    InitializeParams, InitializeResult, JsonRpcError, JsonRpcRequest, JsonRpcResponse,
    ListResourcesResult, ListToolsResult, ReadResourceParams, ReadResourceResult, RequestId,
    RequestMeta, ResourceUpdatedNotification, ServerCapabilities, ServerInfo,
    SubscribeResourceParams, SubscriptionFilter, SubscriptionsListenParams, CACHE_SCOPE_PRIVATE,
    JSONRPC_VERSION, META_CLIENT_CAPABILITIES, META_CLIENT_INFO, META_PROTOCOL_VERSION,
    META_SERVER_INFO, META_SUBSCRIPTION_ID, PROTOCOL_VERSION, RESULT_TYPE_COMPLETE,
    SUPPORTED_PROTOCOL_VERSIONS, UI_EXTENSION_ID, UI_MIME_TYPE,
};
pub use registry::{
    assemble_for_slug, assemble_registry, discover_function_tools, resolve_server_descriptor,
    AssemblyServices, DynTool, Tool, ToolDescriptor, ToolKind, ToolRegistry, DEFAULT_REPO,
    FUNCTION_NODE_TYPE, MCP_SERVER_NODE_TYPE,
};
pub use resources::{
    parse_resource_uri, resource_uri, NodeResourceProvider, ResourceContents, ResourceDescriptor,
    RESOURCE_SCHEME,
};
pub use server::{CustomTool, DataOperation, DataPolicy, McpServerDescriptor, UiBinding, UiMode};
pub use services::{
    AssetBytes, AssetReader, EventSource, FunctionInvoker, NodeChange, NodeChangeStream, SearchHit,
    SearchMode, SearchProvider, SearchQuery, SharedAssetReader, SharedEventSource,
    SharedFunctionInvoker, SharedSearchProvider,
};
pub use transport::McpEndpoint;
