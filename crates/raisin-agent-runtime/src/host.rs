// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! `AgentRunHost`: what a server node runs on behalf of the job queue.
//!
//! The host is the one place that decides HOW a run is driven:
//!
//! - a run bound to a **domain reducer** is driven by this node — the reducer
//!   is an ordinary function (any runtime) resolved through a
//!   [`ReducerResolver`], the operations run through the [`OperationExecutor`];
//! - a run with **no reducer** is driven from outside (an external agent or
//!   client holding the lease over the API). The host never acquires such a
//!   run on a wake; it only repairs it in [`AgentRunHost::sweep`].
//!
//! A step is always entered from a job (`AgentRunStep`) or the sweeper, on
//! whichever node picked it up. Nothing here depends on which node that is:
//! the lease and its fencing epoch live in the record, and every commit is
//! checked against them.

use std::sync::Arc;

use async_trait::async_trait;

use crate::domain::{DomainReducer, ReducerCallError, ReducerRef};
use crate::driver::{DriveOutcome, Mode, NextAction, OperationExecutor, RunDriver, StepPlanner};
use crate::ids::{RunId, RunScope};
use crate::record::{AgentRunRecord, SteerEntry};
use crate::service::{AgentRunService, ServiceError};
use crate::service_exec::Recovered;

/// Resolves the reducer a run is bound to.
#[async_trait]
pub trait ReducerResolver: Send + Sync {
    /// Pin the function at `function_path` as it is NOW (for a create). The
    /// returned ref carries the artifact hash every later call is checked
    /// against.
    async fn bind(
        &self,
        scope: &RunScope,
        function_path: &str,
        handler: &str,
    ) -> Result<ReducerRef, ReducerCallError>;

    /// The reducer behind a bound ref, in `scope`.
    fn reducer(&self, scope: &RunScope, reducer: &ReducerRef) -> Arc<dyn DomainReducer>;
}

/// The planner of a run with no reducer: nothing to decide on the server. A
/// re-dispatched operation of such a run finishes, then the lease is released
/// so its external driver can pick the run up again.
pub struct ExternalPlanner;

#[async_trait]
impl StepPlanner for ExternalPlanner {
    async fn next(&self, _record: &AgentRunRecord, _consumed: &[SteerEntry]) -> NextAction {
        NextAction::Nothing
    }
}

/// What one sweep of a scope did.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SweepReport {
    /// Runs recovery touched.
    pub recovered: usize,
    /// Re-dispatched operations driven by this sweep.
    pub redispatched: usize,
    /// Terminal domain runs finalized.
    pub finalized: usize,
    /// Runs whose repair failed, with why (retried next sweep).
    pub errors: Vec<(RunId, String)>,
    /// Child hand-backs delivered.
    pub handbacks: usize,
    /// Children stopped because their parent ended.
    pub cascaded: usize,
    /// Children re-created from their stored spawn plan.
    pub materialized: usize,
}

/// Drives runs for the job queue and the sweeper.
pub struct AgentRunHost {
    service: Arc<AgentRunService>,
    reducers: Arc<dyn ReducerResolver>,
    executor: Arc<dyn OperationExecutor>,
    owner: String,
}

impl AgentRunHost {
    /// A host whose leases are owned by `owner` (e.g. `"{node_id}:{job_id}"`
    /// is appended per step, so two steps on one node never share a lease).
    pub fn new(
        service: Arc<AgentRunService>,
        reducers: Arc<dyn ReducerResolver>,
        executor: Arc<dyn OperationExecutor>,
        owner: impl Into<String>,
    ) -> Self {
        Self {
            service,
            reducers,
            executor,
            owner: owner.into(),
        }
    }

    /// The service.
    pub fn service(&self) -> &Arc<AgentRunService> {
        &self.service
    }

    /// The reducer resolver (create binds through it).
    pub fn reducers(&self) -> &Arc<dyn ReducerResolver> {
        &self.reducers
    }

    fn mode(&self, scope: &RunScope, rec: &AgentRunRecord) -> Mode {
        match &rec.domain {
            Some(d) => Mode::Domain(self.reducers.reducer(scope, &d.reducer)),
            None => Mode::Planner(Arc::new(ExternalPlanner)),
        }
    }

    fn driver(&self, mode: Mode) -> RunDriver {
        RunDriver::new(self.service.clone(), mode, self.executor.clone())
    }

    /// One wake: drive a domain run until it leaves `Running`, or finalize a
    /// terminal one. A run without a reducer is left to its external driver.
    pub async fn step(
        &self,
        scope: &RunScope,
        run: &RunId,
        step_id: &str,
    ) -> Result<DriveOutcome, ServiceError> {
        let rec = self
            .service
            .get(scope, run)
            .await?
            .ok_or(ServiceError::NotFound)?;
        if rec.state.is_terminal() {
            // Whatever the run's driver, its lineage is settled here.
            self.service.settle_lineage(&rec.scope, run).await?;
        }
        if rec.domain.is_none() {
            return Ok(DriveOutcome::Exited(rec.state.status()));
        }
        let owner = format!("{}:{}", self.owner, step_id);
        self.driver(self.mode(scope, &rec))
            .drive(scope, run, &owner)
            .await
    }

    /// Repair one scope: expired leases (taken over; a replay-safe in-flight
    /// operation is re-dispatched and driven HERE), `Queued` runs without a
    /// driver (woken again), expired requests (closed), and terminal domain
    /// runs whose finalize is still owed.
    pub async fn sweep(&self, scope: &RunScope) -> Result<SweepReport, ServiceError> {
        let owner = format!("{}:sweep", self.owner);
        let mut report = SweepReport::default();
        for r in self.service.recover(scope, &owner).await? {
            report.recovered += 1;
            let Recovered::Redispatch { run_id, fence } = r else {
                continue;
            };
            let Some(rec) = self.service.get(scope, &run_id).await? else {
                continue;
            };
            // Drive in the run's OWN scope (its branch lives in the record).
            let own = rec.scope.clone();
            match self
                .driver(self.mode(&own, &rec))
                .drive_with(&own, &run_id, fence)
                .await
            {
                Ok(_) => report.redispatched += 1,
                Err(e) => report.errors.push((run_id, format!("redispatch: {e}"))),
            }
        }
        match self.service.repair_lineage(scope).await {
            Ok(l) => {
                report.handbacks = l.handbacks;
                report.cascaded = l.cascaded;
                report.materialized = l.materialized;
            }
            Err(e) => report
                .errors
                .push((RunId("lineage".into()), format!("lineage: {e}"))),
        }
        for run in self.service.store().scan_unfinalized(scope, 256).await? {
            let Some(rec) = self.service.get(scope, &run).await? else {
                continue;
            };
            let Some(binding) = rec.domain.as_ref() else {
                continue;
            };
            let reducer = self.reducers.reducer(&rec.scope, &binding.reducer);
            match self
                .service
                .finalize_domain(&rec.scope, &run, reducer.as_ref())
                .await
            {
                Ok(_) => report.finalized += 1,
                Err(e) => report.errors.push((run, format!("finalize: {e}"))),
            }
        }
        Ok(report)
    }
}
