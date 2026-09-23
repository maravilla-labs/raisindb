// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The production `WaiterSink`: a flow step waiting for an agent run is
//! resumed by an ordinary `FlowInstanceExecution` resume job — the same job a
//! finished function step sends — carrying the run's result as its resume
//! data. Any node's worker picks it up; the flow instance lock serializes it
//! with everything else that advances the instance.
//!
//! The enqueue is idempotent per run (the dedup key), so a crash between the
//! enqueue and the run's `WaiterNotified` commit never resumes a flow twice.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use raisin_agent_runtime::record::AgentRunRecord;
use raisin_agent_runtime::waiter::{RunWaiter, WaiterSink, FLOW_INSTANCE};
use raisin_storage::jobs::{JobContext, JobId, JobRegistry, JobType};
use serde_json::Value;

use crate::jobs::JobDataStore;

/// Resume reason a flow sees for an agent run's result.
pub const AGENT_RUN_RESUME_REASON: &str = "agent_run";

/// Enqueues flow resume jobs for finished runs.
pub struct FlowResumeSink {
    registry: Arc<JobRegistry>,
    data_store: Arc<JobDataStore>,
}

impl FlowResumeSink {
    /// A sink over the node's job queue.
    pub fn new(registry: Arc<JobRegistry>, data_store: Arc<JobDataStore>) -> Self {
        Self {
            registry,
            data_store,
        }
    }
}

#[async_trait]
impl WaiterSink for FlowResumeSink {
    async fn notify(
        &self,
        run: &AgentRunRecord,
        waiter: &RunWaiter,
        result: Value,
    ) -> Result<(), String> {
        if waiter.kind != FLOW_INSTANCE {
            return Err(format!("unknown waiter kind '{}'", waiter.kind));
        }
        let scope = &run.scope;
        let mut metadata = HashMap::new();
        metadata.insert("function_result".to_string(), result);
        let context = JobContext {
            tenant_id: scope.tenant_id.clone(),
            repo_id: scope.repo_id.clone(),
            branch: waiter.branch.clone(),
            workspace_id: "raisin:system".to_string(),
            revision: raisin_hlc::HLC::new(0, 0),
            metadata,
        };
        let job_id = JobId::new();
        self.data_store
            .put(&job_id, &context)
            .map_err(|e| e.to_string())?;
        let job = JobType::FlowInstanceExecution {
            instance_id: waiter.target.clone(),
            execution_type: "resume".to_string(),
            resume_reason: Some(AGENT_RUN_RESUME_REASON.to_string()),
        };
        let dedup = format!(
            "agentrun-waiter:{}/{}/{}",
            scope.tenant_id, scope.repo_id, run.run_id
        );
        match self
            .registry
            .register_job_with_id_idempotent(
                job_id.clone(),
                job,
                scope.tenant_id.clone(),
                dedup,
                None,
            )
            .await
        {
            Ok(true) => Ok(()),
            // Already enqueued by an earlier delivery: done.
            Ok(false) => {
                let _ = self.data_store.delete(&scope.tenant_id, &job_id);
                Ok(())
            }
            Err(e) => {
                let _ = self.data_store.delete(&scope.tenant_id, &job_id);
                Err(e.to_string())
            }
        }
    }
}
