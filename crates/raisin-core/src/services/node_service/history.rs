//! Revision-history listing for NodeService (git-style "file history").
//!
//! Exposes the MVCC revision walk as a lightweight, newest-first list of
//! [`NodeRevisionEntry`]s. Each entry carries the `revision` (usable with
//! `at_revision()` reads to fetch the full snapshot), plus who/when and whether
//! the node was deleted at that revision.

use raisin_error::Result;
use raisin_models::nodes::NodeRevisionEntry;
use raisin_storage::{transactional::TransactionalStorage, NodeRepository, Storage};

use super::NodeService;

impl<S: Storage + TransactionalStorage> NodeService<S> {
    /// List a node's revision history (newest first).
    ///
    /// Read permission is enforced the same way as `get()`: the caller must be
    /// able to read the node at HEAD. If the node does not exist (or the caller
    /// lacks read permission), an empty history is returned rather than leaking
    /// the existence of revisions.
    ///
    /// # Arguments
    ///
    /// * `id` - The node ID.
    /// * `limit` - Optional cap on the number of revisions returned.
    pub async fn history(&self, id: &str, limit: Option<usize>) -> Result<Vec<NodeRevisionEntry>> {
        // Enforce read access using the existing get() path (workspace-delta +
        // RLS aware). If the node isn't currently readable, don't expose history.
        if self.get(id).await?.is_none() {
            return Ok(Vec::new());
        }

        self.storage
            .nodes()
            .get_node_history(self.scope(), id, limit)
            .await
    }

    /// List a node's revision history by path (newest first).
    ///
    /// Resolves the path to an ID (RLS aware) and delegates to [`Self::history`].
    pub async fn history_by_path(
        &self,
        path: &str,
        limit: Option<usize>,
    ) -> Result<Vec<NodeRevisionEntry>> {
        match self.get_by_path(path).await? {
            Some(node) => {
                self.storage
                    .nodes()
                    .get_node_history(self.scope(), &node.id, limit)
                    .await
            }
            None => Ok(Vec::new()),
        }
    }
}
