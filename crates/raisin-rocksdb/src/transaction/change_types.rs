//! Type aliases and structs for transaction change tracking
//!
//! These types replace complex nested tuples used throughout the transaction
//! commit pipeline, improving readability and maintainability.

use raisin_hlc::HLC;
use raisin_models::tree::ChangeOperation;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Tracks a single node change within a transaction.
///
/// Stored per node_id in [`ChangedNodesMap`] during transaction operations.
/// Path and node_type are preserved for proper WebSocket subscription matching
/// on delete events (where the node is no longer queryable).
#[derive(Debug, Clone)]
pub struct NodeChange {
    pub workspace: String,
    pub revision: HLC,
    pub operation: ChangeOperation,
    /// Node path, stored for event matching on delete
    pub path: Option<String>,
    /// Node type, stored for subscription filtering on delete
    pub node_type: Option<String>,
}

/// Maps node_id to its change info for a transaction.
pub type ChangedNodesMap = HashMap<String, NodeChange>;

/// Thread-safe shared state for tracking changed nodes during a transaction.
pub type SharedChangedNodes = Arc<Mutex<ChangedNodesMap>>;

/// Tracks a single translation change within a transaction.
#[derive(Debug, Clone)]
pub struct TranslationChange {
    pub workspace: String,
    pub revision: HLC,
    pub operation: ChangeOperation,
}

/// Maps (node_id, locale) to its translation change info.
pub type ChangedTranslationsMap = HashMap<(String, String), TranslationChange>;

/// Thread-safe shared state for tracking changed translations during a transaction.
pub type SharedChangedTranslations = Arc<Mutex<ChangedTranslationsMap>>;

/// Metadata extracted from a transaction for the commit phase.
///
/// Replaces the 7-tuple returned by `extract_commit_metadata`.
pub struct CommitMetadata {
    pub tenant_id: Arc<String>,
    pub repo_id: Arc<String>,
    pub branch: Option<Arc<String>>,
    pub transaction_revision: Option<HLC>,
    pub actor: Option<Arc<String>>,
    pub message: Option<Arc<String>>,
    pub is_system: bool,
}
