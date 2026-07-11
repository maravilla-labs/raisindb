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

//! Integration / mount callback: `raisin.integrations.sync_now(mountId, mode?)`.
//!
//! Enqueues a one-shot `VirtualMountSync` job on the unified job queue,
//! deduped per mount via the `vmount-sync:{mount_id}` key. This mirrors the
//! HTTP `POST /api/{repo}/{branch}/mounts/{mount_id}/sync` handler
//! (`raisin-transport-http/.../integrations/manage.rs::sync_mount`) exactly so
//! a provider webhook and an operator's manual "sync now" collapse into the
//! same in-flight job. Fire-and-forget: returns
//! `{ "job_id": String|null, "status": "queued"|"already_running" }`.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_error::Error;
use raisin_storage::jobs::{JobContext, JobId, JobType};
use serde_json::json;

use crate::api::IntegrationsSyncNowCallback;

/// Config branch/workspace the sync job context is stamped with. The mount's
/// own config resolves the target branch inside the sync engine; these match
/// the HTTP handler's constants.
const CONFIG_BRANCH: &str = "main";
const CONFIG_WORKSPACE: &str = "raisin:system";

/// Create the `integrations_sync_now` callback backing
/// `raisin.integrations.sync_now(mountId, mode?)`.
pub fn create_integrations_sync_now(
    job_registry: Arc<raisin_storage::jobs::JobRegistry>,
    job_data_store: Arc<raisin_rocksdb::JobDataStore>,
    tenant_id: String,
    repo_id: String,
) -> IntegrationsSyncNowCallback {
    Arc::new(move |mount_id: String, mode: Option<String>| {
        let job_registry = job_registry.clone();
        let job_data_store = job_data_store.clone();
        let tenant_id = tenant_id.clone();
        let repo_id = repo_id.clone();

        Box::pin(async move {
            let mode = mode.unwrap_or_else(|| "delta".to_string());

            let job_type = JobType::VirtualMountSync {
                mount_id: mount_id.clone(),
                mode,
            };
            let job_id = JobId::new();
            let context = JobContext {
                tenant_id: tenant_id.clone(),
                repo_id: repo_id.clone(),
                branch: CONFIG_BRANCH.to_string(),
                workspace_id: CONFIG_WORKSPACE.to_string(),
                revision: raisin_hlc::HLC::now(),
                metadata: HashMap::new(),
            };

            // Store the job context BEFORE registering so dispatch can never
            // observe the job without its context.
            job_data_store
                .put(&job_id, &context)
                .map_err(|e| Error::Backend(format!("failed to store sync job context: {e}")))?;

            let registered = job_registry
                .register_job_with_id_idempotent(
                    job_id.clone(),
                    job_type,
                    tenant_id.clone(),
                    format!("vmount-sync:{}", mount_id),
                    None,
                )
                .await
                .map_err(|e| Error::Backend(format!("failed to enqueue sync job: {e}")))?;

            if registered {
                Ok(json!({ "job_id": job_id.0, "status": "queued" }))
            } else {
                Ok(json!({ "job_id": null, "status": "already_running" }))
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dedup contract the callback relies on: two registrations sharing the
    /// `vmount-sync:{mount_id}` key collapse to one active job — the first
    /// returns `queued`, the second `already_running`. (The `JobDataStore.put`
    /// leg needs a RocksDB handle, so this exercises the registry dedup path
    /// that decides `queued` vs `already_running`.)
    #[tokio::test]
    async fn test_sync_now_dedup_queued_then_already_running() {
        let registry = raisin_storage::jobs::JobRegistry::new();
        let mount_id = "mount-abc";
        let dedup_key = format!("vmount-sync:{}", mount_id);

        let make_job = || JobType::VirtualMountSync {
            mount_id: mount_id.to_string(),
            mode: "delta".to_string(),
        };

        let first = registry
            .register_job_with_id_idempotent(
                JobId::new(),
                make_job(),
                "default".to_string(),
                dedup_key.clone(),
                None,
            )
            .await
            .unwrap();
        assert!(first, "first sync should be queued");

        let second = registry
            .register_job_with_id_idempotent(
                JobId::new(),
                make_job(),
                "default".to_string(),
                dedup_key.clone(),
                None,
            )
            .await
            .unwrap();
        assert!(!second, "second sync should dedup to already_running");
    }
}
