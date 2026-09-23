// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Changeset wire types: operations, the dry-run plan, the durable record,
//! and the committed-change receipt.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::types::{ExpectedRevision, NodeLocator, Target, WorkRoot};

/// One operation in a changeset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ChangeOp {
    /// Create a node at `path` (its parent must exist, or be created earlier
    /// in the same changeset).
    Create {
        /// Workspace (default: the first root's).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace: Option<String>,
        /// Full path of the new node (relative or absolute).
        path: String,
        /// Node type.
        node_type: String,
        /// Archetype.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        archetype: Option<String>,
        /// Properties.
        #[serde(default)]
        properties: Map<String, Value>,
    },
    /// Set and/or unset top-level properties.
    Patch {
        /// The node.
        target: Target,
        /// Refuse (as a conflict) unless the node is at this revision.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_revision: Option<ExpectedRevision>,
        /// Properties to set.
        #[serde(default)]
        set: Map<String, Value>,
        /// Properties to remove.
        #[serde(default)]
        unset: Vec<String>,
        /// New archetype.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        archetype: Option<String>,
    },
    /// Move a node (with its whole subtree) under another parent, keeping
    /// its id, children, references and permissions.
    Move {
        /// The node.
        target: Target,
        /// The new parent (`{path: "/"}` for the workspace root).
        to_parent: Target,
        /// Optional new name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        new_name: Option<String>,
        /// Refuse unless the node is at this revision.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_revision: Option<ExpectedRevision>,
    },
    /// Rename in place (a move to the same parent).
    Rename {
        /// The node.
        target: Target,
        /// New name.
        new_name: String,
        /// Refuse unless the node is at this revision.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_revision: Option<ExpectedRevision>,
    },
    /// Deep-copy a node under another parent (new ids).
    Copy {
        /// What to copy.
        source: Target,
        /// The new parent.
        to_parent: Target,
        /// Optional name for the copy.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        new_name: Option<String>,
    },
    /// Delete a node; `recursive` is required when it has children.
    Delete {
        /// The node.
        target: Target,
        /// Refuse unless the node is at this revision.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_revision: Option<ExpectedRevision>,
        /// Delete the subtree too.
        #[serde(default)]
        recursive: bool,
    },
}

/// A changeset request (propose, dry-run, or apply).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChangesetRequest {
    /// The working roots (and grant). Required.
    pub roots: Vec<WorkRoot>,
    /// Operations, applied in order, atomically.
    pub ops: Vec<ChangeOp>,
    /// Replaying the same key never applies twice; it returns the receipt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    /// Commit message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Index of the op whose node is the result's primary artifact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<usize>,
    /// Artifact kind reported in tool results (default `node`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// What a planned op will do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpAction {
    /// Creates a node.
    Created,
    /// Updates properties.
    Updated,
    /// Moves (or renames) a subtree.
    Moved,
    /// Copies a subtree.
    Copied,
    /// Deletes a node (and subtree).
    Deleted,
}

/// A node that references a moved or deleted node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Referrer {
    /// The referrer's workspace.
    pub workspace: String,
    /// The referrer's id.
    pub node_id: String,
    /// The referrer's path, when readable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The property holding the reference.
    pub property: String,
    /// The referenced node's id.
    pub target_id: String,
}

/// A descendant carried by a move.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MovedEntry {
    /// Path before.
    pub from_path: String,
    /// Where it is (or will be).
    pub to: NodeLocator,
}

/// One op, resolved and validated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannedOp {
    /// Position in the changeset.
    pub index: usize,
    /// What it does.
    pub action: OpAction,
    /// Workspace.
    pub workspace: String,
    /// The node's id (pre-assigned for a create; `None` for a copy's new root).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    /// The node as read at plan time (with its revision).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<NodeLocator>,
    /// Its path after the op.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_path: Option<String>,
    /// Node type.
    pub node_type: String,
    /// Keys the op changes (patch/create).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed_properties: Vec<String>,
    /// Descendants moved with the node.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub moved_descendants: Vec<MovedEntry>,
    /// Descendants a delete removes, or a copy duplicates (source side).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub descendants: Vec<NodeLocator>,
    /// Nodes referencing a moved/deleted node.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub referrers: Vec<Referrer>,
}

/// Why an op cannot commit as reviewed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conflict {
    /// Op index.
    pub index: usize,
    /// `stale_revision` | `missing` | `exists` | `not_empty` | `invalid_move`.
    pub code: String,
    /// Explanation.
    pub message: String,
    /// The expected revision value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    /// The node as it is now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual: Option<NodeLocator>,
}

/// A dry run: every target resolved, authorized and checked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangesetPlan {
    /// Resolved ops.
    pub ops: Vec<PlannedOp>,
    /// Conflicts; a plan with any cannot commit.
    pub conflicts: Vec<Conflict>,
    /// sha256 over the ops and the revisions they were planned against.
    /// Approving a changeset means approving THIS digest.
    pub digest: String,
}

/// Lifecycle of a changeset record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangesetStatus {
    /// Planned, reviewable, not applied.
    Proposed,
    /// Applied atomically.
    Committed,
    /// Abandoned.
    Discarded,
}

/// One op's committed effect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpReceipt {
    /// Op index.
    pub index: usize,
    /// What it did.
    pub action: OpAction,
    /// The node before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old: Option<NodeLocator>,
    /// The node after.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new: Option<NodeLocator>,
    /// Changed property keys.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed_properties: Vec<String>,
    /// Descendants created (copy).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub created_descendants: Vec<NodeLocator>,
    /// Descendants deleted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deleted_descendants: Vec<NodeLocator>,
    /// Descendants moved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub moved_descendants: Vec<MovedEntry>,
    /// References to a moved node. They are keyed by the node's stable id and
    /// stay valid; their denormalized path is rewritten by the platform's
    /// reference-retarget job.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rewritten_references: Vec<Referrer>,
}

/// Exactly what committed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Receipt {
    /// The changeset.
    pub changeset_id: String,
    /// Repository.
    pub repository: String,
    /// Branch.
    pub branch: String,
    /// The branch head after the commit (HLC).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub committed_revision: Option<String>,
    /// Per-op effects.
    pub ops: Vec<OpReceipt>,
    /// True when this answer is a replay of an earlier commit.
    #[serde(default)]
    pub replayed: bool,
}

/// The durable changeset record (a node in `raisin:system`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangesetRecord {
    /// Id (derived from the idempotency key when one is given).
    pub changeset_id: String,
    /// Idempotency key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    /// Who proposed it.
    pub owner: String,
    /// Repository.
    pub repository: String,
    /// Branch.
    pub branch: String,
    /// The request.
    pub request: ChangesetRequest,
    /// Status.
    pub status: ChangesetStatus,
    /// The plan as last reviewed.
    pub plan: ChangesetPlan,
    /// When proposed (RFC 3339).
    pub created_at: String,
    /// Who committed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub committed_by: Option<String>,
    /// The receipt, once committed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<Receipt>,
}

/// Outcome of a commit (or apply).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum CommitOutcome {
    /// Everything applied atomically.
    Committed {
        /// What committed.
        receipt: Receipt,
    },
    /// Nothing applied: the plan no longer holds.
    Conflict {
        /// The changeset (still `proposed`).
        changeset_id: String,
        /// Why.
        conflicts: Vec<Conflict>,
        /// The digest of the current (conflicting) plan.
        digest: String,
    },
}
