// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The production `RunWaker`: a wake is an `AgentRunStep` job on the ordinary
//! job queue.
//!
//! Nothing about a step is tied to the node that woke it: the job carries only
//! `(tenant, repo, branch, run)`, and whichever worker runs it acquires the
//! run's execution lease from the record like any other driver. Duplicates are
//! harmless — a second step finds the lease held and returns — so the dedup
//! key only damps bursts (one job per run and reason per second). A wake that
//! is lost anyway (a crash between commit and enqueue) is re-issued by the
//! sweeper, which wakes every `Queued` run.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_agent_runtime::ids::{RunId, RunScope};
use raisin_agent_runtime::state::WakeReason;
use raisin_agent_runtime::wake::RunWaker;
use raisin_storage::jobs::{JobContext, JobId, JobRegistry, JobType};

use crate::jobs::JobDataStore;

/// Workspace recorded on step jobs (runs are not workspace-scoped).
pub const AGENT_RUN_JOB_WORKSPACE: &str = "raisin:system";

/// Enqueues `AgentRunStep` jobs.
pub struct JobQueueWaker {
    registry: Arc<JobRegistry>,
    data_store: Arc<JobDataStore>,
}

impl JobQueueWaker {
    /// A waker over the node's job queue.
    pub fn new(registry: Arc<JobRegistry>, data_store: Arc<JobDataStore>) -> Self {
        Self {
            registry,
            data_store,
        }
    }

    /// Enqueue one step (context first, so a worker never sees a job without
    /// its context).
    pub async fn enqueue(
        registry: &JobRegistry,
        data_store: &JobDataStore,
        scope: &RunScope,
        run: &RunId,
        reason: &str,
    ) -> raisin_error::Result<bool> {
        let job_id = JobId::new();
        let context = JobContext {
            tenant_id: scope.tenant_id.clone(),
            repo_id: scope.repo_id.clone(),
            branch: scope.branch.clone(),
            workspace_id: AGENT_RUN_JOB_WORKSPACE.to_string(),
            revision: raisin_hlc::HLC::new(0, 0),
            metadata: HashMap::new(),
        };
        data_store.put(&job_id, &context)?;
        let second = chrono::Utc::now().timestamp();
        let dedup = format!(
            "agentrun:{}/{}/{}:{reason}:{second}",
            scope.tenant_id, scope.repo_id, run
        );
        let job_type = JobType::AgentRunStep {
            run_id: run.to_string(),
            reason: reason.to_string(),
        };
        match registry
            .register_job_with_id_idempotent(
                job_id.clone(),
                job_type,
                scope.tenant_id.clone(),
                dedup,
                Some(0),
            )
            .await
        {
            Ok(true) => Ok(true),
            Ok(false) => {
                let _ = data_store.delete(&scope.tenant_id, &job_id);
                Ok(false)
            }
            Err(e) => {
                let _ = data_store.delete(&scope.tenant_id, &job_id);
                Err(e)
            }
        }
    }
}

impl RunWaker for JobQueueWaker {
    fn wake(&self, scope: &RunScope, run: &RunId, reason: WakeReason) {
        let registry = self.registry.clone();
        let data_store = self.data_store.clone();
        let scope = scope.clone();
        let run = run.clone();
        let reason = format!("{reason:?}");
        // The trait is synchronous (it is called right after a commit); the
        // enqueue is not, so it runs detached. A failure is only a delay: the
        // sweeper wakes every Queued run again.
        tokio::spawn(async move {
            if let Err(e) = Self::enqueue(&registry, &data_store, &scope, &run, &reason).await {
                tracing::warn!(run = %run, error = %e, "agent run wake could not be enqueued");
            }
        });
    }
}
