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

//! Finding every MCP connection in the deployment.
//!
//! Three subsystems need this walk — the discovery check, the token-refresh
//! sweep, and the notification listener — and it had already been written twice
//! before the third arrived. Each copy is a place for the scoping to drift: a
//! filter applied in one and not another means a connection quietly served by
//! two of the three and not the third, with no error anywhere.
//!
//! The walk is always tenants → repos → **default branch** → `raisin:system`.
//! Connections live on the default branch only; a feature branch inherits
//! whatever branching copied and is never independently scanned, or N branches
//! would mean N× the writes and N× the load on somebody else's server.

use std::sync::Arc;

use raisin_error::Result;
use raisin_mcp_protocol::client::McpConnectionDescriptor;
use raisin_models::nodes::Node;

use super::{apply, CONNECTION_NODE_TYPE, SYSTEM_WORKSPACE};
use crate::RocksDBStorage;

/// One connection, with the scope needed to act on it.
pub struct ConnectionEntry {
    pub tenant: String,
    pub repo: String,
    pub branch: String,
    pub node: Node,
    pub descriptor: McpConnectionDescriptor,
}

/// Every connection across the deployment, or one tenant's when filtered.
///
/// A repo whose scan fails is logged and skipped rather than aborting the walk:
/// one broken repo must not stop the other ninety-nine from being served.
pub async fn all_connections(
    storage: &Arc<RocksDBStorage>,
    tenant_filter: Option<&str>,
) -> Result<Vec<ConnectionEntry>> {
    let tenants = match tenant_filter {
        Some(t) => vec![t.to_string()],
        None => crate::management::list_tenants(storage).await?,
    };

    let mut found = Vec::new();
    for tenant in tenants {
        let repos = match crate::management::list_repositories(storage, &tenant).await {
            Ok(repos) => repos,
            Err(e) => {
                tracing::warn!(%tenant, error = %e, "mcp: failed to list repos");
                continue;
            }
        };
        for repo in repos {
            match repo_connections(storage, &tenant, &repo).await {
                Ok(mut entries) => found.append(&mut entries),
                Err(e) => tracing::warn!(%tenant, %repo, error = %e, "mcp: repo scan failed"),
            }
        }
    }
    Ok(found)
}

/// Connections in one repo, on its default branch.
pub async fn repo_connections(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
) -> Result<Vec<ConnectionEntry>> {
    let branch = apply::default_branch(storage, tenant, repo).await;
    let svc = apply::system_service(storage, tenant, repo, &branch, SYSTEM_WORKSPACE);

    let mut entries = Vec::new();
    for node in svc.list_by_type(CONNECTION_NODE_TYPE).await? {
        // A node that will not parse is skipped here and reported by the
        // surfaces that can show it — this walk is not the place to raise it.
        let Ok(descriptor) = McpConnectionDescriptor::from_node(&node) else {
            continue;
        };
        entries.push(ConnectionEntry {
            tenant: tenant.to_string(),
            repo: repo.to_string(),
            branch: branch.clone(),
            node,
            descriptor,
        });
    }
    Ok(entries)
}
