mod op_impl;
mod op_type;

#[cfg(test)]
mod tests;

use crate::vector_clock::VectorClock;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

pub use op_type::OpType;

/// A replayable operation that represents a single mutation in the database.
///
/// Operations are the fundamental unit of replication. They are:
/// - Commutative: Can be applied in any order (with CRDT merge rules)
/// - Idempotent: Applying the same operation twice has the same effect as applying it once
/// - Causally-ordered: Vector clocks track dependencies
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Operation {
    /// Unique identifier for this operation
    pub op_id: Uuid,

    /// Per-node monotonically increasing sequence number
    /// Used for efficient range scans in the operation log
    pub op_seq: u64,

    /// ID of the cluster node (server instance) that originated this operation
    pub cluster_node_id: String,

    /// Timestamp in milliseconds since epoch (for tie-breaking)
    pub timestamp_ms: u64,

    /// Vector clock capturing causal dependencies
    pub vector_clock: VectorClock,

    /// Tenant this operation belongs to
    pub tenant_id: String,

    /// Repository this operation belongs to
    pub repo_id: String,

    /// Branch this operation was performed on
    pub branch: String,

    /// The type and data of this operation
    pub op_type: OpType,

    /// Optional revision (Hybrid Logical Clock) associated with this operation
    #[serde(default)]
    pub revision: Option<HLC>,

    /// User or system actor that performed this operation
    pub actor: String,

    /// The non-human principal that *initiated* this operation, if any —
    /// `mcp:<slug>`, `agent:<agent-node-path>`, `trigger:<trigger-node-path>`,
    /// optionally suffixed with its origin (`agent:/a@trigger:/t`).
    ///
    /// WHY it rides on the operation: RaisinDB is a masterless
    /// eventual-consistency cluster — no replica is "lesser" than the node that
    /// first accepted the write. An audit entry written when this operation is
    /// replayed on a peer must therefore carry the SAME attribution the
    /// originating node recorded. `actor` already replicated; `agent` did not,
    /// so agent-initiated writes landed unattributed on every other node.
    ///
    /// Independent of `actor`: an autonomous workflow or AI agent legitimately
    /// sets `agent` while `actor` stays `"system"`.
    ///
    /// MIXED-VERSION SAFETY: `Operation` is persisted to the oplog and sent over
    /// TCP/HTTP with `rmp_serde::to_vec_named` / serde_json — both **map**
    /// encodings keyed by field name (see `repositories/oplog/helpers.rs` and
    /// `raisin-replication/src/tcp_protocol/message_impl.rs`; the invariant is
    /// asserted by `operation/tests.rs::compact encoding`). An older peer routes
    /// the unknown `agent` key to `IgnoredAny` and applies the op unchanged; a
    /// newer peer reading an older record gets `None` from `#[serde(default)]`.
    /// Rollback to the previous binary is therefore safe too. This is the same
    /// additive pattern already used by `revision` and `acknowledged_by`, and is
    /// explicitly NOT the positional-array trap documented on
    /// `raisin-context/src/repository/branch.rs` (`to_vec`, arity-sensitive) or
    /// on `RevisionMeta` — neither of which may gain a field this way.
    #[serde(default)]
    pub agent: Option<String>,

    /// Optional commit message (for user-initiated commits)
    pub message: Option<String>,

    /// Whether this is a system-generated operation
    pub is_system: bool,

    /// Nodes that have acknowledged receiving this operation (for GC)
    #[serde(default)]
    pub acknowledged_by: HashSet<String>,
}

/// Fully materialized node change included inside an ApplyRevision operation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReplicatedNodeChange {
    /// Node snapshot (post-apply for upserts, pre-delete for deletes)
    pub node: Node,
    /// Parent node identifier used for ordered-children indexes
    #[serde(default)]
    pub parent_id: Option<String>,
    /// How this snapshot should be applied
    pub kind: ReplicatedNodeChangeKind,
    /// Full CF order key from ORDERED_CHILDREN (e.g., "a0::node2-abc123")
    /// This preserves the exact ordering including node_id suffix for masterless conflict avoidance
    pub cf_order_key: String,
}

/// Indicates whether a replicated node snapshot represents an upsert or delete
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReplicatedNodeChangeKind {
    Upsert,
    Delete,
}

/// What an operation targets/modifies
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OperationTarget {
    Node(String),
    NodeType(String),
    Archetype(String),
    ElementType(String),
    Workspace(String),
    Branch(String),
    Tag(String),
    User(String),
    Tenant(String),
    Deployment(String),
    Repository(String),
    Permission(String),
    Identity(String),
    Session(String),
    /// A registered OAuth client.
    OAuthClient(String),
    /// A single refresh token, keyed by its hash.
    OAuthRefreshToken(String),
    /// A refresh-token rotation family (revocation).
    OAuthRefreshFamily(String),
    /// An API key.
    ApiKey(String),
    /// A single committed revision (branch + head HLC). Revisions are
    /// cumulative deltas, NOT convergent register states: every distinct
    /// ApplyRevision must be applied, so each gets its own target and is
    /// never LWW-merged with sibling revisions of the same branch.
    Revision(String),
}

impl std::fmt::Display for OperationTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Node(id) => write!(f, "node:{}", id),
            Self::NodeType(id) => write!(f, "node_type:{}", id),
            Self::Archetype(id) => write!(f, "archetype:{}", id),
            Self::ElementType(id) => write!(f, "element_type:{}", id),
            Self::Workspace(id) => write!(f, "workspace:{}", id),
            Self::Branch(id) => write!(f, "branch:{}", id),
            Self::Tag(id) => write!(f, "tag:{}", id),
            Self::User(id) => write!(f, "user:{}", id),
            Self::Tenant(id) => write!(f, "tenant:{}", id),
            Self::Deployment(id) => write!(f, "deployment:{}", id),
            Self::Repository(id) => write!(f, "repository:{}", id),
            Self::Permission(id) => write!(f, "permission:{}", id),
            Self::Identity(id) => write!(f, "identity:{}", id),
            Self::Session(id) => write!(f, "session:{}", id),
            Self::OAuthClient(id) => write!(f, "oauth_client:{}", id),
            Self::OAuthRefreshToken(id) => write!(f, "oauth_refresh_token:{}", id),
            Self::OAuthRefreshFamily(id) => write!(f, "oauth_refresh_family:{}", id),
            Self::ApiKey(id) => write!(f, "api_key:{}", id),
            Self::Revision(id) => write!(f, "revision:{}", id),
        }
    }
}
