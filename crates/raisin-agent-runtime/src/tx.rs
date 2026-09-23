// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! A transition under construction.
//!
//! Every pure transition builds on a [`Tx`]: a copy of the record plus the
//! events it emits. Because a commit is version-checked, the store assigns
//! seqs `last_seq + 1 ..` in push order — so a transition KNOWS the seq of every
//! event it emits, and can write it into a checkpoint, a steer or a request
//! without a second pass.
//!
//! The helpers here carry the rules every transition shares: a status change
//! is logged; leaving a lease-holding state bumps `lease_epoch`; leaving
//! `Running` ends the open turn; a terminal commit supersedes the stored
//! projection and clears everything a terminal record may not hold.

use serde_json::Value;

use crate::checkpoint::{self, RunCheckpoint};
use crate::control::ControlAck;
use crate::events::{CheckpointReason, NewEvent, ResultRef, RunEventKind};
use crate::ids::{ControlId, OperationId, RequestId, RunId, Seq};
use crate::record::{AgentRunRecord, PendingRequest};
use crate::state::{Activity, NonEmpty, RunOutcome, RunState, TerminalStatus, WakeReason};

/// A computed transition, ready to commit.
#[derive(Debug, Clone)]
pub struct Transition {
    /// The record after the transition (version and `last_seq` already set).
    pub record: AgentRunRecord,
    /// The version the transition was computed from.
    pub expected_version: crate::ids::Version,
    /// Events, in order.
    pub events: Vec<NewEvent>,
    /// An operation to cancel after the commit.
    pub cancel: Option<(RunId, OperationId)>,
    /// Wake the run after the commit.
    pub wake: Option<WakeReason>,
    /// A checkpoint to write.
    pub checkpoint: Option<RunCheckpoint>,
    /// Idempotency markers to write (`idem\0{key}` → seq of completion).
    pub idem: Vec<(String, Seq)>,
    /// Control ack to record (`ctl\0{id}`), with the payload digest.
    pub control: Option<(ControlId, ControlAck, String)>,
    /// Domain state to write once under `dom\0{rev}`.
    pub domain_state: Option<(u64, Value)>,
    /// Large payloads.
    pub results: Vec<(ResultRef, Vec<u8>)>,
}

/// A transition under construction.
#[derive(Debug, Clone)]
pub struct Tx {
    base_seq: Seq,
    base_version: crate::ids::Version,
    /// The record being transformed.
    pub rec: AgentRunRecord,
    /// Commit time.
    pub now_ms: u64,
    events: Vec<NewEvent>,
    /// See [`Transition::cancel`].
    pub cancel: Option<(RunId, OperationId)>,
    /// See [`Transition::wake`].
    pub wake: Option<WakeReason>,
    /// See [`Transition::checkpoint`].
    pub checkpoint: Option<RunCheckpoint>,
    /// See [`Transition::idem`].
    pub idem: Vec<(String, Seq)>,
    /// See [`Transition::control`].
    pub control: Option<(ControlId, ControlAck, String)>,
    /// See [`Transition::domain_state`].
    pub domain_state: Option<(u64, Value)>,
    /// See [`Transition::results`].
    pub results: Vec<(ResultRef, Vec<u8>)>,
}

impl Tx {
    /// Start from the stored record.
    pub fn new(rec: &AgentRunRecord, now_ms: u64) -> Self {
        Self {
            base_seq: rec.last_seq,
            base_version: rec.version,
            rec: rec.clone(),
            now_ms,
            events: Vec::new(),
            cancel: None,
            wake: None,
            checkpoint: None,
            idem: Vec::new(),
            control: None,
            domain_state: None,
            results: Vec::new(),
        }
    }

    /// The seq the next pushed event will get.
    pub fn next_seq(&self) -> Seq {
        Seq(self.base_seq.0 + self.events.len() as u64 + 1)
    }

    /// Number of events pushed so far.
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// The events pushed so far.
    pub fn events(&self) -> &[NewEvent] {
        &self.events
    }

    /// Push an event; returns its seq.
    pub fn push(&mut self, kind: RunEventKind) -> Seq {
        let op_id = op_id_of(&kind);
        self.push_with(op_id, kind)
    }

    /// Push an event about `op_id`; returns its seq.
    pub fn push_with(&mut self, op_id: Option<OperationId>, kind: RunEventKind) -> Seq {
        let seq = self.next_seq();
        self.events.push(NewEvent {
            turn: self.rec.current_turn,
            op_id,
            kind,
        });
        seq
    }

    /// Replace the state, applying the shared rules (see module docs).
    pub fn set_state(&mut self, next: RunState, reason: Option<String>) {
        let from = self.rec.state.status();
        let to = next.status();
        let had_lease = self.rec.state.lease().is_some();
        let has_lease = next.lease().is_some();
        let leaving_running = self.rec.state.lease().is_some() && next.lease().is_none();
        if leaving_running {
            if let Some(turn) = self.rec.current_turn.take() {
                self.events.push(NewEvent {
                    turn: Some(turn),
                    op_id: None,
                    kind: RunEventKind::TurnEnded { turn },
                });
            }
        }
        if had_lease && !has_lease {
            self.rec.lease_epoch = self.rec.lease_epoch.next();
            let epoch = self.rec.lease_epoch;
            self.push(RunEventKind::LeaseReleased { epoch });
        }
        if next.is_terminal() {
            if let Some(d) = self.rec.domain.as_mut() {
                d.projection_superseded = true;
                d.outbox = None;
            }
        }
        if let RunState::Queued { wake, .. } = &next {
            if from != to {
                self.wake = Some(*wake);
            }
        }
        self.rec.state = next;
        if from != to {
            self.push(RunEventKind::StatusChanged { from, to, reason });
        }
    }

    /// Write a checkpoint of the current record.
    pub fn checkpoint(&mut self, reason: CheckpointReason, summary: Option<String>) {
        self.rec.counters.checkpoint += 1;
        let checkpoint_no = self.rec.counters.checkpoint;
        let seq = self.next_seq();
        self.rec.last_checkpoint_seq = Some(seq);
        self.push(RunEventKind::CheckpointWritten {
            checkpoint_no,
            at_seq: seq,
            reason,
        });
        self.checkpoint = Some(checkpoint::build(
            &self.rec,
            reason,
            seq,
            self.now_ms,
            summary,
        ));
    }

    /// End the run: close open requests, discard steers, drop unanswered
    /// calls and the domain outbox, then log `Terminal`.
    pub fn terminate(
        &mut self,
        status: TerminalStatus,
        outcome: RunOutcome,
        reason: Option<String>,
    ) {
        for request in self.rec.state.open_requests() {
            self.push(RunEventKind::RequestClosed {
                request_id: request.request_id,
                reason: "cancelled".into(),
            });
        }
        for steer in std::mem::take(&mut self.rec.steer_queue) {
            self.push(RunEventKind::SteerDiscarded {
                steer_id: steer.steer_id,
            });
        }
        self.rec.unanswered_calls.clear();
        self.rec.status_reason = reason.clone();
        self.set_state(
            RunState::Terminal {
                terminal: status,
                outcome: outcome.clone(),
            },
            reason,
        );
        self.push(RunEventKind::Terminal { status, outcome });
    }

    /// Remove an open request. A `Waiting` run left with none goes `Queued`.
    /// Returns the removed request.
    pub fn remove_request(&mut self, request_id: &RequestId) -> Option<PendingRequest> {
        let mut open = self.rec.state.open_requests();
        let pos = open.iter().position(|r| &r.request_id == request_id)?;
        let removed = open.remove(pos);
        let state = self.rec.state.clone();
        match state {
            RunState::Waiting { .. } => match NonEmpty::from_vec(open) {
                Some(rest) => self.rec.state = RunState::Waiting { open: rest },
                None => self.set_state(
                    RunState::Queued {
                        open: Vec::new(),
                        wake: WakeReason::RequestResolved,
                    },
                    None,
                ),
            },
            RunState::Queued { wake, .. } => self.rec.state = RunState::Queued { open, wake },
            RunState::Paused { reason, .. } => self.rec.state = RunState::Paused { open, reason },
            RunState::Running {
                lease,
                activity: Activity::Idle { .. },
            } => {
                self.rec.state = RunState::Running {
                    lease,
                    activity: Activity::Idle { open },
                }
            }
            _ => return None,
        }
        Some(removed)
    }

    /// Finish: set version, `last_seq` and `updated_at_ms`.
    pub fn finish(mut self) -> Transition {
        self.rec.version = self.base_version.next();
        self.rec.last_seq = Seq(self.base_seq.0 + self.events.len() as u64);
        self.rec.updated_at_ms = self.now_ms;
        Transition {
            record: self.rec,
            expected_version: self.base_version,
            events: self.events,
            cancel: self.cancel,
            wake: self.wake,
            checkpoint: self.checkpoint,
            idem: self.idem,
            control: self.control,
            domain_state: self.domain_state,
            results: self.results,
        }
    }
}

/// The operation an event kind is about, if any.
fn op_id_of(kind: &RunEventKind) -> Option<OperationId> {
    match kind {
        RunEventKind::OperationStarted { op_id, .. }
        | RunEventKind::OperationCompleted { op_id, .. }
        | RunEventKind::OperationCancelled { op_id, .. }
        | RunEventKind::OperationAbandoned { op_id, .. } => Some(op_id.clone()),
        _ => None,
    }
}
