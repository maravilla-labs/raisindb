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

//! The `search_nodes` data tool, backed by a [`SearchProvider`].
//!
//! Wires the MCP `search_nodes` tool to the full-text / vector search engines
//! the hosting transport supplies. The query resolves against the caller's
//! active workspace and branch.

use serde_json::{json, Value};

use crate::data_tools::nodes::{resolve_workspace, workspace_arg_schema, AllowedWorkspaces};
use crate::error::{McpError, Result};
use crate::identity::McpIdentity;
use crate::registry::{Tool, ToolDescriptor, ToolKind};
use crate::services::{SearchMode, SearchQuery, SharedSearchProvider};

/// Default number of hits returned when the caller does not request a limit.
const DEFAULT_LIMIT: usize = 20;

/// Largest number of hits a single `search_nodes` call returns.
const MAX_LIMIT: usize = 200;

/// `search_nodes` — full-text or vector search over a workspace.
pub struct SearchNodesTool {
    search: SharedSearchProvider,
    allowed: AllowedWorkspaces,
}

impl SearchNodesTool {
    /// Wrap a search provider as the `search_nodes` tool.
    pub fn new(search: SharedSearchProvider, allowed: AllowedWorkspaces) -> Self {
        Self { search, allowed }
    }
}

impl Tool for SearchNodesTool {
    fn name(&self) -> &str {
        "search_nodes"
    }

    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            "search_nodes",
            "Search nodes in a workspace by full-text or vector similarity.",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query text." },
                    "mode": {
                        "type": "string",
                        "enum": ["fulltext", "vector"],
                        "description": "Search mode: lexical (default) or semantic."
                    },
                    "node_type": { "type": "string", "description": "Restrict to an exact node type." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": MAX_LIMIT, "description": "Maximum hits to return." },
                    "workspace": workspace_arg_schema()
                },
                "required": ["query"]
            }),
            ToolKind::Data,
        )
    }

    async fn call(&self, identity: &McpIdentity, args: Value) -> Result<Value> {
        let workspace = resolve_workspace(&args, identity, &self.allowed)?;
        let query_text = args
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| McpError::invalid_params("missing required string field `query`"))?;

        let mode = match args.get("mode").and_then(Value::as_str) {
            None | Some("fulltext") => SearchMode::Fulltext,
            Some("vector") => SearchMode::Vector,
            Some(other) => {
                return Err(McpError::invalid_params(format!(
                    "unknown search mode `{other}` (expected `fulltext` or `vector`)"
                )))
            }
        };

        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map(|n| (n as usize).min(MAX_LIMIT))
            .unwrap_or(DEFAULT_LIMIT);

        let node_type = args
            .get("node_type")
            .and_then(Value::as_str)
            .map(str::to_string);

        let query = SearchQuery {
            workspace: workspace.into_owned(),
            branch: identity.branch.clone(),
            query: query_text.to_string(),
            mode,
            node_type,
            limit,
        };

        let hits = self.search.search(identity, query).await?;
        Ok(json!({ "hits": hits }))
    }
}
