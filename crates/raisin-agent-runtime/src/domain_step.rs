// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Delivering reducer calls: one commit per response, the log as inbox, and a
//! lease-free finalize.
//!
//! For each undelivered deliverable event, in seq order, the driver calls the
//! reducer, validates the response, and commits in ONE `CommitRequest`: the new
//! state under `dom\0{rev}`, `DomainApplied`, `delivered_seq = event.seq`, and
//! the effect translation (`OperationStarted` / `RequestOpened` /
//! `CheckpointWritten` / `Terminal`). If the budget refuses the operation, the
//! effect goes into the outbox in that same commit. No crash point can leave
//! "state says awaiting X" without a matching `OperationStarted` or outbox entry.

use async_trait::async_trait;
use raisin_agent_contract::{validate_response, ReducerResponse};
use serde_json::Value;

use crate::domain::{build_request, map_event, DomainReducer, ReducerCallError, ResultLoader};
pub use crate::domain_apply::apply_domain_response;
use crate::domain_apply::start_or_park;
use crate::events::RunEvent;
use crate::ids::{RunId, RunScope, Seq};
use crate::lifecycle::{self, Fence, LeaseFence};
use crate::record::AgentRunRecord;
use crate::service::{AgentRunService, ServiceError};
use crate::state::{RunOutcome, RunState, TerminalStatus};
use crate::store::AgentRunStore;
use crate::tx::{Transition, Tx};

/// What one domain step did.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum DomainStep {
    /// State changed; call again (more events may be undelivered).
    Continue,
    /// An operation was started.
    Operation(crate::record::ActiveOperation),
    /// The run left `Running` (waiting, paused, terminal).
    Left,
    /// Nothing deliverable and nothing to dispatch.
    Stalled,
}

struct StoreLoader<'a> {
    store: &'a dyn AgentRunStore,
    scope: &'a RunScope,
    run: &'a RunId,
}

#[async_trait]
impl ResultLoader for StoreLoader<'_> {
    async fn load_json(&self, key: &str) -> Option<Value> {
        let bytes = self
            .store
            .read_result(self.scope, self.run, key)
            .await
            .ok()??;
        serde_json::from_slice(&bytes).ok()
    }
}

/// Events read per page while looking for the next deliverable one.
const PAGE: usize = 256;

/// First deliverable event in `events`, with its reducer form.
async fn next_deliverable(
    events: &[RunEvent],
    loader: &dyn ResultLoader,
) -> Option<(Seq, raisin_agent_contract::ReducerEvent)> {
    for ev in events {
        if let Some(re) = map_event(ev, loader).await {
            return Some((ev.seq, re));
        }
    }
    None
}

/// First deliverable event after `after`, however many non-deliverable events
/// (control acks, lease moves, …) lie in between: the inbox is the whole log,
/// not its next page.
async fn scan_deliverable(
    store: &dyn AgentRunStore,
    scope: &RunScope,
    run: &RunId,
    after: Seq,
    loader: &dyn ResultLoader,
) -> Result<Option<(Seq, raisin_agent_contract::ReducerEvent)>, ServiceError> {
    let mut cursor = after;
    loop {
        let page = store.read_events(scope, run, cursor, PAGE).await?;
        if let Some(found) = next_deliverable(&page, loader).await {
            return Ok(Some(found));
        }
        match page.last() {
            Some(last) if page.len() == PAGE => cursor = last.seq,
            _ => return Ok(None),
        }
    }
}

impl AgentRunService {
    async fn call_reducer(
        &self,
        scope: &RunScope,
        rec: &AgentRunRecord,
        reducer: &dyn DomainReducer,
        event: raisin_agent_contract::ReducerEvent,
    ) -> Result<(raisin_agent_contract::ReducerRequest, ReducerResponse), ReducerCallError> {
        let b = rec.domain.as_ref().expect("domain run");
        if reducer.reducer_ref().artifact_hash != b.reducer.artifact_hash {
            return Err(ReducerCallError::Changed {
                actual_hash: reducer.reducer_ref().artifact_hash.clone(),
            });
        }
        let state = if b.state_rev == 0 {
            None
        } else {
            self.store
                .domain_state(scope, &rec.run_id, b.state_rev)
                .await
                .ok()
                .flatten()
                .and_then(|v| v.get("state").cloned())
        };
        let req = build_request(rec, b, state, event);
        let resp = reducer.reduce(&req).await?;
        if let Some(r) = &resp.refused {
            return Err(ReducerCallError::Refused {
                code: r.code.clone(),
                message: r.message.clone(),
            });
        }
        validate_response(&req, &resp).map_err(ReducerCallError::Invalid)?;
        Ok((req, resp))
    }

    /// One domain step at the idle boundary, under the caller's lease.
    pub async fn domain_step(
        &self,
        scope: &RunScope,
        run: &RunId,
        fence: &LeaseFence,
        reducer: &dyn DomainReducer,
    ) -> Result<DomainStep, ServiceError> {
        let rec = self
            .store
            .load(scope, run)
            .await?
            .ok_or(ServiceError::NotFound)?;
        let b = rec
            .domain
            .clone()
            .ok_or_else(|| ServiceError::Invalid("run has no reducer".into()))?;
        let fenced = Fence::Lease(fence.clone());
        if let Some(pending) = b.outbox.clone() {
            let now = self.now();
            let mut tx = Tx::new(&rec, now);
            if let Some(d) = tx.rec.domain.as_mut() {
                d.outbox = None;
            }
            start_or_park(&mut tx, &pending.effect, pending.state_rev)?;
            let t = tx.finish();
            return self.commit_step(scope, &t, fenced, now).await;
        }
        let loader = StoreLoader {
            store: self.store.as_ref(),
            scope,
            run,
        };
        let found =
            scan_deliverable(self.store.as_ref(), scope, run, b.delivered_seq, &loader).await?;
        let Some((seq, event)) = found else {
            return Ok(DomainStep::Stalled);
        };
        let now = self.now();
        let t = match self.call_reducer(scope, &rec, reducer, event).await {
            Ok((_, resp)) => match apply_domain_response(&rec, seq, &resp, None, now) {
                Ok(t) => t,
                Err(r) => fail_transition(&rec, &format!("reducer_refused:{}", r.code), now)?,
            },
            Err(ReducerCallError::Unavailable(_)) => {
                lifecycle::apply_reducer_pause(&rec, "reducer_unavailable", now)?
            }
            Err(ReducerCallError::Changed { actual_hash }) => {
                let mut t = lifecycle::apply_reducer_pause(&rec, "reducer_changed", now)?;
                if let Some(d) = t.record.domain.as_mut() {
                    d.pending_artifact_hash = Some(actual_hash);
                }
                t
            }
            Err(ReducerCallError::Refused { code, .. }) => {
                fail_transition(&rec, &format!("reducer_refused:{code}"), now)?
            }
            Err(ReducerCallError::Invalid(r)) if r.code == "contract_mismatch" => {
                fail_transition(&rec, "reducer_contract_mismatch", now)?
            }
            Err(ReducerCallError::Invalid(r)) => {
                fail_transition(&rec, &format!("reducer_refused:{}", r.code), now)?
            }
        };
        self.commit_step(scope, &t, fenced, now).await
    }

    /// Commit a step; a version conflict (a control landed meanwhile) means
    /// "re-run the step" — the reducer is pure, so re-invoking it is safe.
    async fn commit_step(
        &self,
        scope: &RunScope,
        t: &Transition,
        fence: Fence,
        now: u64,
    ) -> Result<DomainStep, ServiceError> {
        let _guard = self.run_locks.lock(t.record.run_id.clone()).await;
        match self.commit(scope, t, fence, now).await {
            Ok(()) => Ok(step_of(&t.record)),
            Err(crate::store::StoreError::VersionConflict { .. }) => Ok(DomainStep::Continue),
            Err(e) => Err(e.into()),
        }
    }

    /// Deliver every undelivered event of a TERMINAL run, lease-free. Fenced
    /// by the state revision read, so two finalizers cannot both commit the
    /// same revision. Returns whether the run is finalized.
    pub async fn finalize_domain(
        &self,
        scope: &RunScope,
        run: &RunId,
        reducer: &dyn DomainReducer,
    ) -> Result<bool, ServiceError> {
        loop {
            let rec = self
                .store
                .load(scope, run)
                .await?
                .ok_or(ServiceError::NotFound)?;
            let Some(b) = rec.domain.clone() else {
                return Ok(true);
            };
            if !rec.state.is_terminal() {
                return Ok(false);
            }
            if b.finalized {
                return Ok(true);
            }
            let fence = Fence::DomainFinalize {
                expected_state_rev: b.state_rev,
            };
            let loader = StoreLoader {
                store: self.store.as_ref(),
                scope,
                run,
            };
            let store = self.store.as_ref();
            let found = scan_deliverable(store, scope, run, b.delivered_seq, &loader).await?;
            let now = self.now();
            let t = match found {
                None => {
                    let mut tx = Tx::new(&rec, now);
                    if let Some(d) = tx.rec.domain.as_mut() {
                        d.finalized = true;
                    }
                    tx.finish()
                }
                Some((seq, event)) => {
                    let last = scan_deliverable(store, scope, run, seq, &loader)
                        .await?
                        .is_none();
                    match self.call_reducer(scope, &rec, reducer, event).await {
                        Ok((_, resp)) => apply_domain_response(&rec, seq, &resp, Some(last), now)?,
                        // Best effort on a terminal run: record delivery and move on.
                        Err(_) => {
                            let mut tx = Tx::new(&rec, now);
                            if let Some(d) = tx.rec.domain.as_mut() {
                                d.delivered_seq = seq;
                                d.finalized = last;
                            }
                            tx.finish()
                        }
                    }
                }
            };
            let guard = self.run_locks.lock(run.clone()).await;
            let committed = self.commit(scope, &t, fence, now).await;
            drop(guard);
            match committed {
                Ok(()) => {}
                Err(crate::store::StoreError::FinalizeFenced) => return Ok(false),
                Err(e) => return Err(e.into()),
            }
        }
    }
}

fn fail_transition(rec: &AgentRunRecord, code: &str, now: u64) -> Result<Transition, ServiceError> {
    let outcome = RunOutcome {
        kind: "failed".into(),
        code: Some(code.into()),
        message: None,
        detail: None,
    };
    Ok(lifecycle::apply_terminal(
        rec,
        TerminalStatus::Failed,
        outcome,
        Some(code.into()),
        now,
    )?)
}

fn step_of(rec: &AgentRunRecord) -> DomainStep {
    match &rec.state {
        RunState::Running {
            activity: crate::state::Activity::Operating { op, .. },
            ..
        } => DomainStep::Operation(op.clone()),
        RunState::Running { .. } => DomainStep::Continue,
        _ => DomainStep::Left,
    }
}
