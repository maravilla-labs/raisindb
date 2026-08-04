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

#![warn(missing_docs)]

//! Model Context Protocol wire types, and the outbound MCP **client**.
//!
//! Split out of `raisin-mcp` for one concrete reason: `raisin-mcp` serves
//! RaisinDB's own tools, so it depends on `raisin-functions` — which in turn
//! depends on `raisin-rocksdb`. Anything below that line (a job handler, the
//! storage layer) therefore cannot depend on `raisin-mcp` without closing a
//! dependency cycle, and Cargo rejects package cycles regardless of features.
//!
//! Nothing here knows how to *serve* MCP. This crate holds only what both
//! directions genuinely share — the JSON-RPC envelopes, the typed payloads, the
//! content blocks — plus [`client`], which needs none of the server machinery.
//! `raisin-mcp` depends on this crate and re-exports it, so
//! `raisin_mcp::protocol::…` paths keep resolving unchanged.

pub mod client;
pub mod content;
pub mod error;
pub mod props;
pub mod protocol;
pub mod resource_types;

pub use content::ContentBlock;
pub use error::{McpError, Result};
pub use resource_types::{
    parse_resource_uri, resource_uri, ResourceContents, ResourceDescriptor, RESOURCE_SCHEME,
};
