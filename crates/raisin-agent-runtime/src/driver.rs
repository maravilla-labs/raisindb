// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The run driver.
//!
//! 1. Acquire the lease (refused → return).
//! 2. Loop: reload. `Running{Idle}` is the ONLY safe boundary: drain steers,
//!    then ask the planner (or the domain reducer) for the next action; for a
//!    domain run the reducer response, its effect translation and the
//!    operation start are one commit.
//! 3. An operation runs as: idempotency check → register the token → re-check
//!    the state (a stop may have landed first) → execute under `select!` with
//!    the renew tick → `finish_operation`.
//!
//! Every executor commit is fenced by the lease; `LeaseLost` means stop
//! immediately without writing.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::domain::DomainReducer;
use crate::domain_step::DomainStep;
use crate::events::{CheckpointReason, OpOutcome};
use crate::ids::{Principal, RunId, RunScope};
use crate::lifecycle::{BeginRefusal, LeaseFence, NewRequest, OperationResult, OperationSpec};
use crate::record::{ActiveOperation, AgentRunRecord, SteerEntry};
use crate::service::{AgentRunService, ServiceError};
use crate::state::{Activity, RunOutcome, RunState, RunStatus, TerminalStatus};

/// What a planner wants next.
#[derive(Debug, Clone, PartialEq)]
pub enum NextAction {
    /// Start an operation.
    Operation(OperationSpec),
    /// Open requests and wait.
    Wait(Vec<NewRequest>),
    /// End the run.
    Terminal {
        /// How.
        status: TerminalStatus,
        /// With what.
        outcome: RunOutcome,
    },
    /// Write a checkpoint.
    Checkpoint,
    /// Nothing to do: release the lease without a wake.
    Nothing,
}

/// Decides the next action of a run without a domain reducer.
#[async_trait]
pub trait StepPlanner: Send + Sync {
    /// Next action; `consumed` are the steers consumed at this boundary.
    async fn next(&self, record: &AgentRunRecord, consumed: &[SteerEntry]) -> NextAction;
}

/// What an executor gets.
#[derive(Debug, Clone)]
pub struct ExecContext {
    /// Scope.
    pub scope: RunScope,
    /// Run.
    pub run_id: RunId,
    /// The run's principal: every operation executes under it.
    pub principal: Principal,
    /// The operation.
    pub op: ActiveOperation,
    /// Cancelled when a stop lands.
    pub token: CancellationToken,
    /// The run's opaque agent reference.
    pub agent_ref: Option<String>,
    /// What the run is about.
    pub subject: Option<crate::ids::SubjectRef>,
    /// The run's opaque executor configuration.
    pub executor_config: Option<serde_json::Value>,
}

/// Executes operations.
#[async_trait]
pub trait OperationExecutor: Send + Sync {
    /// Run one operation to its result.
    async fn execute(&self, ctx: ExecContext) -> OperationResult;
}

/// What a hook tells the driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookAction {
    /// Carry on.
    Continue,
    /// Simulate a crash: return without writing anything else.
    Crash,
}

/// Fail points (tests).
#[async_trait]
pub trait DriverHooks: Send + Sync {
    /// After `OperationStarted` is committed, before the token is registered.
    async fn after_begin_commit(&self, _op: &ActiveOperation) -> HookAction {
        HookAction::Continue
    }
    /// After the executor returned, before `finish_operation`.
    async fn after_execute(&self, _op: &ActiveOperation, _result: &OperationResult) -> HookAction {
        HookAction::Continue
    }
}

/// No hooks.
pub struct NoHooks;

impl DriverHooks for NoHooks {}

/// Planner or domain reducer.
#[derive(Clone)]
pub enum Mode {
    /// A generic planner.
    Planner(Arc<dyn StepPlanner>),
    /// A domain reducer.
    Domain(Arc<dyn DomainReducer>),
}

/// How a drive ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriveOutcome {
    /// The lease could not be acquired.
    NotAcquired,
    /// The lease was lost; nothing more was written.
    LeaseLost,
    /// A hook simulated a crash.
    Crashed,
    /// The run left `Running` (waiting, paused, queued, terminal).
    Exited(RunStatus),
}

enum Exec {
    Done,
    Lost,
    Crashed,
}

/// The driver.
pub struct RunDriver {
    service: Arc<AgentRunService>,
    mode: Mode,
    executor: Arc<dyn OperationExecutor>,
    hooks: Arc<dyn DriverHooks>,
}

impl RunDriver {
    /// A driver.
    pub fn new(
        service: Arc<AgentRunService>,
        mode: Mode,
        executor: Arc<dyn OperationExecutor>,
    ) -> Self {
        Self {
            service,
            mode,
            executor,
            hooks: Arc::new(NoHooks),
        }
    }

    /// With fail points.
    pub fn with_hooks(mut self, hooks: Arc<dyn DriverHooks>) -> Self {
        self.hooks = hooks;
        self
    }

    /// Acquire and drive.
    pub async fn drive(
        &self,
        scope: &RunScope,
        run: &RunId,
        owner: &str,
    ) -> Result<DriveOutcome, ServiceError> {
        let grant = match self.service.acquire_lease(scope, run, owner).await {
            Ok(g) => g,
            Err(ServiceError::Refused(_)) => {
                self.finalize_if_terminal(scope, run).await?;
                return Ok(DriveOutcome::NotAcquired);
            }
            Err(e) => return Err(e),
        };
        self.drive_with(scope, run, grant.fence).await
    }

    /// Recover a scope and drive every run whose in-flight operation must be
    /// re-dispatched; finalize terminal domain runs.
    pub async fn recover_and_drive(
        &self,
        scope: &RunScope,
        owner: &str,
    ) -> Result<Vec<(RunId, DriveOutcome)>, ServiceError> {
        let mut out = Vec::new();
        for r in self.service.recover(scope, owner).await? {
            match r {
                crate::service_exec::Recovered::Redispatch { run_id, fence } => {
                    let outcome = self.drive_with(scope, &run_id, fence).await?;
                    out.push((run_id, outcome));
                }
                crate::service_exec::Recovered::TakenOver { run_id, status }
                    if status.is_terminal() =>
                {
                    self.finalize_if_terminal(scope, &run_id).await?;
                    out.push((run_id, DriveOutcome::Exited(status)));
                }
                _ => {}
            }
        }
        Ok(out)
    }

    async fn finalize_if_terminal(
        &self,
        scope: &RunScope,
        run: &RunId,
    ) -> Result<(), ServiceError> {
        if let Mode::Domain(reducer) = &self.mode {
            self.service
                .finalize_domain(scope, run, reducer.as_ref())
                .await?;
        }
        Ok(())
    }

    /// Drive under an already-held lease.
    pub async fn drive_with(
        &self,
        scope: &RunScope,
        run: &RunId,
        fence: LeaseFence,
    ) -> Result<DriveOutcome, ServiceError> {
        match self.drive_loop(scope, run, &fence).await {
            Err(e) if e.is_lease_lost() => Ok(DriveOutcome::LeaseLost),
            other => other,
        }
    }

    async fn drive_loop(
        &self,
        scope: &RunScope,
        run: &RunId,
        fence: &LeaseFence,
    ) -> Result<DriveOutcome, ServiceError> {
        loop {
            let rec = self
                .service
                .get(scope, run)
                .await?
                .ok_or(ServiceError::NotFound)?;
            let mine = rec
                .state
                .lease()
                .is_some_and(|l| l.owner == fence.owner && l.epoch == fence.epoch);
            match &rec.state {
                RunState::Running {
                    activity: Activity::Operating { op, .. },
                    ..
                } if mine => match self.execute(scope, &rec, fence, op.clone()).await? {
                    Exec::Done => {}
                    Exec::Lost => return Ok(DriveOutcome::LeaseLost),
                    Exec::Crashed => return Ok(DriveOutcome::Crashed),
                },
                RunState::Running {
                    activity: Activity::Idle { .. },
                    ..
                } if mine => {
                    if let Some(outcome) = self.boundary(scope, run, fence, &rec).await? {
                        return Ok(outcome);
                    }
                }
                RunState::Cancelling { op, .. } if mine => {
                    // Stopped before it ever ran.
                    let result = OperationResult {
                        outcome: Some(OpOutcome::Cancelled),
                        ..OperationResult::default()
                    };
                    self.service
                        .finish_operation(scope, run, fence, &op.op_id, result)
                        .await?;
                }
                RunState::Terminal { .. } => {
                    self.finalize_if_terminal(scope, run).await?;
                    return Ok(DriveOutcome::Exited(rec.state.status()));
                }
                RunState::Running { .. } | RunState::Cancelling { .. } => {
                    return Ok(DriveOutcome::LeaseLost)
                }
                other => return Ok(DriveOutcome::Exited(other.status())),
            }
        }
    }

    /// One step at the idle boundary. `Some` ends the drive.
    async fn boundary(
        &self,
        scope: &RunScope,
        run: &RunId,
        fence: &LeaseFence,
        rec: &AgentRunRecord,
    ) -> Result<Option<DriveOutcome>, ServiceError> {
        let consumed = self.service.consume_steers(scope, run, fence).await?;
        match &self.mode {
            Mode::Domain(reducer) => match self
                .service
                .domain_step(scope, run, fence, reducer.as_ref())
                .await?
            {
                DomainStep::Continue | DomainStep::Left | DomainStep::Operation(_) => Ok(None),
                DomainStep::Stalled => {
                    // A reducer that neither acts, waits nor finishes cannot
                    // progress; fail honestly rather than idle forever.
                    let outcome = RunOutcome {
                        kind: "failed".into(),
                        code: Some("reducer_stalled".into()),
                        ..RunOutcome::default()
                    };
                    self.service
                        .complete(
                            scope,
                            run,
                            fence,
                            TerminalStatus::Failed,
                            outcome,
                            Some("reducer_stalled".into()),
                        )
                        .await?;
                    Ok(None)
                }
            },
            Mode::Planner(planner) => {
                let rec = if consumed.is_empty() {
                    rec.clone()
                } else {
                    self.service
                        .get(scope, run)
                        .await?
                        .ok_or(ServiceError::NotFound)?
                };
                match planner.next(&rec, &consumed).await {
                    NextAction::Operation(spec) => {
                        match self.service.begin_operation(scope, run, fence, spec).await {
                            Ok(_) | Err(ServiceError::Begin(BeginRefusal::Budget(_))) => Ok(None),
                            Err(e) => Err(e),
                        }
                    }
                    NextAction::Wait(requests) => self
                        .service
                        .wait(scope, run, fence, requests)
                        .await
                        .map(|_| None),
                    NextAction::Terminal { status, outcome } => self
                        .service
                        .complete(scope, run, fence, status, outcome, None)
                        .await
                        .map(|_| None),
                    NextAction::Checkpoint => self
                        .service
                        .checkpoint(scope, run, fence, CheckpointReason::Periodic, None)
                        .await
                        .map(|_| None),
                    NextAction::Nothing => {
                        let rec = self.service.release_lease(scope, run, fence, false).await?;
                        Ok(Some(DriveOutcome::Exited(rec.state.status())))
                    }
                }
            }
        }
    }

    async fn execute(
        &self,
        scope: &RunScope,
        rec: &AgentRunRecord,
        fence: &LeaseFence,
        op: ActiveOperation,
    ) -> Result<Exec, ServiceError> {
        let run = &rec.run_id;
        let svc = &self.service;
        if svc
            .store()
            .idem_seen(scope, run, &op.idempotency_key)
            .await?
            .is_some()
        {
            let result = OperationResult {
                payload: Some(json!({ "deduplicated": true })),
                ..OperationResult::default()
            };
            svc.finish_operation(scope, run, fence, &op.op_id, result)
                .await?;
            return Ok(Exec::Done);
        }
        if self.hooks.after_begin_commit(&op).await == HookAction::Crash {
            return Ok(Exec::Crashed);
        }
        let token = svc.cancels().register(run, &op.op_id);
        let now = svc.get(scope, run).await?.ok_or(ServiceError::NotFound)?;
        let still_mine = matches!(&now.state, RunState::Running { activity: Activity::Operating { op: o, .. }, .. } if o.op_id == op.op_id);
        if !still_mine || token.is_cancelled() {
            // A stop (or a lost lease) landed before the operation ran. Nothing
            // executes — not even a non-interruptible operation, which only
            // promises to FINISH once started. A cancelling run records an
            // acknowledged cancel; anything else answers LeaseLost.
            token.cancel();
            let result = OperationResult {
                outcome: Some(OpOutcome::Cancelled),
                ..OperationResult::default()
            };
            return match svc
                .finish_operation(scope, run, fence, &op.op_id, result)
                .await
            {
                Ok(_) => Ok(Exec::Done),
                Err(e) if e.is_lease_lost() => Ok(Exec::Lost),
                Err(ServiceError::Refused(_)) => Ok(Exec::Lost),
                Err(e) => Err(e),
            };
        }
        let ctx = ExecContext {
            scope: scope.clone(),
            run_id: run.clone(),
            principal: rec.principal.clone(),
            op: op.clone(),
            token: token.clone(),
            agent_ref: rec.agent_ref.clone(),
            subject: Some(rec.subject.clone()),
            executor_config: rec.executor_config.clone(),
        };
        let fut = self.executor.execute(ctx);
        tokio::pin!(fut);
        let period = Duration::from_millis(svc.config.renew_every_ms.max(1));
        let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        let result = loop {
            tokio::select! {
                biased;
                r = &mut fut => break r,
                _ = token.cancelled(), if op.interruptible => {
                    break OperationResult { outcome: Some(OpOutcome::Cancelled), ..OperationResult::default() };
                }
                _ = tick.tick() => match svc.renew_lease(scope, run, fence).await {
                    Ok(r) => if matches!(r.state, RunState::Cancelling { .. }) { token.cancel(); },
                    Err(e) if e.is_lease_lost() => return Ok(Exec::Lost),
                    Err(e) => return Err(e),
                },
            }
        };
        if self.hooks.after_execute(&op, &result).await == HookAction::Crash {
            return Ok(Exec::Crashed);
        }
        match svc
            .finish_operation(scope, run, fence, &op.op_id, result)
            .await
        {
            Ok(_) => Ok(Exec::Done),
            Err(e) if e.is_lease_lost() => Ok(Exec::Lost),
            Err(e) => Err(e),
        }
    }
}
