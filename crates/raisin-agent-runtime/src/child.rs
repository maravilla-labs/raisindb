// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Child runs: the typed objective a parent hands a child, the link the parent
//! keeps, and the durable mailbox the child's hand-back lands in.
//!
//! A child is an ordinary run (its own aggregate, lease, log and budgets) with
//! `parent_run_id`, `root_run_id` and `depth` set. Nothing crosses aggregates
//! atomically; every cross-run effect is an idempotent commit on ONE run,
//! retried by the job queue and the sweeper until it lands:
//!
//! 1. **spawn** — the parent commits `ChildSpawned` + a [`ChildLink`] (the
//!    admission and the budget reservation), then the child is created under
//!    the id the link names. A crash in between is repaired from the stored
//!    spawn request.
//! 2. **hand-back** — the child's terminal commit leaves it *hand-back owed*;
//!    the parent commits `ChildHandback` + a [`MailboxItem`] (deduplicated by
//!    the link), then the child commits `HandbackDelivered`.
//! 3. **cascade** — a terminal parent stops every live child with a
//!    deterministic control id.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::RunId;
use crate::record::{BudgetPolicy, RunBudgets, RunUsage};
use crate::state::RunStatus;

/// Deepest allowed child (the root is depth 0).
pub const MAX_DEPTH: u8 = 4;
/// Children one run may ever spawn (keeps the record small).
pub const MAX_CHILDREN_PER_RUN: usize = 64;
/// Unacknowledged mailbox items one run may hold.
pub const MAILBOX_LIMIT: usize = 256;
/// Items a `recent_turns` context may carry.
pub const MAX_CONTEXT_ITEMS: usize = 50;
/// Bytes an inline context or message may carry.
pub const MAX_INLINE_BYTES: usize = 64 * 1024;

/// What context a child starts with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ContextSelection {
    /// Nothing but the objective.
    #[default]
    None,
    /// The last `turns` turns of the parent's transcript, as the caller
    /// selected them (core does not own transcripts).
    RecentTurns {
        /// Bound.
        turns: u32,
        /// The items (at most `turns`, at most [`MAX_CONTEXT_ITEMS`]).
        #[serde(default)]
        items: Vec<Value>,
    },
    /// A structured snapshot: a REFERENCE to one of the parent's checkpoints
    /// (default: its latest), optionally with caller-selected data.
    Snapshot {
        /// Parent checkpoint; `None` = the latest at spawn time.
        #[serde(default)]
        checkpoint_no: Option<u32>,
        /// Extra structured data (bounded).
        #[serde(default)]
        data: Option<Value>,
    },
}

/// An artifact the child is expected to produce.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpectedArtifact {
    /// Kind (`node`, `function`, `appdef`, …) — opaque to core.
    pub kind: String,
    /// Where it should be, when known.
    #[serde(default)]
    pub locator: Option<Value>,
    /// What it is.
    #[serde(default)]
    pub description: Option<String>,
    /// A hand-back without it violates the contract.
    #[serde(default = "yes")]
    pub required: bool,
}

/// A check the hand-back must report as passed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcceptanceCheck {
    /// Id the child reports against (`detail.checks[].id`).
    pub id: String,
    /// What it verifies.
    #[serde(default)]
    pub description: Option<String>,
    /// Machine-readable check (opaque to core).
    #[serde(default)]
    pub check: Option<Value>,
    /// A hand-back that does not report it passed violates the contract.
    #[serde(default = "yes")]
    pub required: bool,
}

fn yes() -> bool {
    true
}

/// What the child must hand back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct HandBackContract {
    /// Top-level keys the outcome detail must carry.
    #[serde(default)]
    pub required_fields: Vec<String>,
    /// A schema for the detail (carried to the child; opaque to core).
    #[serde(default)]
    pub schema: Option<Value>,
    /// Free-form description of the expected hand-back.
    #[serde(default)]
    pub description: Option<String>,
}

/// The typed objective of a child.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChildObjective {
    /// Short title.
    pub title: String,
    /// What to do.
    #[serde(default)]
    pub instructions: String,
    /// Selected context.
    #[serde(default)]
    pub context: ContextSelection,
    /// Tool function paths the child may call (`*` suffix = prefix). Empty =
    /// no restriction beyond the principal's own rights.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Write roots (`{workspace, path, ops?}`), narrowed to the parent's own
    /// grant; they become the child's `executor_config.node_dev.roots`.
    /// `None` inherits the parent's grant; `[]` means no writes.
    #[serde(default)]
    pub allowed_writes: Option<Vec<Value>>,
    /// Artifacts the hand-back must name.
    #[serde(default)]
    pub expected_artifacts: Vec<ExpectedArtifact>,
    /// Checks the hand-back must report.
    #[serde(default)]
    pub acceptance_checks: Vec<AcceptanceCheck>,
    /// Terminal hand-back contract.
    #[serde(default)]
    pub hand_back: HandBackContract,
}

/// What a child record carries about its delegation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Delegation {
    /// The objective.
    pub objective: ChildObjective,
    /// Parent's child number.
    pub child_no: u32,
    /// Spawn idempotency key.
    #[serde(default)]
    pub spawn_key: Option<String>,
    /// The parent's hand-back has been committed.
    #[serde(default)]
    pub handback_delivered: bool,
}

/// A parent's view of one child.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChildLink {
    /// 1-based, per parent.
    pub child_no: u32,
    /// The child.
    pub run_id: RunId,
    /// Title of its objective.
    pub title: String,
    /// Spawn idempotency key.
    #[serde(default)]
    pub spawn_key: Option<String>,
    /// Last status the parent committed (live until the hand-back).
    pub status: RunStatus,
    /// Budgets reserved from the parent while the child is live.
    pub reserved: RunBudgets,
    /// The hand-back is in the mailbox.
    #[serde(default)]
    pub delivered: bool,
    /// Where the hand-back envelope is stored (parent results).
    #[serde(default)]
    pub result_key: Option<String>,
    /// The child's tree usage at hand-back.
    #[serde(default)]
    pub usage: Option<RunUsage>,
}

impl ChildLink {
    /// Live = not handed back yet.
    pub fn is_live(&self) -> bool {
        !self.delivered
    }

    /// The resume key a waiting tool uses to wait for this child.
    pub fn resume_key(&self) -> String {
        resume_key(&self.run_id)
    }
}

/// `child:{run_id}` — the resume key of a tool waiting for a child.
pub fn resume_key(child: &RunId) -> String {
    format!("child:{child}")
}

/// What a mailbox item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailKind {
    /// A child's terminal hand-back.
    Completion,
    /// A message a child posted to its parent.
    Message,
}

/// One unacknowledged mailbox item. The payload lives in the run's results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MailboxItem {
    /// 1-based, per run.
    pub mail_no: u64,
    /// Kind.
    pub kind: MailKind,
    /// The child it came from.
    pub from_run: RunId,
    /// Seq of the event that delivered it.
    pub seq: u64,
    /// Result key of the payload.
    pub result_key: String,
    /// Child status (completions).
    #[serde(default)]
    pub status: Option<RunStatus>,
}

/// A spawn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpawnChild {
    /// The typed objective.
    pub objective: ChildObjective,
    /// Budgets requested (clamped to what the parent can spare).
    #[serde(default)]
    pub budgets: RunBudgets,
    /// Budget policy of the child (default: fail — a child paused on budget
    /// would silently stall its parent).
    #[serde(default)]
    pub on_exceeded: Option<BudgetPolicy>,
    /// Spawn idempotency key (a retry returns the same child).
    #[serde(default)]
    pub spawn_key: Option<String>,
    /// Subject override (default: `<parent path>#child-<n>`).
    #[serde(default)]
    pub subject: Option<crate::ids::SubjectRef>,
    /// Agent the child runs as (on behalf of the parent's user).
    #[serde(default)]
    pub as_agent: Option<String>,
    /// Opaque agent reference (default: the parent's).
    #[serde(default)]
    pub agent_ref: Option<String>,
    /// Extra input.
    #[serde(default)]
    pub input: Value,
    /// Executor configuration merged over the parent's.
    #[serde(default)]
    pub executor_config: Option<Value>,
    /// Server-driven: the bound reducer. `None` = client-driven (the parent,
    /// or whoever it hands the child to, drives it over the API). Transports
    /// bind it from a function path; it never comes from a request body.
    #[serde(skip)]
    pub reducer: Option<crate::domain::ReducerRef>,
}

/// Whether `tool` is allowed by `allowed` (empty = everything).
pub fn tool_allowed(allowed: &[String], tool: &str) -> bool {
    allowed.is_empty()
        || allowed.iter().any(|a| match a.strip_suffix('*') {
            Some(prefix) => tool.starts_with(prefix),
            None => a == tool,
        })
}
