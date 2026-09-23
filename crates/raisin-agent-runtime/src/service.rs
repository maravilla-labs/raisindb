// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! `AgentRunService`: create, read, subscribe, control.
//!
//! Every mutation is read → pure transition → `commit`, retried a bounded
//! number of times on a version conflict. Controls commit with `Fence::None`
//! and only ever take the store's short per-run critical section, so a stop
//! never waits behind a model call. The executor-facing half (leases,
//! operations, recovery) is in `service_exec`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use raisin_agent_contract::{effective_projection, PlanProjection};
use serde_json::Value;
use tokio::sync::watch;

use crate::cancel::CancellationRegistry;
use crate::clock::Clock;
use crate::control::{authorize, capability_hash, ControlAck, ControlCommand};
use crate::domain::{DomainBinding, ReducerRef};
use crate::events::{RunEvent, RunEventKind};
use crate::ids::{
    Principal, PrincipalKind, RunId, RunScope, Seq, SubjectRef, SystemToken, Version,
};
use crate::lifecycle::{apply_control, BeginRefusal, Fence, TransitionRefusal};
use crate::record::{AgentRunRecord, Counters, RunBudgets, RunUsage};
use crate::state::{RunState, WakeReason};
use crate::store::{AgentRunStore, CommitRequest, CreateOutcome, StoreError};
use crate::tx::{Transition, Tx};
use crate::wake::RunWaker;

/// Tunables.
#[derive(Debug, Clone)]
pub struct ServiceConfig {
    /// Lease time-to-live.
    pub lease_ttl_ms: u64,
    /// Renewal period while an operation runs.
    pub renew_every_ms: u64,
    /// Version-conflict retries per mutation.
    pub max_conflict_retries: usize,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            lease_ttl_ms: 90_000,
            renew_every_ms: 30_000,
            max_conflict_retries: 5,
        }
    }
}

/// Service failures.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ServiceError {
    /// Storage.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// No such run.
    #[error("run not found")]
    NotFound,
    /// A pure transition refused.
    #[error(transparent)]
    Refused(#[from] TransitionRefusal),
    /// `begin_operation` refused.
    #[error("begin refused: {0:?}")]
    Begin(BeginRefusal),
    /// Not allowed.
    #[error("unauthorized")]
    Unauthorized,
    /// Bad input.
    #[error("invalid input: {0}")]
    Invalid(String),
}

impl ServiceError {
    /// The caller no longer holds the lease; it must stop without writing.
    pub fn is_lease_lost(&self) -> bool {
        matches!(self, Self::Store(StoreError::LeaseLost { .. }))
    }
}

/// Everything needed to create a run.
#[derive(Debug, Clone)]
pub struct CreateRun {
    /// Scope.
    pub scope: RunScope,
    /// Subject.
    pub subject: SubjectRef,
    /// Principal.
    pub principal: Principal,
    /// Control capability (stored hashed).
    pub control_capability: Option<String>,
    /// Opaque agent reference.
    pub agent_ref: Option<String>,
    /// Idempotency key of the create.
    pub create_key: Option<String>,
    /// Budgets.
    pub budgets: RunBudgets,
    /// Input, delivered to a reducer as `run_started.data.input`.
    pub input: Value,
    /// Domain reducer, if any.
    pub reducer: Option<ReducerRef>,
    /// Opaque executor configuration (see `AgentRunRecord::executor_config`).
    pub executor_config: Option<Value>,
    /// What waits for the run to end outside its tree (a flow step).
    pub waiter: Option<crate::waiter::RunWaiter>,
}

/// The record of a freshly created run (`Queued`, version 1, seq 1).
pub fn new_record(req: &CreateRun, run_id: RunId, now: u64) -> AgentRunRecord {
    AgentRunRecord {
        run_id,
        scope: req.scope.clone(),
        subject: req.subject.clone(),
        principal: req.principal.clone(),
        control_capability_hash: req.control_capability.as_deref().map(capability_hash),
        agent_ref: req.agent_ref.clone(),
        create_key: req.create_key.clone(),
        state: RunState::Queued {
            open: Vec::new(),
            wake: WakeReason::Created,
        },
        status_reason: None,
        version: Version(1),
        last_seq: Seq(1),
        created_at_ms: now,
        updated_at_ms: now,
        counters: Counters::default(),
        current_turn: None,
        lease_epoch: crate::ids::LeaseEpoch(0),
        steer_queue: Vec::new(),
        unanswered_calls: Vec::new(),
        budgets: req.budgets.clone(),
        usage: RunUsage::default(),
        last_checkpoint_seq: None,
        domain: req.reducer.clone().map(DomainBinding::new),
        parent_run_id: None,
        root_run_id: None,
        depth: 0,
        delegation: None,
        children: Vec::new(),
        mailbox: Vec::new(),
        large_results: Vec::new(),
        executor_config: req.executor_config.clone(),
        waiter: req.waiter.clone(),
    }
}

/// The run service.
pub struct AgentRunService {
    pub(crate) store: Arc<dyn AgentRunStore>,
    pub(crate) waker: Arc<dyn RunWaker>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) cancels: Arc<CancellationRegistry>,
    subs: Mutex<HashMap<RunId, watch::Sender<Seq>>>,
    pub(crate) run_locks: crate::keyed::KeyedMutex<RunId>,
    pub(crate) waiter_sink: std::sync::OnceLock<Arc<dyn crate::waiter::WaiterSink>>,
    /// Tunables.
    pub config: ServiceConfig,
}

impl AgentRunService {
    /// A service over `store`.
    pub fn new(
        store: Arc<dyn AgentRunStore>,
        waker: Arc<dyn RunWaker>,
        clock: Arc<dyn Clock>,
        config: ServiceConfig,
    ) -> Self {
        Self {
            store,
            waker,
            clock,
            cancels: Arc::new(CancellationRegistry::new()),
            subs: Mutex::new(HashMap::new()),
            run_locks: Default::default(),
            waiter_sink: std::sync::OnceLock::new(),
            config,
        }
    }

    /// Install where waiters' results go (once; later calls are ignored).
    pub fn set_waiter_sink(&self, sink: Arc<dyn crate::waiter::WaiterSink>) {
        let _ = self.waiter_sink.set(sink);
    }

    /// The store.
    pub fn store(&self) -> &Arc<dyn AgentRunStore> {
        &self.store
    }

    /// The cancellation registry.
    pub fn cancels(&self) -> &Arc<CancellationRegistry> {
        &self.cancels
    }

    /// Now, per the service clock.
    pub fn now(&self) -> u64 {
        self.clock.now_ms()
    }

    /// Create a run, or find the live run of its subject / its create key.
    pub async fn create(
        &self,
        req: CreateRun,
        system: Option<&SystemToken>,
    ) -> Result<CreateOutcome, ServiceError> {
        if req.principal.kind == PrincipalKind::System && system.is_none() {
            return Err(ServiceError::Unauthorized);
        }
        let now = self.now();
        let rec = new_record(&req, RunId::new_v4(), now);
        let first = RunEventKind::RunCreated {
            subject: req.subject.clone(),
            principal: req.principal.clone(),
            budgets: req.budgets.clone(),
            input: req.input.clone(),
        };
        let outcome = self
            .store
            .create(rec, first, req.create_key.as_deref())
            .await?;
        if let CreateOutcome::Created { run_id, seq } = &outcome {
            self.notify(run_id, *seq);
            self.waker.wake(&req.scope, run_id, WakeReason::Created);
        }
        Ok(outcome)
    }

    /// Load a record.
    pub async fn get(
        &self,
        scope: &RunScope,
        run: &RunId,
    ) -> Result<Option<AgentRunRecord>, ServiceError> {
        Ok(self.store.load(scope, run).await?)
    }

    /// Events after `after_seq`: replay-safe and gap-free.
    pub async fn read_events(
        &self,
        scope: &RunScope,
        run: &RunId,
        after_seq: Seq,
        limit: usize,
    ) -> Result<Vec<RunEvent>, ServiceError> {
        Ok(self.store.read_events(scope, run, after_seq, limit).await?)
    }

    /// "Something new, reread from your last seq." A lagging subscriber never
    /// loses an event: the value is only a hint, the log is the truth.
    pub fn subscribe(&self, run: &RunId) -> watch::Receiver<Seq> {
        let mut subs = self.subs.lock().expect("subs poisoned");
        subs.entry(run.clone())
            .or_insert_with(|| watch::channel(Seq(0)).0)
            .subscribe()
    }

    pub(crate) fn notify(&self, run: &RunId, last: Seq) {
        if let Some(tx) = self.subs.lock().expect("subs poisoned").get(run) {
            tx.send_replace(last);
        }
    }

    /// The projection a reader should display, whatever the domain stored.
    pub async fn effective_projection(
        &self,
        scope: &RunScope,
        run: &RunId,
    ) -> Result<Option<PlanProjection>, ServiceError> {
        let rec = self
            .store
            .load(scope, run)
            .await?
            .ok_or(ServiceError::NotFound)?;
        let Some(rev) = rec.domain.as_ref().and_then(|d| d.projection_rev) else {
            return Ok(None);
        };
        let stored = self.store.domain_state(scope, run, rev).await?;
        let projection = stored
            .and_then(|v| v.get("projection").cloned())
            .and_then(|p| PlanProjection::parse(&p).ok().flatten());
        Ok(effective_projection(
            projection.as_ref(),
            rec.state.status(),
        ))
    }

    /// Read → transition → commit, retried on version conflicts.
    pub(crate) async fn apply<T, F>(
        &self,
        scope: &RunScope,
        run: &RunId,
        fence: Fence,
        mut f: F,
    ) -> Result<(T, AgentRunRecord), ServiceError>
    where
        F: FnMut(&AgentRunRecord, u64) -> Result<(Option<Transition>, T), ServiceError>,
    {
        let mut attempt = 0;
        loop {
            // The commit mutex: in-process callers never race each other; a
            // version conflict can only come from another process.
            let _guard = self.run_locks.lock(run.clone()).await;
            let rec = self
                .store
                .load(scope, run)
                .await?
                .ok_or(ServiceError::NotFound)?;
            let now = self.now();
            // A fenced call on a record whose lease is gone learns it LOST the
            // lease — whatever the transition itself would have said.
            if let Fence::Lease(_) = &fence {
                if crate::lifecycle::check_fence(&rec, &fence, now).is_err() {
                    return Err(StoreError::LeaseLost {
                        current_epoch: rec.lease_epoch.0,
                    }
                    .into());
                }
            }
            let (t, out) = f(&rec, now)?;
            let Some(t) = t else { return Ok((out, rec)) };
            match self.commit(scope, &t, fence.clone(), now).await {
                Ok(()) => return Ok((out, t.record)),
                Err(StoreError::VersionConflict { .. })
                    if attempt < self.config.max_conflict_retries =>
                {
                    attempt += 1
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Commit a transition and run its post-commit actions.
    pub(crate) async fn commit(
        &self,
        scope: &RunScope,
        t: &Transition,
        fence: Fence,
        now: u64,
    ) -> Result<(), StoreError> {
        self.store
            .commit(CommitRequest::from_transition(scope, t, fence, now))
            .await?;
        if !t.events.is_empty() {
            self.notify(&t.record.run_id, t.record.last_seq);
        }
        if let Some((run, op)) = &t.cancel {
            self.cancels.cancel(run, op, now);
        }
        // Wakes carry the run's OWN scope: its branch lives in the record,
        // whatever scope the caller addressed it through.
        let own = &t.record.scope;
        if let (Some(reason), RunState::Queued { .. }) = (t.wake, &t.record.state) {
            self.waker.wake(own, &t.record.run_id, reason);
        }
        // A terminal commit of a domain run: whoever drives it next must
        // finalize (deliver the rest of the log, e.g. `stopped`). A driver that
        // is in the middle of it finalizes anyway; a stop that landed while no
        // driver held the run would otherwise leave the domain unfinalized.
        let ended = t
            .events
            .iter()
            .any(|e| matches!(e.kind, RunEventKind::Terminal { .. }));
        if ended && t.record.domain.as_ref().is_some_and(|d| !d.finalized) {
            self.waker.wake(own, &t.record.run_id, WakeReason::Finalize);
        }
        // The lineage is owed its share too: a child's hand-back, a parent's
        // cascade. Both run from a job on whichever node picks it up.
        for reason in Self::lineage_wakes(&t.record, ended) {
            self.waker.wake(own, &t.record.run_id, reason);
        }
        Ok(())
    }

    /// Submit a control. Deduplicated by `control_id` + payload digest.
    pub async fn submit_control(
        &self,
        scope: &RunScope,
        run: &RunId,
        cmd: ControlCommand,
        system: Option<&SystemToken>,
    ) -> Result<ControlAck, ServiceError> {
        let digest = crate::control::payload_digest(&cmd.kind);
        let mut attempt = 0;
        loop {
            let guard = self.run_locks.lock(run.clone()).await;
            if let Some((ack, stored)) = self
                .store
                .control_ack(scope, run, cmd.control_id.as_str())
                .await?
            {
                drop(guard);
                if stored == digest {
                    return Ok(ack.as_duplicate());
                }
                return self
                    .log_rejection(scope, run, &cmd, "control_id_reused")
                    .await;
            }
            let rec = self
                .store
                .load(scope, run)
                .await?
                .ok_or(ServiceError::NotFound)?;
            let now = self.now();
            let authorized = authorize(&rec, &cmd.issued_by, system);
            let ct = apply_control(&rec, &cmd, authorized, now);
            let committed = self.commit(scope, &ct.transition, Fence::None, now).await;
            drop(guard);
            match committed {
                Ok(()) => return Ok(ct.ack),
                Err(StoreError::ControlDuplicate { ack }) => return Ok(ack),
                Err(StoreError::ControlReused) => {
                    return self
                        .log_rejection(scope, run, &cmd, "control_id_reused")
                        .await
                }
                Err(StoreError::VersionConflict { .. })
                    if attempt < self.config.max_conflict_retries =>
                {
                    attempt += 1
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Log a rejection WITHOUT touching the stored ack of `cmd.control_id`.
    async fn log_rejection(
        &self,
        scope: &RunScope,
        run: &RunId,
        cmd: &ControlCommand,
        reason: &str,
    ) -> Result<ControlAck, ServiceError> {
        let (ack, _) = self
            .apply(scope, run, Fence::None, |rec, now| {
                let mut tx = Tx::new(rec, now);
                tx.push(RunEventKind::ControlReceived {
                    control_id: cmd.control_id.clone(),
                    command: cmd.kind.clone(),
                    issued_by: cmd.issued_by.redacted(),
                });
                let seq = tx.push(RunEventKind::ControlRejected {
                    control_id: cmd.control_id.clone(),
                    reason: reason.into(),
                });
                Ok((
                    Some(tx.finish()),
                    ControlAck::Rejected {
                        reason: reason.into(),
                        seq,
                    },
                ))
            })
            .await?;
        Ok(ack)
    }
}
