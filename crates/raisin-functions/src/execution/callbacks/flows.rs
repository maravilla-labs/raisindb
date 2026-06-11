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

//! Flow execution callback: `raisin.flows.run(flowPath, input)`.
//!
//! Starts a `raisin:Flow` (functions workspace) via the same
//! transport-agnostic service the HTTP `POST /api/flows/{repo}/run`
//! handler uses (`raisin_flow_runtime::service::run_flow`), scheduling
//! the `FlowInstanceExecution` job through the unified job queue.
//! Fire-and-forget: returns `{ instance_id, job_id, status: "queued" }`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use raisin_binary::BinaryStorage;
use raisin_error::Error;
use raisin_flow_runtime::service::FlowJobScheduler;
use raisin_flow_runtime::types::FlowError;
use raisin_models::auth::AuthContext;
use raisin_storage::jobs::{JobContext, JobType};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use serde_json::{json, Value};

use crate::api::FlowRunCallback;
use crate::execution::types::ExecutionDependencies;

const TENANT_ID: &str = "default";
const DEFAULT_BRANCH: &str = "main";
const FUNCTIONS_WORKSPACE: &str = "functions";

/// [`FlowJobScheduler`] over the unified job queue (`JobRegistry` +
/// `JobDataStore`) - mirrors the `RocksDBStorage` implementation in
/// `raisin-rocksdb/src/jobs/flow_scheduler.rs`, but works from the
/// function-execution dependencies instead of the storage handle.
struct JobQueueFlowScheduler {
    job_registry: Arc<raisin_storage::jobs::JobRegistry>,
    job_data_store: Arc<raisin_rocksdb::JobDataStore>,
}

#[async_trait]
impl FlowJobScheduler for JobQueueFlowScheduler {
    async fn schedule_flow_job(
        &self,
        repo: &str,
        job_type: JobType,
        metadata: HashMap<String, Value>,
    ) -> Result<String, FlowError> {
        let context = JobContext {
            tenant_id: TENANT_ID.to_string(),
            repo_id: repo.to_string(),
            branch: DEFAULT_BRANCH.to_string(),
            workspace_id: FUNCTIONS_WORKSPACE.to_string(),
            revision: raisin_hlc::HLC::new(0, 0),
            metadata,
        };

        // Store job context BEFORE registering so dispatch can never
        // observe the job without its context.
        let job_id = raisin_storage::jobs::JobId::new();
        self.job_data_store
            .put(&job_id, &context)
            .map_err(|e| FlowError::Other(e.to_string()))?;

        self.job_registry
            .register_job_with_id(
                job_id.clone(),
                job_type,
                TENANT_ID.to_string(),
                None,
                None,
                None,
            )
            .await
            .map_err(|e| FlowError::Other(e.to_string()))?;

        Ok(job_id.to_string())
    }

    async fn cancel_flow_jobs(&self, instance_id: &str) -> Result<(), FlowError> {
        let jobs = self.job_registry.list_jobs().await;
        for job in jobs {
            if let JobType::FlowInstanceExecution {
                instance_id: ref job_instance_id,
                ..
            } = job.job_type
            {
                if job_instance_id == instance_id {
                    let _ = self.job_registry.cancel_job(&job.id).await;
                    break;
                }
            }
        }
        Ok(())
    }
}

/// Create the flow_run callback: `raisin.flows.run(flowPath, input)`.
///
/// The starting actor is taken from the execution's auth context (falling
/// back to `"function"`), so flow instances record who triggered them.
pub fn create_flow_run<S, B>(
    deps: Arc<ExecutionDependencies<S, B>>,
    job_registry: Arc<raisin_storage::jobs::JobRegistry>,
    job_data_store: Arc<raisin_rocksdb::JobDataStore>,
    repo_id: String,
    auth_context: Option<AuthContext>,
) -> FlowRunCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    Arc::new(move |flow_path: String, input: Value| {
        let deps = deps.clone();
        let scheduler = JobQueueFlowScheduler {
            job_registry: job_registry.clone(),
            job_data_store: job_data_store.clone(),
        };
        let repo = repo_id.clone();
        let actor = auth_context
            .as_ref()
            .and_then(|a| a.user_id.clone())
            .unwrap_or_else(|| "function".to_string());
        let actor_home = auth_context.as_ref().and_then(|a| a.home.clone());

        Box::pin(async move {
            // Normalize the flow path: ensure a leading slash for the lookup
            let flow_path = if flow_path.starts_with('/') {
                flow_path
            } else {
                format!("/{}", flow_path)
            };

            // Tolerate JSON-stringified input (some wrapper layers pass
            // objects through JSON.stringify)
            let input = match input {
                Value::String(ref s) => {
                    serde_json::from_str::<Value>(s).unwrap_or_else(|_| input.clone())
                }
                other => other,
            };

            let result = raisin_flow_runtime::service::run_flow(
                deps.storage.as_ref(),
                &scheduler,
                &repo,
                &flow_path,
                input,
                actor,
                actor_home,
            )
            .await
            .map_err(|e| Error::Backend(format!("Failed to start flow '{}': {}", flow_path, e)))?;

            tracing::info!(
                flow_path = %flow_path,
                instance_id = %result.instance_id,
                job_id = %result.job_id,
                "raisin.flows.run queued flow instance"
            );

            Ok(json!({
                "instance_id": result.instance_id,
                "job_id": result.job_id,
                "status": "queued",
            }))
        })
    })
}
