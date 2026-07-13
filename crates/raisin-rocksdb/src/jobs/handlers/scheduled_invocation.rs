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

//! One-shot scheduled invocation handler
//!
//! Processes `JobType::ScheduledInvocation` jobs: a one-shot invocation of a
//! function or flow at a fixed point in time. The job itself is registered
//! with a future `scheduled_at` (via `register_job_at_with_id`), so the
//! delayed-dispatch monitor holds it until the time arrives; this handler
//! then routes it to its target:
//!
//! - `target_kind == "function"`: enqueues a regular `FunctionExecution` job
//!   carrying the persisted input.
//! - `target_kind == "flow"`: starts a flow instance via the transport-provided
//!   [`FlowStartCallback`].
//!
//! All rich invocation data (target path, input, actor, external key,
//! scheduled time) lives in the persisted `JobContext`, keeping the
//! `JobType` variant minimal and round-trip safe.

use raisin_error::{Error, Result};
use raisin_storage::jobs::{JobContext, JobInfo, JobRegistry, JobType};
use std::collections::HashMap;
use std::sync::Arc;

use crate::jobs::data_store::JobDataStore;
use crate::jobs::dispatcher::JobDispatcher;

/// JobContext metadata key: target function/flow path.
pub const META_TARGET_PATH: &str = "target_path";
/// JobContext metadata key: invocation input value.
pub const META_INPUT: &str = "input";
/// JobContext metadata key: actor that scheduled the invocation.
pub const META_ACTOR: &str = "actor";
/// JobContext metadata key: caller-supplied idempotency/lookup key.
pub const META_EXTERNAL_KEY: &str = "external_key";
/// JobContext metadata key: RFC3339 time the invocation was scheduled for.
pub const META_SCHEDULED_FOR: &str = "scheduled_for";

/// Callback type for starting a flow instance.
///
/// Provided at job-system init (it needs the flow runtime + storage handle,
/// which the handler layer does not depend on directly).
/// Arguments: (tenant_id, repo_id, branch, flow_path, input, actor).
/// Returns the flow-start result, e.g. `{ instance_id, job_id, status }`.
pub type FlowStartCallback = Arc<
    dyn Fn(
            String,            // tenant_id
            String,            // repo_id
            String,            // branch
            String,            // flow_path
            serde_json::Value, // input
            String,            // actor
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<serde_json::Value>> + Send>,
        > + Send
        + Sync,
>;

/// Handler for one-shot scheduled invocation jobs
///
/// Routes a due `ScheduledInvocation` job to its target: functions are
/// enqueued as `FunctionExecution` jobs, flows are started via the
/// configured [`FlowStartCallback`].
pub struct ScheduledInvocationHandler {
    /// Job registry for enqueueing function execution jobs
    job_registry: Arc<JobRegistry>,
    /// Job data store for storing job context
    job_data_store: Arc<JobDataStore>,
    /// Job dispatcher for routing jobs to worker queues
    dispatcher: Arc<JobDispatcher>,
    /// Optional callback to start flow instances (set at init)
    flow_starter: Option<FlowStartCallback>,
}

impl ScheduledInvocationHandler {
    /// Create a new scheduled invocation handler
    pub fn new(
        job_registry: Arc<JobRegistry>,
        job_data_store: Arc<JobDataStore>,
        dispatcher: Arc<JobDispatcher>,
    ) -> Self {
        Self {
            job_registry,
            job_data_store,
            dispatcher,
            flow_starter: None,
        }
    }

    /// Set the flow starter callback
    ///
    /// This should be provided at job-system initialization so scheduled
    /// invocations can target flows in addition to functions.
    pub fn with_flow_starter(mut self, starter: FlowStartCallback) -> Self {
        self.flow_starter = Some(starter);
        self
    }

    /// Handle a due scheduled invocation job
    ///
    /// # Arguments
    ///
    /// * `job` - Job information containing the JobType::ScheduledInvocation variant
    /// * `context` - Persisted job context carrying the invocation payload
    pub async fn handle(
        &self,
        job: &JobInfo,
        context: &JobContext,
    ) -> Result<Option<serde_json::Value>> {
        let (invocation_id, target_kind) = match &job.job_type {
            JobType::ScheduledInvocation {
                invocation_id,
                target_kind,
            } => (invocation_id.clone(), target_kind.clone()),
            _ => {
                return Err(Error::Validation(
                    "Expected ScheduledInvocation job type".to_string(),
                ))
            }
        };

        let target_path = context
            .metadata
            .get(META_TARGET_PATH)
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                Error::Validation(
                    "Scheduled invocation is missing 'target_path' in its job context".to_string(),
                )
            })?
            .to_string();
        let input = context
            .metadata
            .get(META_INPUT)
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let actor = context
            .metadata
            .get(META_ACTOR)
            .and_then(|v| v.as_str())
            .unwrap_or("scheduler")
            .to_string();

        tracing::info!(
            job_id = %job.id,
            invocation_id = %invocation_id,
            target_kind = %target_kind,
            target_path = %target_path,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            "Processing due scheduled invocation"
        );

        match target_kind.as_str() {
            "function" => {
                self.invoke_function(&invocation_id, &target_path, input, context)
                    .await
            }
            "flow" => self.start_flow(&target_path, input, actor, context).await,
            other => Err(Error::Validation(format!(
                "Unknown scheduled invocation target kind '{}' (expected 'function' or 'flow')",
                other
            ))),
        }
    }

    /// Enqueue a `FunctionExecution` job for a due function invocation.
    async fn invoke_function(
        &self,
        invocation_id: &str,
        target_path: &str,
        input: serde_json::Value,
        context: &JobContext,
    ) -> Result<Option<serde_json::Value>> {
        let execution_id = nanoid::nanoid!();

        let function_job_type = JobType::FunctionExecution {
            function_path: target_path.to_string(),
            trigger_name: Some("scheduler".to_string()),
            execution_id: execution_id.clone(),
        };

        let mut metadata = HashMap::new();
        metadata.insert("input".to_string(), input);
        metadata.insert(
            "invocation_id".to_string(),
            serde_json::json!(invocation_id),
        );

        let function_context = JobContext {
            tenant_id: context.tenant_id.clone(),
            repo_id: context.repo_id.clone(),
            branch: context.branch.clone(),
            workspace_id: context.workspace_id.clone(),
            revision: raisin_hlc::HLC::new(0, 0),
            metadata,
        };

        // Store the context BEFORE registering so dispatch can never
        // observe the job without its context.
        let function_job_id = raisin_storage::jobs::JobId::new();
        self.job_data_store
            .put(&function_job_id, &function_context)?;

        self.job_registry
            .register_job_with_id(
                function_job_id.clone(),
                function_job_type.clone(),
                context.tenant_id.clone(),
                None,
                None,
                None,
            )
            .await?;

        // Dispatch to priority queue
        let priority = function_job_type.default_priority();
        self.dispatcher
            .dispatch(function_job_id.clone(), priority)
            .await;

        tracing::debug!(
            job_id = %function_job_id,
            execution_id = %execution_id,
            invocation_id = %invocation_id,
            function_path = %target_path,
            "Enqueued and dispatched scheduled function execution job"
        );

        Ok(Some(serde_json::json!({
            "target_kind": "function",
            "execution_id": execution_id,
            "job_id": function_job_id.to_string(),
            "status": "queued",
        })))
    }

    /// Start a flow instance for a due flow invocation.
    async fn start_flow(
        &self,
        target_path: &str,
        input: serde_json::Value,
        actor: String,
        context: &JobContext,
    ) -> Result<Option<serde_json::Value>> {
        let starter = self.flow_starter.as_ref().ok_or_else(|| {
            Error::Validation(
                "Flow starter not configured. The job system init must provide the flow start callback.".to_string(),
            )
        })?;

        let result = starter(
            context.tenant_id.clone(),
            context.repo_id.clone(),
            context.branch.clone(),
            target_path.to_string(),
            input,
            actor,
        )
        .await?;

        Ok(Some(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_storage::jobs::{JobId, JobStatus};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn test_setup() -> (
        Arc<JobRegistry>,
        Arc<JobDataStore>,
        Arc<JobDispatcher>,
        tempfile::TempDir,
    ) {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = Arc::new(crate::open_db(temp_dir.path()).unwrap());
        let data_store = Arc::new(JobDataStore::new(db));
        let registry = Arc::new(JobRegistry::new());
        let (dispatcher, _receivers) = crate::jobs::RocksDBWorkerPool::create_dispatcher();
        (registry, data_store, dispatcher, temp_dir)
    }

    fn invocation_context(metadata: HashMap<String, serde_json::Value>) -> JobContext {
        JobContext {
            tenant_id: "default".to_string(),
            repo_id: "test-repo".to_string(),
            branch: "main".to_string(),
            workspace_id: "functions".to_string(),
            revision: raisin_hlc::HLC::new(0, 0),
            metadata,
        }
    }

    fn invocation_job(target_kind: &str) -> JobInfo {
        JobInfo {
            id: JobId::new(),
            job_type: JobType::ScheduledInvocation {
                invocation_id: nanoid::nanoid!(),
                target_kind: target_kind.to_string(),
            },
            status: JobStatus::Running,
            tenant: "default".to_string(),
            started_at: chrono::Utc::now(),
            completed_at: None,
            progress: None,
            error: None,
            result: None,
            retry_count: 0,
            max_retries: 0,
            last_heartbeat: None,
            timeout_seconds: 300,
            next_retry_at: None,
            executing_since: None,
        }
    }

    #[tokio::test]
    async fn function_target_enqueues_function_execution_with_input() {
        let (registry, data_store, dispatcher, _tmp) = test_setup();
        let handler =
            ScheduledInvocationHandler::new(registry.clone(), data_store.clone(), dispatcher);

        let mut metadata = HashMap::new();
        metadata.insert(META_TARGET_PATH.to_string(), serde_json::json!("/lib/hello"));
        metadata.insert(META_INPUT.to_string(), serde_json::json!({"name": "world"}));
        metadata.insert(META_ACTOR.to_string(), serde_json::json!("tester"));
        let context = invocation_context(metadata);

        let job = invocation_job("function");
        let result = handler.handle(&job, &context).await.unwrap().unwrap();
        assert_eq!(result["target_kind"], "function");
        assert_eq!(result["status"], "queued");

        // The FunctionExecution job must be registered with the persisted input.
        let jobs = registry.list_jobs_by_tenant("default").await;
        let exec_job = jobs
            .iter()
            .find(|j| matches!(j.job_type, JobType::FunctionExecution { .. }))
            .expect("FunctionExecution job registered");
        match &exec_job.job_type {
            JobType::FunctionExecution {
                function_path,
                trigger_name,
                ..
            } => {
                assert_eq!(function_path, "/lib/hello");
                assert_eq!(trigger_name.as_deref(), Some("scheduler"));
            }
            _ => unreachable!(),
        }
        let exec_ctx = data_store
            .get("default", &exec_job.id)
            .unwrap()
            .expect("context stored before registration");
        assert_eq!(exec_ctx.repo_id, "test-repo");
        assert_eq!(
            exec_ctx.metadata.get("input"),
            Some(&serde_json::json!({"name": "world"}))
        );
    }

    #[tokio::test]
    async fn flow_target_calls_flow_starter() {
        let (registry, data_store, dispatcher, _tmp) = test_setup();

        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_cb = calls.clone();
        let starter: FlowStartCallback = Arc::new(
            move |_tenant, repo, _branch, flow_path, input, actor| {
                let calls = calls_for_cb.clone();
                Box::pin(async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    assert_eq!(repo, "test-repo");
                    assert_eq!(flow_path, "/flows/nightly");
                    assert_eq!(input, serde_json::json!({"n": 1}));
                    assert_eq!(actor, "tester");
                    Ok(serde_json::json!({
                        "instance_id": "inst-1",
                        "job_id": "job-1",
                        "status": "queued",
                    }))
                })
            },
        );

        let handler = ScheduledInvocationHandler::new(registry, data_store, dispatcher)
            .with_flow_starter(starter);

        let mut metadata = HashMap::new();
        metadata.insert(
            META_TARGET_PATH.to_string(),
            serde_json::json!("/flows/nightly"),
        );
        metadata.insert(META_INPUT.to_string(), serde_json::json!({"n": 1}));
        metadata.insert(META_ACTOR.to_string(), serde_json::json!("tester"));
        let context = invocation_context(metadata);

        let job = invocation_job("flow");
        let result = handler.handle(&job, &context).await.unwrap().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(result["instance_id"], "inst-1");
        assert_eq!(result["status"], "queued");
    }

    #[tokio::test]
    async fn unknown_target_kind_is_rejected() {
        let (registry, data_store, dispatcher, _tmp) = test_setup();
        let handler = ScheduledInvocationHandler::new(registry, data_store, dispatcher);

        let mut metadata = HashMap::new();
        metadata.insert(META_TARGET_PATH.to_string(), serde_json::json!("/lib/x"));
        let context = invocation_context(metadata);

        let job = invocation_job("cron");
        let err = handler.handle(&job, &context).await.unwrap_err();
        assert!(matches!(err, Error::Validation(_)));
    }

    #[tokio::test]
    async fn missing_target_path_is_rejected() {
        let (registry, data_store, dispatcher, _tmp) = test_setup();
        let handler = ScheduledInvocationHandler::new(registry, data_store, dispatcher);

        let context = invocation_context(HashMap::new());
        let job = invocation_job("function");
        let err = handler.handle(&job, &context).await.unwrap_err();
        assert!(matches!(err, Error::Validation(_)));
    }

    #[tokio::test]
    async fn cancel_before_fire_marks_job_cancelled() {
        let (registry, data_store, _dispatcher, _tmp) = test_setup();

        // Schedule an invocation one hour out; the delayed-dispatch monitor
        // will not fire it, so a cancel must land cleanly.
        let job_id = JobId::new();
        let mut metadata = HashMap::new();
        metadata.insert(META_TARGET_PATH.to_string(), serde_json::json!("/lib/x"));
        data_store
            .put(&job_id, &invocation_context(metadata))
            .unwrap();
        registry
            .register_job_at_with_id(
                job_id.clone(),
                JobType::ScheduledInvocation {
                    invocation_id: nanoid::nanoid!(),
                    target_kind: "function".to_string(),
                },
                "default".to_string(),
                chrono::Utc::now() + chrono::Duration::hours(1),
                Some(0),
            )
            .await
            .unwrap();

        let info = registry.get_job_info(&job_id).await.unwrap();
        assert!(matches!(info.status, JobStatus::Scheduled));
        assert!(info.next_retry_at.is_some(), "scheduled_at must persist");

        registry.cancel_job(&job_id).await.unwrap();
        let info = registry.get_job_info(&job_id).await.unwrap();
        assert!(matches!(info.status, JobStatus::Cancelled));
    }
}
