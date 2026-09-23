// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Checkpoints: everything core owns, structured, plus a REFERENCE to the
//! write-once domain state. Entering `Paused` always writes one.

use serde::{Deserialize, Serialize};

use crate::events::{CheckpointReason, ResultRef};
use crate::ids::{CallId, RunId, Seq, SubjectRef, TurnNo};
use crate::record::{AgentRunRecord, Counters, PendingRequest, RunBudgets, RunUsage, SteerEntry};
use crate::state::RunStatus;

/// A checkpoint record, write-once under `ckpt\0{n}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunCheckpoint {
    /// Run.
    pub run_id: RunId,
    /// `counters.checkpoint` after this checkpoint.
    pub checkpoint_no: u32,
    /// Seq of its `CheckpointWritten` event.
    pub at_seq: Seq,
    /// Commit time.
    pub at_ms: u64,
    /// Why.
    pub reason: CheckpointReason,
    /// Open turn.
    pub turn: Option<TurnNo>,
    /// Core-owned state.
    pub core: CheckpointCore,
    /// Reference to `dom\0{rev}` (write-once, so no copy).
    pub domain_state_rev: Option<u64>,
    /// Contract of the domain state.
    pub domain_contract: Option<String>,
    /// Opaque locator of the last message folded in.
    pub transcript_cutoff: Option<SubjectRef>,
    /// Optional prose, never load-bearing.
    pub summary: Option<String>,
    /// Refs, not payloads.
    pub large_refs: Vec<ResultRef>,
    /// Structured compaction state (objective, decisions, pending
    /// questions, …) stored beside the run, never inline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_ref: Option<ResultRef>,
}

/// The core half of a checkpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckpointCore {
    /// Status at the checkpoint.
    pub status: RunStatus,
    /// Open requests.
    pub open: Vec<PendingRequest>,
    /// Queued steers.
    pub steer_queue: Vec<SteerEntry>,
    /// Unanswered calls.
    pub unanswered_calls: Vec<CallId>,
    /// Usage.
    pub usage: RunUsage,
    /// Budgets.
    pub budgets: RunBudgets,
    /// Counters.
    pub counters: Counters,
    /// Child links (delegation state survives compaction).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<crate::child::ChildLink>,
    /// Unacknowledged mailbox items.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mailbox: Vec<crate::child::MailboxItem>,
}

/// Build the checkpoint of `rec` (already carrying its incremented
/// `counters.checkpoint`), to be written with a `CheckpointWritten` at `at_seq`.
pub fn build(
    rec: &AgentRunRecord,
    reason: CheckpointReason,
    at_seq: Seq,
    now_ms: u64,
    summary: Option<String>,
) -> RunCheckpoint {
    RunCheckpoint {
        run_id: rec.run_id.clone(),
        checkpoint_no: rec.counters.checkpoint,
        at_seq,
        at_ms: now_ms,
        reason,
        turn: rec.current_turn,
        core: CheckpointCore {
            status: rec.state.status(),
            open: rec.state.open_requests(),
            steer_queue: rec.steer_queue.clone(),
            unanswered_calls: rec.unanswered_calls.clone(),
            usage: rec.usage,
            budgets: rec.budgets.clone(),
            counters: rec.counters,
            children: rec.children.clone(),
            mailbox: rec.mailbox.clone(),
        },
        domain_state_rev: rec.domain.as_ref().map(|d| d.state_rev),
        domain_contract: rec.domain.as_ref().map(|d| d.contract.clone()),
        transcript_cutoff: None,
        summary,
        large_refs: rec.large_results.clone(),
        structured_ref: None,
    }
}
