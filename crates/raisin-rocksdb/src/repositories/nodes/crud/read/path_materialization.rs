//! Decoding a node blob with its path, for the repository read path.
//!
//! A thin delegate: the ONE decoder and the ONE path read rule live in
//! [`crate::mvcc_read`] (`deserialize_node_with_path_as`, `materialize_path`).
//! This file only binds them to the repository's database handle.

use super::super::super::storage_node::PropertiesMode;
use super::super::super::NodeRepositoryImpl;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;

impl NodeRepositoryImpl {
    /// Decode `bytes` (the version stored at `blob_revision`) with its path as
    /// of `read_at`, by the Phase 10 read rule: the newer of `NODE_PATH` and
    /// a legacy full-`Node` blob's embedded path. See
    /// [`crate::mvcc_read::deserialize_node_with_path`].
    #[allow(clippy::too_many_arguments)]
    pub(in crate::repositories::nodes) fn deserialize_node_with_path(
        &self,
        bytes: &[u8],
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        read_at: &HLC,
        blob_revision: &HLC,
    ) -> Result<Node> {
        self.deserialize_node_with_path_as(
            bytes,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            read_at,
            blob_revision,
            PropertiesMode::Load,
        )
    }

    /// [`Self::deserialize_node_with_path`], optionally without decoding the
    /// properties (`PropertiesMode::Skip` returns an empty map).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::repositories::nodes) fn deserialize_node_with_path_as(
        &self,
        bytes: &[u8],
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        read_at: &HLC,
        blob_revision: &HLC,
        mode: PropertiesMode,
    ) -> Result<Node> {
        crate::mvcc_read::deserialize_node_with_path_as(
            &self.db,
            bytes,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            read_at,
            blob_revision,
            mode,
        )
    }
}
