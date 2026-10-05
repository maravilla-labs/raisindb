//! Helper methods for retrieving node/translation properties at specific revisions.

use crate::keys;
use raisin_error::Result;
use raisin_hlc::HLC;

use super::super::BranchRepositoryImpl;

impl BranchRepositoryImpl {
    /// Retrieve base, target, and source properties for a conflict
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn retrieve_conflict_properties(
        &self,
        tenant_id: &str,
        repo_id: &str,
        target_branch: &str,
        source_branch: &str,
        target_workspace: &str,
        source_workspace: &str,
        node_id: &str,
        translation_locale: Option<&str>,
        common_ancestor: &HLC,
        target_head: &HLC,
        source_head: &HLC,
        cf_nodes: &rocksdb::ColumnFamily,
        cf_translation_data: &rocksdb::ColumnFamily,
    ) -> Result<(
        Option<serde_json::Value>,
        Option<serde_json::Value>,
        Option<serde_json::Value>,
    )> {
        if let Some(locale) = translation_locale {
            // Translation conflict - get overlays
            Ok((
                if *common_ancestor != HLC::new(0, 0) {
                    self.get_translation_at_revision(
                        tenant_id,
                        repo_id,
                        target_branch,
                        target_workspace,
                        node_id,
                        locale,
                        common_ancestor,
                        cf_translation_data,
                    )
                    .await?
                } else {
                    None
                },
                self.get_translation_at_revision(
                    tenant_id,
                    repo_id,
                    target_branch,
                    target_workspace,
                    node_id,
                    locale,
                    target_head,
                    cf_translation_data,
                )
                .await?,
                self.get_translation_at_revision(
                    tenant_id,
                    repo_id,
                    source_branch,
                    source_workspace,
                    node_id,
                    locale,
                    source_head,
                    cf_translation_data,
                )
                .await?,
            ))
        } else {
            // Base node conflict - get node properties
            Ok((
                if *common_ancestor != HLC::new(0, 0) {
                    self.get_node_properties_at_revision(
                        tenant_id,
                        repo_id,
                        target_branch,
                        target_workspace,
                        node_id,
                        common_ancestor,
                        cf_nodes,
                    )
                    .await?
                } else {
                    None
                },
                self.get_node_properties_at_revision(
                    tenant_id,
                    repo_id,
                    target_branch,
                    target_workspace,
                    node_id,
                    target_head,
                    cf_nodes,
                )
                .await?,
                self.get_node_properties_at_revision(
                    tenant_id,
                    repo_id,
                    source_branch,
                    source_workspace,
                    node_id,
                    source_head,
                    cf_nodes,
                )
                .await?,
            ))
        }
    }

    /// Resolve the path for a conflict node
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn resolve_conflict_path(
        &self,
        tenant_id: &str,
        repo_id: &str,
        target_branch: &str,
        target_workspace: &str,
        node_id: &str,
        translation_locale: Option<&String>,
        target_head: &HLC,
        target_properties: &Option<serde_json::Value>,
        source_properties: &Option<serde_json::Value>,
        cf_nodes: &rocksdb::ColumnFamily,
    ) -> Result<String> {
        let is_translation = translation_locale.is_some();

        if is_translation {
            // For translation conflicts, fetch the base node's path
            let node_props = self
                .get_node_properties_at_revision(
                    tenant_id,
                    repo_id,
                    target_branch,
                    target_workspace,
                    node_id,
                    target_head,
                    cf_nodes,
                )
                .await?;
            Ok(node_props
                .as_ref()
                .and_then(|p| p.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or(node_id)
                .to_string())
        } else if let Some(ref props) = target_properties {
            Ok(props
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string())
        } else if let Some(ref props) = source_properties {
            Ok(props
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string())
        } else {
            Ok(node_id.to_string())
        }
    }

    /// Retrieve node properties at or before a specific revision
    ///
    /// Uses prefix scan to find the latest version of the node at or before target_revision.
    pub(crate) async fn get_node_properties_at_revision(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        target_revision: &HLC,
        cf_nodes: &rocksdb::ColumnFamily,
    ) -> Result<Option<serde_json::Value>> {
        // The newest version at or below the revision, by one seek. A
        // tombstone is `T` (`keys::is_tombstone_value`): this reader used to
        // test for a `TOMBSTONE` prefix no writer produces, so a deleted node
        // reached the decoder and failed the whole conflict listing.
        let prefix = keys::node_key_prefix(tenant_id, repo_id, branch, workspace, node_id);
        let Some((blob_revision, bytes)) = crate::mvcc_read::newest_at_or_before(
            &self.db,
            cf_nodes,
            &prefix,
            Some(target_revision),
        )?
        else {
            return Ok(None);
        };
        if keys::is_tombstone_value(&bytes) {
            return Ok(None);
        }

        let node = crate::mvcc_read::deserialize_node_with_path(
            &self.db,
            &bytes,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            target_revision,
            &blob_revision,
        )
        .map_err(|e| {
            raisin_error::Error::storage(format!("Failed to deserialize node {}: {}", node_id, e))
        })?;

        let json = serde_json::to_value(node.properties).map_err(|e| {
            raisin_error::Error::storage(format!(
                "Failed to convert node properties to JSON: {}",
                e
            ))
        })?;

        Ok(Some(json))
    }

    /// Retrieve translation overlay at or before a specific revision —
    /// through the one translation reader (`{locale}::{block_uuid}` names a
    /// block overlay, the revision-meta convention).
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn get_translation_at_revision(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        locale: &str,
        target_revision: &HLC,
        _cf_translation_data: &rocksdb::ColumnFamily,
    ) -> Result<Option<serde_json::Value>> {
        let version = match locale.split_once("::") {
            Some((locale, block_uuid)) => crate::translation_read::read_block_version(
                &self.db,
                tenant_id,
                repo_id,
                branch,
                workspace,
                node_id,
                block_uuid,
                locale,
                Some(target_revision),
            )?,
            None => crate::translation_read::read_version(
                &self.db,
                tenant_id,
                repo_id,
                branch,
                workspace,
                node_id,
                locale,
                Some(target_revision),
            )?,
        };
        match version.and_then(|v| v.overlay) {
            Some(overlay) => serde_json::to_value(&overlay).map(Some).map_err(|e| {
                raisin_error::Error::storage(format!(
                    "Failed to convert translation overlay for {}:{}: {}",
                    node_id, locale, e
                ))
            }),
            None => Ok(None),
        }
    }
}
