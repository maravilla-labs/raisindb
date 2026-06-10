// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Flow wait sweeper - backstop for flow wait deadlines.
//!
//! The primary mechanism for waking waiting flows is a scheduled
//! `FlowInstanceExecution { execution_type: "timeout_check" }` job queued at
//! the wait deadline (see `handle_wait_result` / `handle_error_result` in
//! raisin-flow-runtime). Those jobs are persisted and survive restarts.
//!
//! This sweeper covers the cases where that job is missing anyway: queueing
//! failed at wait time, the job was cleaned up, or clock skew. It
//! periodically scans `raisin:FlowInstance` nodes across all repositories
//! and enqueues an idempotent timeout-check job for every waiting instance
//! whose deadline has passed.
//!
//! `check_flow_timeout` itself is a no-op for waits that are not due, so a
//! spurious enqueue is harmless.

#![cfg(feature = "storage-rocksdb")]

use std::sync::Arc;

use raisin_models::nodes::properties::PropertyValue;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::jobs::{JobContext, JobId, JobType};
use raisin_storage::{ListOptions, NodeRepository, Storage, StorageScope};

/// Workspace where flow instances are persisted (matches the flow callbacks'
/// default `flows_workspace`).
const FLOWS_WORKSPACE: &str = "raisin:system";

/// Branch scanned for flow instances (flow execution currently runs on main).
const FLOWS_BRANCH: &str = "main";

/// Spawn the periodic flow wait sweeper.
pub fn spawn_flow_wait_sweeper(storage: Arc<RocksDBStorage>, interval_secs: u64) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Skip the immediate first tick so we don't race server startup.
        interval.tick().await;
        loop {
            interval.tick().await;
            let (scanned, enqueued) = sweep_once(&storage).await;
            tracing::debug!(
                scanned = scanned,
                enqueued = enqueued,
                "Flow sweeper: pass complete"
            );
        }
    });
    tracing::info!(interval_secs = interval_secs, "Flow wait sweeper started");
}

/// Scan all repositories for waiting flow instances with due deadlines (or
/// stale Running instances) and enqueue idempotent recovery jobs for them.
///
/// Returns (instances scanned, jobs enqueued).
async fn sweep_once(storage: &Arc<RocksDBStorage>) -> (usize, usize) {
    let mut scanned = 0usize;
    let mut enqueued = 0usize;

    // Enumerate tenants -> repos via the background-job helpers (the
    // RepositoryManagementRepository facade is tenant-scoped and returns
    // nothing here)
    let tenants = match raisin_rocksdb::management::list_tenants(storage).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "Flow sweeper: failed to list tenants");
            return (scanned, enqueued);
        }
    };

    let mut repos: Vec<(String, String)> = Vec::new();
    for tenant in tenants {
        match raisin_rocksdb::management::list_repositories(storage, &tenant).await {
            Ok(repo_ids) => {
                repos.extend(repo_ids.into_iter().map(|r| (tenant.clone(), r)));
            }
            Err(e) => {
                tracing::warn!(tenant = %tenant, error = %e, "Flow sweeper: failed to list repositories");
            }
        }
    }

    let now = chrono::Utc::now();

    for (tenant_id, repo_id) in repos {
        let scope = StorageScope::new(&tenant_id, &repo_id, FLOWS_BRANCH, FLOWS_WORKSPACE);

        // Resolve the instances folder and list its children by parent id.
        // (Path-based listing is authoritative; the __node_type property
        // index can lag behind for frequently-updated instance nodes.)
        let instances_folder = match storage
            .nodes()
            .get_by_path(scope, "/flows/instances", None)
            .await
        {
            Ok(Some(folder)) => folder,
            Ok(None) => continue, // repo has no flows yet
            Err(e) => {
                tracing::debug!(
                    tenant = %tenant_id,
                    repo = %repo_id,
                    error = %e,
                    "Flow sweeper: failed to resolve /flows/instances (skipping repo)"
                );
                continue;
            }
        };

        let instances = match storage
            .nodes()
            .list_by_parent(scope, &instances_folder.id, ListOptions::default())
            .await
        {
            Ok(nodes) => nodes,
            Err(e) => {
                tracing::debug!(
                    tenant = %tenant_id,
                    repo = %repo_id,
                    error = %e,
                    "Flow sweeper: failed to list flow instances (skipping repo)"
                );
                continue;
            }
        };

        for node in instances {
            scanned += 1;
            let status = match node.properties.get("status") {
                Some(PropertyValue::String(s)) => s.as_str(),
                _ => continue,
            };

            let instance_id = match node.properties.get("id") {
                Some(PropertyValue::String(s)) => s.clone(),
                _ => continue,
            };

            match status {
                // Waiting flows: enforce their deadline
                "waiting" => {
                    let Some(PropertyValue::Object(wait_info)) = node.properties.get("wait_info")
                    else {
                        continue;
                    };

                    let Some(PropertyValue::String(timeout_at)) = wait_info.get("timeout_at")
                    else {
                        // No deadline configured - nothing to enforce
                        continue;
                    };

                    let Ok(timeout_at) = chrono::DateTime::parse_from_rfc3339(timeout_at) else {
                        continue;
                    };
                    if timeout_at.with_timezone(&chrono::Utc) > now {
                        continue;
                    }

                    enqueue_flow_job(
                        storage,
                        &tenant_id,
                        &repo_id,
                        &instance_id,
                        "timeout_check",
                        "flow-timeout",
                    )
                    .await;
                    enqueued += 1;
                }
                // Stale Running flows: a crash mid-execution (process kill,
                // OOM, deploy) leaves the instance Running forever. After a
                // generous staleness window, re-enqueue execution -
                // execute_flow continues from current_node_id and is
                // OCC-protected against racing an actually-live execution.
                "running" => {
                    // Staleness: prefer the node's updated_at; fall back to
                    // the instance's started_at (listings may not populate
                    // node timestamps)
                    let last_activity =
                        node.updated_at
                            .or_else(|| match node.properties.get("started_at") {
                                Some(PropertyValue::String(s)) => {
                                    chrono::DateTime::parse_from_rfc3339(s)
                                        .ok()
                                        .map(|t| t.with_timezone(&chrono::Utc))
                                }
                                Some(PropertyValue::Date(d)) => Some((*d).into()),
                                _ => None,
                            });
                    let stale = last_activity
                        .map(|t| now - t > chrono::Duration::minutes(STALE_RUNNING_MINUTES))
                        .unwrap_or(false);
                    if stale {
                        tracing::warn!(
                            instance_id = %instance_id,
                            tenant = %tenant_id,
                            repo = %repo_id,
                            "Flow sweeper: recovering stale Running instance"
                        );
                        enqueue_flow_job(
                            storage,
                            &tenant_id,
                            &repo_id,
                            &instance_id,
                            "start",
                            "flow-recover",
                        )
                        .await;
                        enqueued += 1;
                    }
                }
                _ => {}
            }
        }
    }

    (scanned, enqueued)
}

/// Minutes of inactivity after which a Running instance is considered
/// crashed and re-executed. Generous to avoid re-entering long AI calls.
const STALE_RUNNING_MINUTES: i64 = 15;

/// Register an idempotent flow job (timeout check or recovery execution)
/// for one flow instance.
///
/// Stores the job context first, then registers with a pre-generated ID so
/// the worker never picks up a job whose context is missing.
async fn enqueue_flow_job(
    storage: &Arc<RocksDBStorage>,
    tenant_id: &str,
    repo_id: &str,
    instance_id: &str,
    execution_type: &str,
    dedup_prefix: &str,
) {
    let job_id = JobId::new();
    let context = JobContext {
        tenant_id: tenant_id.to_string(),
        repo_id: repo_id.to_string(),
        branch: FLOWS_BRANCH.to_string(),
        workspace_id: FLOWS_WORKSPACE.to_string(),
        revision: raisin_hlc::HLC::new(0, 0),
        metadata: std::collections::HashMap::new(),
    };

    if let Err(e) = storage.job_data_store().put(&job_id, &context) {
        tracing::warn!(
            instance_id = %instance_id,
            error = %e,
            "Flow sweeper: failed to store flow job context"
        );
        return;
    }

    let job_type = JobType::FlowInstanceExecution {
        instance_id: instance_id.to_string(),
        execution_type: execution_type.to_string(),
        resume_reason: Some("sweeper".to_string()),
    };

    match storage
        .job_registry()
        .register_job_with_id_idempotent(
            job_id.clone(),
            job_type,
            tenant_id.to_string(),
            format!("{}-{}", dedup_prefix, instance_id),
            Some(0),
        )
        .await
    {
        Ok(true) => {
            tracing::info!(
                instance_id = %instance_id,
                tenant = %tenant_id,
                repo = %repo_id,
                execution_type = %execution_type,
                "Flow sweeper: enqueued flow job"
            );
        }
        Ok(false) => {
            // An active check already exists - drop the orphan context
            let _ = storage.job_data_store().delete(tenant_id, &job_id);
        }
        Err(e) => {
            tracing::warn!(
                instance_id = %instance_id,
                error = %e,
                "Flow sweeper: failed to register flow job"
            );
            let _ = storage.job_data_store().delete(tenant_id, &job_id);
        }
    }
}
