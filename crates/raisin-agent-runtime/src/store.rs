// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The storage contract. `commit` is the ONE write path.
//!
//! Inside a single critical section (per run), `commit` checks the version,
//! the fence and the invariants, assigns seqs `last_seq + 1 ..`, and writes
//! the record, the events, the control ack, idempotency markers, the checkpoint,
//! domain state, result payloads and every index move in ONE batch.

use async_trait::async_trait;
use serde_json::Value;

use crate::checkpoint::RunCheckpoint;
use crate::control::ControlAck;
use crate::events::{NewEvent, ResultRef, RunEvent, RunEventKind};
use crate::ids::{ControlId, InvalidKey, RunId, RunScope, Seq, Version};
use crate::lifecycle::Fence;
use crate::record::{check_invariants, AgentRunRecord, InvariantViolation};
use crate::state::RunStatus;
use crate::tx::Transition;

/// Result of a create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateOutcome {
    /// A new run, whose `RunCreated` is `seq`.
    Created {
        /// Id.
        run_id: RunId,
        /// Seq of `RunCreated` (always 1).
        seq: Seq,
    },
    /// The subject already has a live run, or the create key was seen.
    Existing {
        /// That run.
        run_id: RunId,
        /// Its status.
        status: RunStatus,
    },
}

/// One commit.
#[derive(Debug, Clone)]
pub struct CommitRequest {
    /// Scope.
    pub scope: RunScope,
    /// Run.
    pub run_id: RunId,
    /// The stored version this commit replaces.
    pub expected_version: Version,
    /// What the commit must prove about the stored record.
    pub fence: Fence,
    /// Commit time (fence expiry and event timestamps).
    pub now_ms: u64,
    /// The next record.
    pub record: AgentRunRecord,
    /// Events, in order.
    pub events: Vec<NewEvent>,
    /// Control ack + payload digest.
    pub control: Option<(ControlId, ControlAck, String)>,
    /// Idempotency markers.
    pub idem: Vec<(String, Seq)>,
    /// Checkpoint.
    pub checkpoint: Option<RunCheckpoint>,
    /// Domain state `(rev, {state, projection})`, write-once.
    pub domain_state: Option<(u64, Value)>,
    /// Large payloads.
    pub results: Vec<(ResultRef, Vec<u8>)>,
}

impl CommitRequest {
    /// A commit of `t` under `fence`.
    pub fn from_transition(scope: &RunScope, t: &Transition, fence: Fence, now_ms: u64) -> Self {
        Self {
            scope: scope.clone(),
            run_id: t.record.run_id.clone(),
            expected_version: t.expected_version,
            fence,
            now_ms,
            record: t.record.clone(),
            events: t.events.clone(),
            control: t.control.clone(),
            idem: t.idem.clone(),
            checkpoint: t.checkpoint.clone(),
            domain_state: t.domain_state.clone(),
            results: t.results.clone(),
        }
    }
}

/// What a commit wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitOutcome {
    /// New version.
    pub version: Version,
    /// Seq of the first event written (`last_seq + 1` when none).
    pub first_seq: Seq,
    /// Seq of the last event.
    pub last_seq: Seq,
}

/// Store failures.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum StoreError {
    /// The stored version moved.
    #[error("version conflict: stored version is {current}")]
    VersionConflict {
        /// Stored version.
        current: Version,
    },
    /// The fence failed (lease lost, expired or taken over).
    #[error("lease lost (current epoch {current_epoch})")]
    LeaseLost {
        /// Stored epoch.
        current_epoch: u64,
    },
    /// A finalize fence failed.
    #[error("finalize fenced")]
    FinalizeFenced,
    /// The record would violate an invariant.
    #[error(transparent)]
    Invariant(#[from] InvariantViolation),
    /// The control id was already applied with the same payload.
    #[error("duplicate control")]
    ControlDuplicate {
        /// The original ack.
        ack: ControlAck,
    },
    /// The control id was already applied with a DIFFERENT payload.
    #[error("control id reused with a different payload")]
    ControlReused,
    /// Another node holds the run's commit section past the acquire budget;
    /// retry.
    #[error("run busy on another node")]
    Busy,
    /// An id cannot enter a key.
    #[error(transparent)]
    InvalidKey(#[from] InvalidKey),
    /// No such run.
    #[error("run not found")]
    NotFound,
    /// A commit's events or record are inconsistent with the stored record.
    #[error("malformed commit: {0}")]
    Malformed(String),
    /// Backend failure.
    #[error("backend: {0}")]
    Backend(String),
}

impl StoreError {
    /// Stable code for transports and logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::VersionConflict { .. } => "version_conflict",
            Self::LeaseLost { .. } => "lease_lost",
            Self::FinalizeFenced => "finalize_fenced",
            Self::Invariant(_) => "invariant_violated",
            Self::ControlDuplicate { .. } => "control_duplicate",
            Self::ControlReused => "control_id_reused",
            Self::Busy => "run_busy",
            Self::InvalidKey(_) => "invalid_key",
            Self::NotFound => "not_found",
            Self::Malformed(_) => "malformed_commit",
            Self::Backend(_) => "backend",
        }
    }
}

impl From<raisin_error::Error> for StoreError {
    fn from(e: raisin_error::Error) -> Self {
        Self::Backend(e.to_string())
    }
}

/// Durable home of runs.
#[async_trait]
pub trait AgentRunStore: Send + Sync {
    /// Create a run with its first event (seq 1), honouring subject admission
    /// and the create key.
    async fn create(
        &self,
        rec: AgentRunRecord,
        first: RunEventKind,
        create_key: Option<&str>,
    ) -> Result<CreateOutcome, StoreError>;

    /// Load a record.
    async fn load(
        &self,
        scope: &RunScope,
        run: &RunId,
    ) -> Result<Option<AgentRunRecord>, StoreError>;

    /// The one write path.
    async fn commit(&self, req: CommitRequest) -> Result<CommitOutcome, StoreError>;

    /// Events with `seq > after`, at most `limit`, in order.
    async fn read_events(
        &self,
        scope: &RunScope,
        run: &RunId,
        after: Seq,
        limit: usize,
    ) -> Result<Vec<RunEvent>, StoreError>;

    /// The stored ack of a control id, with its payload digest.
    async fn control_ack(
        &self,
        scope: &RunScope,
        run: &RunId,
        id: &str,
    ) -> Result<Option<(ControlAck, String)>, StoreError>;

    /// Seq of the completion recorded under an idempotency key.
    async fn idem_seen(
        &self,
        scope: &RunScope,
        run: &RunId,
        key: &str,
    ) -> Result<Option<Seq>, StoreError>;

    /// The newest checkpoint.
    async fn latest_checkpoint(
        &self,
        scope: &RunScope,
        run: &RunId,
    ) -> Result<Option<RunCheckpoint>, StoreError>;

    /// The domain `{state, projection}` at `rev`.
    async fn domain_state(
        &self,
        scope: &RunScope,
        run: &RunId,
        rev: u64,
    ) -> Result<Option<Value>, StoreError>;

    /// A stored result payload.
    async fn read_result(
        &self,
        scope: &RunScope,
        run: &RunId,
        key: &str,
    ) -> Result<Option<Vec<u8>>, StoreError>;

    /// Every run ever created about `subject_key` (see
    /// [`SubjectRef::key`](crate::ids::SubjectRef::key)), oldest first, at
    /// most `limit`. The live one, if any, is among them.
    async fn scan_subject(
        &self,
        scope: &RunScope,
        subject_key: &str,
        limit: usize,
    ) -> Result<Vec<RunId>, StoreError>;

    /// Runs of one scope in `status`.
    async fn scan_status(
        &self,
        scope: &RunScope,
        status: RunStatus,
        limit: usize,
    ) -> Result<Vec<RunId>, StoreError>;

    /// Checkpoint `n`. The default finds it only when it is the latest; a
    /// backend with keyed checkpoints overrides it.
    async fn read_checkpoint(
        &self,
        scope: &RunScope,
        run: &RunId,
        n: u32,
    ) -> Result<Option<RunCheckpoint>, StoreError> {
        Ok(self
            .latest_checkpoint(scope, run)
            .await?
            .filter(|c| c.checkpoint_no == n))
    }

    /// Terminal runs whose hand-back has not reached their parent (or what
    /// waits for them) yet.
    ///
    /// The default scans the terminal status indexes; a backend with an index
    /// of its own overrides it.
    async fn scan_handback_owed(
        &self,
        scope: &RunScope,
        limit: usize,
    ) -> Result<Vec<RunId>, StoreError> {
        let mut out = Vec::new();
        for status in [RunStatus::Completed, RunStatus::Failed, RunStatus::Stopped] {
            for run in self.scan_status(scope, status, usize::MAX).await? {
                let owed = self
                    .load(scope, &run)
                    .await?
                    .is_some_and(|r| r.handback_owed());
                if owed {
                    out.push(run);
                    if out.len() >= limit {
                        return Ok(out);
                    }
                }
            }
        }
        Ok(out)
    }

    /// Terminal domain runs whose reducer has not been finalized yet (the
    /// `stopped` delivery and the final projection are still owed).
    ///
    /// The default scans the terminal status indexes; a backend with an index
    /// of its own overrides it, because terminal runs accumulate forever.
    async fn scan_unfinalized(
        &self,
        scope: &RunScope,
        limit: usize,
    ) -> Result<Vec<RunId>, StoreError> {
        let mut out = Vec::new();
        for status in [RunStatus::Completed, RunStatus::Failed, RunStatus::Stopped] {
            for run in self.scan_status(scope, status, usize::MAX).await? {
                let owed = self
                    .load(scope, &run)
                    .await?
                    .and_then(|r| r.domain)
                    .is_some_and(|d| !d.finalized);
                if owed {
                    out.push(run);
                    if out.len() >= limit {
                        return Ok(out);
                    }
                }
            }
        }
        Ok(out)
    }
}

/// The checks every backend performs inside its critical section, given the
/// stored record. Returns the events with their assigned seqs.
pub fn verify_commit(
    stored: &AgentRunRecord,
    req: &CommitRequest,
) -> Result<Vec<RunEvent>, StoreError> {
    // The fence first: a driver whose lease was cleared (by a stop, a pause,
    // a takeover) must learn it lost the lease, not that it should retry.
    if let Err(code) = crate::lifecycle::check_fence(stored, &req.fence, req.now_ms) {
        return Err(match code {
            "lease_lost" => StoreError::LeaseLost {
                current_epoch: stored.lease_epoch.0,
            },
            _ => StoreError::FinalizeFenced,
        });
    }
    if stored.version != req.expected_version {
        return Err(StoreError::VersionConflict {
            current: stored.version,
        });
    }
    let n = req.events.len() as u64;
    if req.record.version != stored.version.next()
        || req.record.last_seq != Seq(stored.last_seq.0 + n)
    {
        return Err(StoreError::Malformed(
            "record version/last_seq do not follow the stored record".into(),
        ));
    }
    if req.record.run_id != stored.run_id || req.record.scope != stored.scope {
        return Err(StoreError::Malformed("record identity changed".into()));
    }
    check_invariants(Some(stored), &req.record)?;
    Ok(req
        .events
        .iter()
        .enumerate()
        .map(|(i, e)| RunEvent {
            run_id: req.run_id.clone(),
            seq: Seq(stored.last_seq.0 + 1 + i as u64),
            at_ms: req.now_ms,
            turn: e.turn,
            op_id: e.op_id.clone(),
            kind: e.kind.clone(),
        })
        .collect())
}

/// Check a control ack against an existing one (inside the critical section).
pub fn check_control_dedup(
    existing: Option<&(ControlAck, String)>,
    req: &CommitRequest,
) -> Result<(), StoreError> {
    match (existing, &req.control) {
        (Some((ack, digest)), Some((_, _, new_digest))) => {
            if digest == new_digest {
                Err(StoreError::ControlDuplicate {
                    ack: ack.as_duplicate(),
                })
            } else {
                Err(StoreError::ControlReused)
            }
        }
        _ => Ok(()),
    }
}
