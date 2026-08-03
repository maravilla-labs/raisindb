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

//! Event-driven invalidation for the cached MCP server plans.
//!
//! The plan (parsed `raisin:McpServer` descriptor + resolved tool definitions)
//! is expensive to build and identical for every caller, so it is cached. It
//! must not go stale: editing a function's description or adding a tool has to
//! show up in the next `tools/list`, because that is exactly what a package
//! author is doing when they notice.

use std::sync::Arc;

use raisin_core::TtlCache;
use raisin_events::{Event, EventHandler};

/// Drops cached MCP plans when a node that feeds one is written.
pub struct McpPlanCacheInvalidator {
    cache: Arc<TtlCache<Arc<raisin_mcp::McpServerPlan>>>,
}

impl McpPlanCacheInvalidator {
    /// Wrap the cache held on `AppState`.
    pub fn new(cache: Arc<TtlCache<Arc<raisin_mcp::McpServerPlan>>>) -> Self {
        Self { cache }
    }

    /// Whether this event could change any server's tool set.
    ///
    /// Deliberately generous. A `raisin:Function` write matters because tool
    /// schemas and descriptions are inherited from it, and a function-side
    /// `mcp` block declares a tool outright. A `raisin:McpServer` write is the
    /// server itself. Anything else cannot affect a plan.
    fn affects_a_plan(event: &Event) -> bool {
        let Event::Node(node) = event else {
            return false;
        };
        match node.node_type.as_deref() {
            Some(raisin_mcp::MCP_SERVER_NODE_TYPE) | Some(raisin_mcp::FUNCTION_NODE_TYPE) => true,
            // A delete may arrive without a node type; the workspace it landed
            // in is the fallback signal. Missing an invalidation is worse than
            // an extra one — a stale plan is visible to the author, a dropped
            // entry costs one rebuild.
            None => matches!(
                node.workspace_id.as_str(),
                super::MCP_DISCOVERY_WORKSPACE | "functions"
            ),
            Some(_) => false,
        }
    }
}

impl EventHandler for McpPlanCacheInvalidator {
    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if !Self::affects_a_plan(event) {
                return Ok(());
            }
            // Coarse on purpose: the cache is keyed by slug, and a function
            // write does not say which servers reference it — resolving that
            // would mean rebuilding the very plans we are invalidating. Writes
            // to these node types are an authoring action, not traffic, so the
            // cost is one rebuild on the next request per active server.
            self.cache.invalidate_all();
            tracing::debug!("MCP plan cache invalidated by a server/function write");
            Ok(())
        })
    }

    fn name(&self) -> &str {
        "mcp-plan-cache-invalidator"
    }
}
