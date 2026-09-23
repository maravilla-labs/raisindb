// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The executor-facing half of the service: leases, operations, recovery.
//!
//! Every executor commit carries `Fence::Lease`. A `LeaseLost` answer means the
//! caller must stop immediately without writing anything else.

use serde_json::{json, Value};

use crate::control::ControlAck;
use crate::events::{CheckpointReason, ResultRef, RunEventKind};
use crate::ids::{ControlId, SystemToken};
use crate::ids::{OperationId, RunId, RunScope};
use crate::lifecycle::{
    self, apply_budget_exceeded, BeginRefusal, Fence, LeaseFence, NewRequest, OperationResult,
    OperationSpec,
};
use crate::record::PendingKind;
use crate::record::{ActiveOperation, AgentRunRecord, SteerEntry};
use crate::service::{AgentRunService, ServiceError};
use crate::state::{RunOutcome, RunState, RunStatus, TerminalStatus, WakeReason};
use crate::store::StoreError;
use crate::tx::Tx;

/// A granted lease.
#[derive(Debug, Clone)]
pub struct LeaseGrant {
    /// The fence to present on every commit.
    pub fence: LeaseFence,
    /// The record after acquisition.
    pub record: AgentRunRecord,
}

/// A started operation.
#[derive(Debug, Clone)]
pub struct OperationTicket {
    /// The operation.
    pub op: ActiveOperation,
    /// The record after the start.
    pub record: AgentRunRecord,
}

/// What recovery did to one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recovered {
    /// Took over a replay-safe in-flight operation; the new owner must
    /// re-dispatch it under `fence`.
    Redispatch {
        /// Run.
        run_id: RunId,
        /// The new owner's fence.
        fence: LeaseFence,
    },
    /// Took over an expired lease and moved the run to `status`.
    TakenOver {
        /// Run.
        run_id: RunId,
        /// New status.
        status: RunStatus,
    },
    /// Closed expired requests.
    RequestsExpired {
        /// Run.
        run_id: RunId,
    },
    /// Woke a queued run (idempotent backstop for a lost wake).
    Woken {
        /// Run.
        run_id: RunId,
    },
}

impl AgentRunService {
    /// Take the lease of a queued run.
    pub async fn acquire_lease(
        &self,
        scope: &RunScope,
        run: &RunId,
        owner: &str,
    ) -> Result<LeaseGrant, ServiceError> {
        let ttl = self.config.lease_ttl_ms;
        let (fence, record) = self
            .apply(scope, run, Fence::None, |rec, now| {
                let t = lifecycle::apply_acquire(rec, owner, now, ttl)?;
                let fence = LeaseFence {
                    owner: owner.into(),
                    epoch: t.record.lease_epoch,
                };
                Ok((Some(t), fence))
            })
            .await?;
        Ok(LeaseGrant { fence, record })
    }

    /// Extend the lease.
    pub async fn renew_lease(
        &self,
        scope: &RunScope,
        run: &RunId,
        fence: &LeaseFence,
    ) -> Result<AgentRunRecord, ServiceError> {
        let ttl = self.config.lease_ttl_ms;
        let ((), rec) = self
            .apply(scope, run, Fence::Lease(fence.clone()), |rec, now| {
                Ok((Some(lifecycle::apply_renew(rec, now, ttl)?), ()))
            })
            .await?;
        Ok(rec)
    }

    /// Start an operation. A budget refusal is COMMITTED (pause or fail per
    /// policy) before it is returned.
    pub async fn begin_operation(
        &self,
        scope: &RunScope,
        run: &RunId,
        fence: &LeaseFence,
        spec: OperationSpec,
    ) -> Result<OperationTicket, ServiceError> {
        let result =
            self.apply(scope, run, Fence::Lease(fence.clone()), |rec, now| {
                match lifecycle::apply_begin(rec, spec.clone(), now) {
                    Ok(t) => Ok((Some(t), None)),
                    Err(BeginRefusal::Budget(e)) => Ok((
                        Some(apply_budget_exceeded(rec, &e, now)),
                        Some(BeginRefusal::Budget(e)),
                    )),
                    Err(other) => Err(ServiceError::Begin(other)),
                }
            })
            .await?;
        match result {
            (Some(refusal), _) => Err(ServiceError::Begin(refusal)),
            (None, record) => {
                let op = record
                    .state
                    .active_op()
                    .cloned()
                    .ok_or(ServiceError::NotFound)?;
                Ok(OperationTicket { op, record })
            }
        }
    }

    /// Record an operation's result.
    pub async fn finish_operation(
        &self,
        scope: &RunScope,
        run: &RunId,
        fence: &LeaseFence,
        op_id: &OperationId,
        result: OperationResult,
    ) -> Result<AgentRunRecord, ServiceError> {
        let ((), rec) = self
            .apply(scope, run, Fence::Lease(fence.clone()), |rec, now| {
                Ok((
                    Some(lifecycle::apply_finish(rec, op_id, result.clone(), now)?),
                    (),
                ))
            })
            .await?;
        self.cancels.clear(run, op_id);
        Ok(rec)
    }

    /// Release the lease at a boundary. `wake` schedules another driver.
    pub async fn release_lease(
        &self,
        scope: &RunScope,
        run: &RunId,
        fence: &LeaseFence,
        wake: bool,
    ) -> Result<AgentRunRecord, ServiceError> {
        let ((), rec) = self
            .apply(scope, run, Fence::Lease(fence.clone()), |rec, now| {
                Ok((Some(lifecycle::apply_release(rec, now, wake)?), ()))
            })
            .await?;
        Ok(rec)
    }

    /// Consume queued steers at the idle boundary; returns them.
    pub async fn consume_steers(
        &self,
        scope: &RunScope,
        run: &RunId,
        fence: &LeaseFence,
    ) -> Result<Vec<SteerEntry>, ServiceError> {
        let (consumed, _) = self
            .apply(scope, run, Fence::Lease(fence.clone()), |rec, now| {
                let steers = rec.steer_queue.clone();
                Ok(match lifecycle::apply_consume_steers(rec, now) {
                    Some(t) => (Some(t), steers),
                    None => (None, Vec::new()),
                })
            })
            .await?;
        Ok(consumed)
    }

    /// Open requests and wait (`Running{Idle} → Waiting`).
    pub async fn wait(
        &self,
        scope: &RunScope,
        run: &RunId,
        fence: &LeaseFence,
        requests: Vec<NewRequest>,
    ) -> Result<AgentRunRecord, ServiceError> {
        let ((), rec) = self
            .apply(scope, run, Fence::Lease(fence.clone()), |rec, now| {
                Ok((Some(lifecycle::apply_wait(rec, requests.clone(), now)?), ()))
            })
            .await?;
        Ok(rec)
    }

    /// End the run from its idle boundary.
    pub async fn complete(
        &self,
        scope: &RunScope,
        run: &RunId,
        fence: &LeaseFence,
        status: TerminalStatus,
        outcome: RunOutcome,
        reason: Option<String>,
    ) -> Result<AgentRunRecord, ServiceError> {
        let ((), rec) = self
            .apply(scope, run, Fence::Lease(fence.clone()), |rec, now| {
                Ok((
                    Some(lifecycle::apply_terminal(
                        rec,
                        status,
                        outcome.clone(),
                        reason.clone(),
                        now,
                    )?),
                    (),
                ))
            })
            .await?;
        Ok(rec)
    }

    /// Write a checkpoint at idle.
    pub async fn checkpoint(
        &self,
        scope: &RunScope,
        run: &RunId,
        fence: &LeaseFence,
        reason: CheckpointReason,
        summary: Option<String>,
    ) -> Result<AgentRunRecord, ServiceError> {
        let ((), rec) = self
            .apply(scope, run, Fence::Lease(fence.clone()), |rec, now| {
                Ok((
                    Some(lifecycle::apply_checkpoint(
                        rec,
                        reason,
                        summary.clone(),
                        now,
                    )),
                    (),
                ))
            })
            .await?;
        Ok(rec)
    }

    /// Scan one scope and repair what a crash or a lost wake left behind.
    ///
    /// - `Running`/`Cancelling` with an expired lease: take over (see
    ///   [`lifecycle::apply_takeover`]);
    /// - any run with an open request past its expiry: close it (`expired`);
    /// - `Queued`: wake it again (idempotent).
    pub async fn recover(
        &self,
        scope: &RunScope,
        owner: &str,
    ) -> Result<Vec<Recovered>, ServiceError> {
        const PAGE: usize = 10_000;
        let ttl = self.config.lease_ttl_ms;
        let mut out = Vec::new();
        for status in [RunStatus::Running, RunStatus::Cancelling] {
            for run in self.store.scan_status(scope, status, PAGE).await? {
                let res = self
                    .apply(scope, &run, Fence::None, |rec, now| {
                        let expired = rec.state.lease().is_some_and(|l| l.expires_at_ms <= now);
                        if !expired {
                            return Ok((None, None));
                        }
                        let tk = lifecycle::apply_takeover(rec, owner, now, ttl)?;
                        let status = tk.transition.record.state.status();
                        Ok((Some(tk.transition), Some((tk.redispatch, status))))
                    })
                    .await;
                match res {
                    Ok((Some((Some(fence), _)), _)) => {
                        out.push(Recovered::Redispatch { run_id: run, fence })
                    }
                    Ok((Some((None, status)), _)) => out.push(Recovered::TakenOver {
                        run_id: run,
                        status,
                    }),
                    Ok((None, _)) => {}
                    Err(ServiceError::Refused(_)) => {}
                    Err(e) => return Err(e),
                }
            }
        }
        for status in [
            RunStatus::Waiting,
            RunStatus::Paused,
            RunStatus::Queued,
            RunStatus::Running,
        ] {
            for run in self.store.scan_status(scope, status, PAGE).await? {
                let (expired, _) = self
                    .apply(scope, &run, Fence::None, |rec, now| {
                        Ok(match lifecycle::apply_expire_requests(rec, now) {
                            Some(t) => (Some(t), true),
                            None => (None, false),
                        })
                    })
                    .await?;
                if expired {
                    out.push(Recovered::RequestsExpired { run_id: run });
                }
            }
        }
        for run in self
            .store
            .scan_status(scope, RunStatus::Queued, PAGE)
            .await?
        {
            if let Some(rec) = self.store.load(scope, &run).await? {
                if let RunState::Queued { wake, .. } = rec.state {
                    self.waker.wake(&rec.scope, &run, wake);
                    out.push(Recovered::Woken { run_id: run });
                }
            }
        }
        self.cancels.gc(self.now().saturating_sub(600_000));
        Ok(out)
    }

    /// Resolve the `External` request whose `resume_key` matches. Only an
    /// executor (a system caller) may deliver; deduplicated by `delivery_id`.
    pub async fn deliver_external_result(
        &self,
        scope: &RunScope,
        run: &RunId,
        resume_key: &str,
        delivery_id: &str,
        envelope: Value,
        _system: &SystemToken,
    ) -> Result<ControlAck, ServiceError> {
        let control_id = ControlId(format!("ext:{delivery_id}"));
        let digest = blake3::hash(raisin_agent_contract::canonical_json(&envelope).as_bytes())
            .to_hex()
            .to_string();
        if let Some((ack, stored)) = self
            .store
            .control_ack(scope, run, control_id.as_str())
            .await?
        {
            return if stored == digest {
                Ok(ack.as_duplicate())
            } else {
                Err(ServiceError::Invalid("delivery_id reused".into()))
            };
        }
        let ack = self
            .apply(scope, run, Fence::None, |rec, now| {
                let mut tx = Tx::new(rec, now);
                let found = rec.state.open_requests().into_iter().find(|r| {
                    matches!(&r.kind, PendingKind::External { resume_key: k, .. } if k == resume_key)
                });
                let ack = match found {
                    None => {
                        let seq = tx.push(RunEventKind::ControlRejected { control_id: control_id.clone(), reason: "request_not_open".into() });
                        ControlAck::Rejected { reason: "request_not_open".into(), seq }
                    }
                    Some(request) => {
                        let PendingKind::External { op_id, for_call_id, tool, .. } = &request.kind else { unreachable!("matched above") };
                        let bytes = serde_json::to_vec(&envelope).unwrap_or_default();
                        let r = ResultRef { key: control_id.0.clone(), bytes: bytes.len() as u64, content_type: "application/json".into() };
                        tx.results.push((r.clone(), bytes));
                        let resolution = json!({
                            "kind": "external", "result_key": r.key, "op_id": op_id,
                            "effect_id": request.effect_id, "delivery_id": delivery_id,
                            "for_call_id": for_call_id, "tool": tool,
                        });
                        let seq = tx.push(RunEventKind::RequestResolved { request_id: request.request_id.clone(), resolution, by_control: None });
                        tx.remove_request(&request.request_id);
                        if let RunState::Queued { wake, .. } = &mut tx.rec.state {
                            if tx.wake.is_some() {
                                *wake = WakeReason::ExternalResult;
                                tx.wake = Some(WakeReason::ExternalResult);
                            }
                        }
                        ControlAck::Applied { seq }
                    }
                };
                tx.control = Some((control_id.clone(), ack.clone(), digest.clone()));
                Ok((Some(tx.finish()), ack))
            })
            .await;
        match ack {
            Ok((ack, _)) => Ok(ack),
            // A concurrent duplicate delivery won the race.
            Err(ServiceError::Store(StoreError::ControlDuplicate { ack })) => Ok(ack),
            Err(e) => Err(e),
        }
    }

    /// Wake reason of a queued run, for drivers that want it.
    pub fn wake_reason(rec: &AgentRunRecord) -> Option<WakeReason> {
        match rec.state {
            RunState::Queued { wake, .. } => Some(wake),
            _ => None,
        }
    }
}
