//! Internal metadata structures for transaction state management

use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::{nodes::Node, translations::LocaleOverlay};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Transaction metadata
///
/// Uses `Arc<String>` for frequently cloned fields to optimize hot path performance.
/// When metadata is extracted during commit, Arc clones are very cheap (just increment refcount)
/// compared to full String allocations.
#[derive(Debug, Clone, Default)]
pub(crate) struct TransactionMetadata {
    pub(crate) tenant_id: Arc<String>,
    pub(crate) repo_id: Arc<String>,
    pub(crate) branch: Option<Arc<String>>,
    pub(crate) actor: Option<Arc<String>>,
    pub(crate) message: Option<Arc<String>>,
    pub(crate) is_manual_version: bool,
    pub(crate) manual_version_node_id: Option<Arc<String>>,
    /// Whether this is a system commit (background job, migration, etc.)
    pub(crate) is_system: bool,
    /// The single HLC timestamp used for ALL operations in this transaction
    /// This ensures atomicity - all nodes in a transaction share the same revision
    pub(crate) transaction_revision: Option<HLC>,
    /// Authentication context for permission checks
    /// When set, RLS and field-level security will be enforced
    pub(crate) auth_context: Option<Arc<AuthContext>>,
}

/// Read-your-writes cache for transactions
#[derive(Debug, Default)]
pub(crate) struct ReadCache {
    /// Cached nodes: (workspace, node_id) -> Node
    pub(crate) nodes: HashMap<(String, String), Option<Node>>,
    /// Cached paths: (workspace, path) -> node_id
    pub(crate) paths: HashMap<(String, String), Option<String>>,
    /// Cached translations: (workspace, node_id, locale) -> LocaleOverlay
    pub(crate) translations: HashMap<(String, String, String), Option<LocaleOverlay>>,
    /// Last assigned order label per (workspace, parent_id) within this transaction.
    /// Prevents sibling nodes in the same batch from getting identical fractional indexes.
    pub(crate) last_order_labels: HashMap<(String, String), String>,
}

/// Conflict detection tracking
#[derive(Debug, Default)]
pub(crate) struct ConflictTracker {
    /// Set of keys read during this transaction
    pub(crate) read_set: HashSet<Vec<u8>>,
    /// Set of keys written during this transaction
    pub(crate) write_set: HashSet<Vec<u8>>,
}
