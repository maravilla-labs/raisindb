//! Revision and time-travel helpers for NodeService
//!
//! Contains methods for working with historical revisions and time-travel queries.

use raisin_hlc::HLC;
use raisin_storage::{transactional::TransactionalStorage, Storage};

use super::NodeService;

impl<S: Storage + TransactionalStorage> NodeService<S> {
    /// Set a specific revision for time-travel reads.
    ///
    /// When a revision is set, all read operations (get, list, query) will
    /// return nodes as they existed at that revision. Write operations are
    /// not allowed when viewing a historic revision.
    ///
    /// # Arguments
    ///
    /// * `revision` - The revision number to view (e.g., 42)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // View repository state at specific HLC revision
    /// let historic = node_service.at_revision(HLC::new(42, 0));
    /// let old_node = historic.get("node-id").await?;
    /// ```
    pub fn at_revision(mut self, revision: HLC) -> Self {
        self.revision = Some(revision);
        self
    }

    /// Check if this service is viewing a historic revision (not HEAD).
    ///
    /// Returns true if a specific revision is set, false if viewing HEAD.
    pub fn is_historic_view(&self) -> bool {
        self.revision.is_some()
    }

    /// Get the current revision being viewed (if any).
    ///
    /// Returns Some(revision) if viewing a specific revision, None if viewing HEAD.
    pub fn current_revision(&self) -> Option<HLC> {
        self.revision
    }
}
