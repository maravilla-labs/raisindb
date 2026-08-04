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

//! Periodic scan: enqueue `McpToolDiscovery` for connections that are due.
//!
//! Mirrors `virtual_mount_sync::check`. The scan itself is cheap and idempotent
//! — it only enqueues — so it deliberately takes no lease. Every node may run
//! it; the per-connection lease inside the discovery job is what keeps the
//! actual writes single-fire.

use std::sync::Arc;

use raisin_error::Result;
use raisin_mcp_protocol::client::McpConnectionDescriptor;
use raisin_storage::jobs::{JobContext, JobId, JobType};
use serde_json::Value;

use super::apply::{default_branch, system_service};
use super::{CONNECTION_NODE_TYPE, SYSTEM_WORKSPACE};
use crate::RocksDBStorage;

/// Scan every repo for connections whose refresh interval is due.
///
/// Returns how many discovery jobs were enqueued.
pub async fn run_check(
    storage: &Arc<RocksDBStorage>,
    tenant_filter: Option<String>,
) -> Result<usize> {
    let tenants = match &tenant_filter {
        Some(t) => vec![t.clone()],
        None => crate::management::list_tenants(storage).await?,
    };

    let now_secs = chrono::Utc::now().timestamp().max(0) as u64;
    let mut enqueued = 0usize;

    for tenant in tenants {
        let repos = match crate::management::list_repositories(storage, &tenant).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(tenant = %tenant, error = %e, "mcp discovery check: failed to list repos");
                continue;
            }
        };
        for repo in repos {
            match scan_repo(storage, &tenant, &repo, now_secs).await {
                Ok(n) => enqueued += n,
                Err(e) => tracing::warn!(
                    tenant = %tenant,
                    repo = %repo,
                    error = %e,
                    "mcp discovery check: repo scan failed"
                ),
            }
        }
    }
    Ok(enqueued)
}

/// Enqueue discovery for every due connection in one repo.
async fn scan_repo(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    now_secs: u64,
) -> Result<usize> {
    let branch = default_branch(storage, tenant, repo).await;
    let svc = system_service(storage, tenant, repo, &branch, SYSTEM_WORKSPACE);

    let mut enqueued = 0usize;
    for node in svc.list_by_type(CONNECTION_NODE_TYPE).await? {
        let Ok(descriptor) = McpConnectionDescriptor::from_node(&node) else {
            continue;
        };
        if !descriptor.is_callable() || !descriptor.refresh_policy.is_scheduled() {
            continue;
        }

        let last = node
            .properties
            .get("last_synced_at")
            .and_then(|pv| serde_json::to_value(pv).ok())
            .as_ref()
            .and_then(Value::as_str)
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.timestamp().max(0) as u64);

        if !is_due(last, descriptor.refresh_policy.interval_secs, now_secs) {
            continue;
        }
        if enqueue_discovery(storage, tenant, repo, &branch, &descriptor.slug).await? {
            enqueued += 1;
        }
    }
    Ok(enqueued)
}

/// Whether a connection last synced at `last` is due again.
///
/// A connection that has never synced is always due — that is what makes a
/// freshly created connection pick up its tools without a manual refresh.
pub(crate) fn is_due(last_synced: Option<u64>, interval_secs: u64, now_secs: u64) -> bool {
    match last_synced {
        None => true,
        Some(last) => now_secs.saturating_sub(last) >= interval_secs,
    }
}

/// Enqueue one discovery job. Returns whether it was newly registered.
async fn enqueue_discovery(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    branch: &str,
    slug: &str,
) -> Result<bool> {
    let job_type = JobType::McpToolDiscovery {
        connection_slug: slug.to_string(),
        tenant_id: tenant.to_string(),
        repo_id: repo.to_string(),
    };
    let job_id = JobId::new();
    let context = JobContext {
        tenant_id: tenant.to_string(),
        repo_id: repo.to_string(),
        branch: branch.to_string(),
        workspace_id: SYSTEM_WORKSPACE.to_string(),
        revision: raisin_hlc::HLC::now(),
        metadata: std::collections::HashMap::new(),
    };
    // Context before registration: the dispatcher can pick the job up the
    // instant it is registered.
    storage.job_data_store().put(&job_id, &context)?;

    let dedup = format!("mcp-discovery:{tenant}:{repo}:{slug}");
    let registered = storage
        .job_registry()
        .register_job_with_id_idempotent(job_id, job_type, tenant.to_string(), dedup, None)
        .await?;
    Ok(registered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_connection_that_never_synced_is_due() {
        // Otherwise a newly created connection sits toolless until someone
        // presses refresh.
        assert!(is_due(None, 3_600, 1_000));
    }

    #[test]
    fn due_only_after_the_interval_elapses() {
        let now = 10_000;
        assert!(!is_due(Some(now - 100), 3_600, now));
        assert!(is_due(Some(now - 3_600), 3_600, now));
        assert!(is_due(Some(now - 7_200), 3_600, now));
    }

    #[test]
    fn a_future_timestamp_does_not_underflow_into_due() {
        // Clock skew across nodes must not make everything permanently due.
        let now = 1_000;
        assert!(!is_due(Some(now + 5_000), 3_600, now));
    }
}
