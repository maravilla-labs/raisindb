// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! `AgentRunStep`: one wake of one durable agent run.
//!
//! The job carries only the run id; the scope comes from its `JobContext`.
//! Whichever worker picks it up drives the run through the node's
//! [`AgentRunHost`](raisin_agent_runtime::host::AgentRunHost): acquire the
//! execution lease from the record (refused → another driver has it, done),
//! drive until the run leaves `Running`, finalize a terminal domain run.
//! Exclusivity is the lease and its fencing epoch, never this job.

use raisin_agent_runtime::driver::DriveOutcome;
use raisin_agent_runtime::ids::{RunId, RunScope};
use raisin_error::{Error, Result};
use raisin_storage::jobs::{JobContext, JobInfo, JobType};

use crate::agent_runs::agent_run_host;

/// Handler for `JobType::AgentRunStep`.
#[derive(Default)]
pub struct AgentRunStepHandler;

impl AgentRunStepHandler {
    /// A handler reading the process's installed host.
    pub fn new() -> Self {
        Self
    }

    /// Drive the run named by `job`.
    pub async fn handle(
        &self,
        job: &JobInfo,
        context: &JobContext,
    ) -> Result<Option<serde_json::Value>> {
        let JobType::AgentRunStep { run_id, reason } = &job.job_type else {
            return Err(Error::Validation("expected an AgentRunStep job".into()));
        };
        let host = agent_run_host().ok_or_else(|| {
            Error::storage("agent run step dispatched before the agent runtime was installed")
        })?;
        let scope = RunScope::new(&context.tenant_id, &context.repo_id, &context.branch);
        let run = RunId(run_id.clone());
        let outcome = host
            .step(&scope, &run, &job.id.to_string())
            .await
            .map_err(|e| Error::storage(format!("agent run {run_id}: {e}")))?;
        let label = match &outcome {
            DriveOutcome::NotAcquired => "not_acquired".to_string(),
            DriveOutcome::LeaseLost => "lease_lost".to_string(),
            DriveOutcome::Crashed => "crashed".to_string(),
            DriveOutcome::Exited(status) => format!("exited:{status:?}"),
        };
        tracing::debug!(run = %run_id, reason = %reason, outcome = %label, "agent run step");
        Ok(Some(
            serde_json::json!({ "run_id": run_id, "outcome": label }),
        ))
    }
}
