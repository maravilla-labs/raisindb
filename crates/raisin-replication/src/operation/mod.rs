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
        }
    }
}
