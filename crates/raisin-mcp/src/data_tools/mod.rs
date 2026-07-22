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

//! Built-in data tools that expose RaisinDB content over MCP.
//!
//! Each tool wraps a shared [`FunctionApi`] backend — the same RLS-scoped API
//! surface server-side functions run against — so the MCP server reads, queries,
//! and mutates the exact same node store with no duplicated data path.
//!
//! [`build_data_tools`] turns a server's [`DataPolicy`](crate::server::DataPolicy)
//! into the concrete, erased tool set: each enabled
//! [`DataOperation`](crate::server::DataOperation) yields one tool, scoped to the
//! policy's workspaces. `search_nodes` additionally requires a
//! [`SearchProvider`](crate::services::SearchProvider).

mod function;
mod nodes;
mod search;

use std::sync::Arc;

use raisin_functions::FunctionApi;

use crate::registry::DynTool;
use crate::server::{DataOperation, DataPolicy};
use crate::services::SharedSearchProvider;

pub use function::FunctionTool;
pub use nodes::{
    CreateNodeTool, DeleteNodeTool, GetNodeTool, ListWorkspacesTool, QueryNodesTool, UpdateNodeTool,
};
pub use search::SearchNodesTool;

/// Shared handle to the RaisinDB data backend used by data tools.
pub type DataBackend = Arc<dyn FunctionApi>;

/// Build the erased built-in data tools enabled by `policy`.
///
/// One tool is produced per enabled operation. `search_nodes` is only emitted
/// when a `search` provider is supplied; if the policy enables it without one,
/// the operation is silently skipped (the transport simply did not wire search).
pub fn build_data_tools(
    policy: &DataPolicy,
    backend: DataBackend,
    search: Option<SharedSearchProvider>,
) -> Vec<Arc<dyn DynTool>> {
    let mut tools: Vec<Arc<dyn DynTool>> = Vec::new();
    let workspaces = policy.workspaces.clone();
    // Shared, immutable set of workspaces the caller may target via a tool's
    // optional `workspace` argument (defaults to the active workspace).
    let allowed: Arc<Vec<String>> = Arc::new(workspaces.clone());

    for op in DataOperation::ALL {
        if !policy.allows(op) {
            continue;
        }
        match op {
            DataOperation::QueryNodes => {
                tools.push(Arc::new(QueryNodesTool::new(backend.clone(), allowed.clone())));
            }
            DataOperation::GetNode => {
                tools.push(Arc::new(GetNodeTool::new(backend.clone(), allowed.clone())));
            }
            DataOperation::SearchNodes => {
                if let Some(search) = &search {
                    tools.push(Arc::new(SearchNodesTool::new(search.clone(), allowed.clone())));
                }
            }
            DataOperation::CreateNode => {
                tools.push(Arc::new(CreateNodeTool::new(backend.clone(), allowed.clone())));
            }
            DataOperation::UpdateNode => {
                tools.push(Arc::new(UpdateNodeTool::new(backend.clone(), allowed.clone())));
            }
            DataOperation::DeleteNode => {
                tools.push(Arc::new(DeleteNodeTool::new(backend.clone(), allowed.clone())));
            }
            DataOperation::ListWorkspaces => {
                tools.push(Arc::new(ListWorkspacesTool::new(workspaces.clone())));
            }
        }
    }

    tools
}
