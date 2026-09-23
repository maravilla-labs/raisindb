// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The run service's parent/child half: spawn, hand-back, cascade, repair.
//!
//! No step here spans two aggregates in one commit. Each is an idempotent
//! commit on one run, entered from whatever node the job queue or the sweeper
//! picked — the existing per-run commit section and version check are the
//! only concurrency control:
//!
//! - spawn: parent commit (admission + link + plan), then the child's create
//!   under the id the link names (repaired from the plan if it is lost);
//! - hand-back: parent commit (mailbox, deduplicated by the link), then the
//!   child's `HandbackDelivered` (the sweeper's "owed" worklist until then);
//! - cascade: a stop per live child with a deterministic control id.

use serde::Serialize;
use serde_json::json;

use crate::child::SpawnChild;
use crate::child_admit::{apply_spawn, plan_key, ChildPlan};
use crate::child_apply::{apply_handback, apply_handback_delivered, build_handback};
use crate::control::{authorize, ActorRef, ControlAck, ControlCommand, ControlKind};
use crate::events::RunEventKind;
use crate::ids::{ControlId, PrincipalKind, RunId, RunScope, SystemToken};
use crate::lifecycle::Fence;
use crate::record::{AgentRunRecord, RunBudgets};
use crate::service::{new_record, AgentRunService, CreateRun, ServiceError};
use crate::state::{RunOutcome, RunState, RunStatus, TerminalStatus, WakeReason};
use crate::store::CreateOutcome;
use crate::waiter::{apply_waiter_notified, waiter_result};

/// The answer to a spawn.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SpawnOutcome {
    /// The child.
    pub child_run_id: RunId,
    /// Its number under the parent.
    pub child_no: u32,
    /// False when a spawn with the same `spawn_key` came back.
    pub created: bool,
    /// The child's effective budgets (clamped to what the parent could lend).
    pub budgets: RunBudgets,
    /// Resume key a waiting tool uses to wait for the child.
    pub resume_key: String,
}

/// What a lineage repair did.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct LineageReport {
    /// Hand-backs delivered.
    pub handbacks: usize,
    /// Children stopped because their parent ended.
    pub cascaded: usize,
    /// Children re-created from their stored plan.
    pub materialized: usize,
}

fn system_actor() -> ActorRef {
    ActorRef {
        kind: PrincipalKind::System,
        id: "agent-runtime".into(),
        capability: None,
    }
}

impl AgentRunService {
    /// Spawn a child of `parent`. `actor` must be authorized on the parent.
    pub async fn spawn_child(
        &self,
        scope: &RunScope,
        parent: &RunId,
        req: SpawnChild,
        actor: &ActorRef,
        system: Option<&SystemToken>,
    ) -> Result<SpawnOutcome, ServiceError> {
        let child_id = RunId::new_v4();
        let (res, _) = self
            .apply(scope, parent, Fence::None, |rec, now| {
                if !authorize(rec, actor, system) {
                    return Err(ServiceError::Unauthorized);
                }
                if let Some(key) = &req.spawn_key {
                    if let Some(l) = rec
                        .children
                        .iter()
                        .find(|l| l.spawn_key.as_ref() == Some(key))
                    {
                        return Ok((None, Err((l.run_id.clone(), l.child_no))));
                    }
                }
                let (t, plan) = apply_spawn(rec, &req, child_id.clone(), now)?;
                Ok((Some(t), Ok(plan)))
            })
            .await?;
        match res {
            Ok(plan) => {
                self.materialize(&plan, scope).await?;
                Ok(SpawnOutcome {
                    resume_key: crate::child::resume_key(&plan.run_id),
                    child_run_id: plan.run_id,
                    child_no: plan.delegation.child_no,
                    created: true,
                    budgets: plan.budgets,
                })
            }
            Err((run_id, child_no)) => {
                let plan = self.plan(scope, parent, child_no).await?;
                if self.store.load(scope, &run_id).await?.is_none() {
                    self.materialize(&plan, scope).await?;
                }
                Ok(SpawnOutcome {
                    resume_key: crate::child::resume_key(&run_id),
                    child_run_id: run_id,
                    child_no,
                    created: false,
                    budgets: plan.budgets,
                })
            }
        }
    }

    async fn plan(
        &self,
        scope: &RunScope,
        parent: &RunId,
        child_no: u32,
    ) -> Result<ChildPlan, ServiceError> {
        let bytes = self
            .store
            .read_result(scope, parent, &plan_key(child_no))
            .await?
            .ok_or_else(|| ServiceError::Invalid(format!("spawn plan {child_no} is missing")))?;
        serde_json::from_slice(&bytes)
            .map_err(|e| ServiceError::Invalid(format!("spawn plan: {e}")))
    }

    /// Create the child a plan describes (idempotent by its create key).
    async fn materialize(&self, plan: &ChildPlan, scope: &RunScope) -> Result<(), ServiceError> {
        let now = self.now();
        let create = CreateRun {
            scope: scope.clone(),
            subject: plan.subject.clone(),
            principal: plan.principal.clone(),
            control_capability: None,
            agent_ref: plan.agent_ref.clone(),
            create_key: Some(plan.create_key.clone()),
            budgets: plan.budgets.clone(),
            input: plan.input.clone(),
            reducer: plan.reducer.clone(),
            executor_config: plan.executor_config.clone(),
            waiter: None,
        };
        let mut rec = new_record(&create, plan.run_id.clone(), now);
        rec.parent_run_id = Some(plan.parent_run_id.clone());
        rec.root_run_id = Some(plan.root_run_id.clone());
        rec.depth = plan.depth;
        rec.delegation = Some(plan.delegation.clone());
        let first = RunEventKind::RunCreated {
            subject: create.subject.clone(),
            principal: create.principal.clone(),
            budgets: create.budgets.clone(),
            input: create.input.clone(),
        };
        match self
            .store
            .create(rec.clone(), first, Some(&plan.create_key))
            .await?
        {
            CreateOutcome::Created { run_id, seq } => {
                self.notify(&run_id, seq);
                self.waker.wake(scope, &run_id, WakeReason::Created);
                Ok(())
            }
            CreateOutcome::Existing { run_id, .. } if run_id == plan.run_id => Ok(()),
            CreateOutcome::Existing { run_id, .. } => {
                // The subject is busy with an unrelated run: the child can
                // never exist, so its link is failed honestly.
                let mut failed = rec;
                failed.state = RunState::Terminal {
                    terminal: TerminalStatus::Failed,
                    outcome: RunOutcome {
                        kind: "failed".into(),
                        code: Some("subject_busy".into()),
                        message: Some(format!("the subject already has live run {run_id}")),
                        detail: None,
                    },
                };
                self.land_handback(scope, &failed).await.map(|_| ())
            }
        }
    }

    async fn land_handback(
        &self,
        scope: &RunScope,
        child: &AgentRunRecord,
    ) -> Result<bool, ServiceError> {
        let Some(parent) = child.parent_run_id.clone() else {
            return Ok(false);
        };
        let Some(hb) = build_handback(child) else {
            return Ok(false);
        };
        match self
            .apply(scope, &parent, Fence::None, |p, now| {
                Ok((apply_handback(p, child, &hb, now), ()))
            })
            .await
        {
            Ok(_) | Err(ServiceError::NotFound) => Ok(true),
            Err(e) => Err(e),
        }
    }

    /// Deliver a terminal child's hand-back to its parent's mailbox, then mark
    /// it delivered on the child. Idempotent; `false` when nothing was owed.
    pub async fn deliver_handback(
        &self,
        scope: &RunScope,
        child: &RunId,
    ) -> Result<bool, ServiceError> {
        let Some(rec) = self.store.load(scope, child).await? else {
            return Ok(false);
        };
        let owed = rec.state.is_terminal()
            && rec
                .delegation
                .as_ref()
                .is_some_and(|d| !d.handback_delivered);
        if !owed {
            return Ok(false);
        }
        self.land_handback(scope, &rec).await?;
        self.apply(scope, child, Fence::None, |c, now| {
            Ok((apply_handback_delivered(c, now), ()))
        })
        .await?;
        Ok(true)
    }

    /// Hand a terminal run's result to what waits for it (a flow step), then
    /// mark it notified. Idempotent; `false` when nothing was owed or no sink
    /// is installed on this node (the sweeper retries).
    pub async fn deliver_waiter(
        &self,
        scope: &RunScope,
        run: &RunId,
    ) -> Result<bool, ServiceError> {
        let Some(rec) = self.store.load(scope, run).await? else {
            return Ok(false);
        };
        let (Some(waiter), Some(result)) = (rec.waiter.as_ref(), waiter_result(&rec)) else {
            return Ok(false);
        };
        if waiter.delivered {
            return Ok(false);
        }
        let Some(sink) = self.waiter_sink.get() else {
            return Ok(false);
        };
        sink.notify(&rec, waiter, result)
            .await
            .map_err(|e| ServiceError::Store(crate::store::StoreError::Backend(e)))?;
        self.apply(scope, run, Fence::None, |r, now| {
            Ok((apply_waiter_notified(r, now), ()))
        })
        .await?;
        Ok(true)
    }

    /// Stop every live child of a terminal run. Returns how many stops landed.
    pub async fn cascade_stop(
        &self,
        scope: &RunScope,
        parent: &AgentRunRecord,
    ) -> Result<usize, ServiceError> {
        let status = parent.state.status();
        if !status.is_terminal() {
            return Ok(0);
        }
        let mut n = 0;
        for link in parent.children.iter().filter(|l| l.is_live()) {
            if self
                .stop_orphan(scope, &link.run_id, &parent.run_id, status)
                .await?
            {
                n += 1;
            }
        }
        Ok(n)
    }

    async fn stop_orphan(
        &self,
        scope: &RunScope,
        child: &RunId,
        parent: &RunId,
        why: RunStatus,
    ) -> Result<bool, ServiceError> {
        let Some(rec) = self.store.load(scope, child).await? else {
            return Ok(false);
        };
        if rec.state.is_terminal() || matches!(rec.state, RunState::Cancelling { .. }) {
            return Ok(false);
        }
        let cmd = ControlCommand {
            control_id: ControlId(format!("cascade:{parent}")),
            kind: ControlKind::Stop {
                reason: Some(format!("parent_{}", why.as_str())),
            },
            issued_by: system_actor(),
            at_ms: self.now(),
        };
        let ack = self
            .submit_control(scope, child, cmd, Some(&SystemToken::in_process()))
            .await?;
        Ok(matches!(ack, ControlAck::Applied { .. }))
    }

    /// What a wake of a TERMINAL run owes its lineage: its hand-back, and the
    /// stop of its live children.
    pub async fn settle_lineage(&self, scope: &RunScope, run: &RunId) -> Result<(), ServiceError> {
        let Some(rec) = self.store.load(scope, run).await? else {
            return Ok(());
        };
        if !rec.state.is_terminal() {
            return Ok(());
        }
        self.deliver_handback(scope, run).await?;
        self.deliver_waiter(scope, run).await?;
        self.cascade_stop(scope, &rec).await?;
        Ok(())
    }

    /// The sweeper's lineage pass over one scope: owed hand-backs, live
    /// children of ended parents, and children whose create was lost.
    pub async fn repair_lineage(&self, scope: &RunScope) -> Result<LineageReport, ServiceError> {
        let mut report = LineageReport::default();
        for run in self.store.scan_handback_owed(scope, 256).await? {
            if self.deliver_handback(scope, &run).await? {
                report.handbacks += 1;
            }
            if self.deliver_waiter(scope, &run).await? {
                report.handbacks += 1;
            }
        }
        for status in [
            RunStatus::Queued,
            RunStatus::Running,
            RunStatus::Waiting,
            RunStatus::Paused,
        ] {
            for run in self.store.scan_status(scope, status, 10_000).await? {
                let Some(rec) = self.store.load(scope, &run).await? else {
                    continue;
                };
                if let Some(parent) = &rec.parent_run_id {
                    let ended = match self.store.load(scope, parent).await? {
                        Some(p) => p.state.status().is_terminal().then(|| p.state.status()),
                        None => Some(RunStatus::Failed),
                    };
                    if let Some(why) = ended {
                        if self.stop_orphan(scope, &run, parent, why).await? {
                            report.cascaded += 1;
                        }
                        continue;
                    }
                }
                for link in rec.children.iter().filter(|l| l.is_live()) {
                    if self.store.load(scope, &link.run_id).await?.is_none() {
                        let plan = self.plan(scope, &run, link.child_no).await?;
                        self.materialize(&plan, scope).await?;
                        report.materialized += 1;
                    }
                }
            }
        }
        Ok(report)
    }

    /// Wake reasons a commit owes the lineage (see `commit`).
    pub(crate) fn lineage_wakes(rec: &AgentRunRecord, ended: bool) -> Vec<WakeReason> {
        let mut out = Vec::new();
        if ended && rec.handback_owed() {
            out.push(WakeReason::Handback);
        }
        if ended && rec.children.iter().any(|l| l.is_live()) {
            out.push(WakeReason::Cascade);
        }
        out
    }

    /// Transport-neutral usage accounting of one run.
    pub fn usage_report(rec: &AgentRunRecord, now: u64) -> serde_json::Value {
        let tree = crate::budget::tree_used(rec);
        let spare = crate::budget::spare(rec, now);
        let live: Vec<_> = rec.children.iter().filter(|l| l.is_live()).collect();
        json!({
            "own": rec.usage,
            "children": {
                "input_tokens": rec.usage.child_input_tokens, "output_tokens": rec.usage.child_output_tokens,
                "operations": rec.usage.child_operations, "model_calls": rec.usage.child_model_calls,
                "tool_calls": rec.usage.child_tool_calls, "completed": rec.usage.children_completed,
                "spawned": rec.children.len(), "live": live.len(),
            },
            "reserved": {
                "operations": live.iter().filter_map(|l| l.reserved.max_operations).sum::<u64>(),
                "model_calls": live.iter().filter_map(|l| l.reserved.max_model_calls).sum::<u32>(),
                "tokens": live.iter().filter_map(|l| l.reserved.max_total_tokens).sum::<u64>(),
            },
            "total": rec.usage.tree_total(),
            "counted": tree,
            "budgets": rec.budgets,
            "spare": {
                "operations": spare.operations, "model_calls": spare.model_calls,
                "tokens": spare.tokens, "wall_ms": spare.wall_ms,
            },
        })
    }
}
