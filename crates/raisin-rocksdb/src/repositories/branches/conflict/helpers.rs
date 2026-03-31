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
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push("nodes")
            .push(node_id)
            .build_prefix();

        let iter = self.db.prefix_iterator_cf(cf_nodes, prefix.clone());

        for item in iter {
            let (key, bytes) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            if !key.starts_with(&prefix) {
                break;
            }

            let revision = match keys::extract_revision_from_key(&key) {
                Ok(rev) => rev,
                Err(_) => continue,
            };

            if &revision > target_revision {
                continue;
            }

            // Check for tombstone
            if bytes.starts_with(b"TOMBSTONE") {
                return Ok(None);
            }

            let node: raisin_models::nodes::Node = rmp_serde::from_slice(&bytes).map_err(|e| {
                raisin_error::Error::storage(format!(
                    "Failed to deserialize node {}: {}",
                    node_id, e
                ))
            })?;

            let json = serde_json::to_value(node.properties).map_err(|e| {
                raisin_error::Error::storage(format!(
                    "Failed to convert node properties to JSON: {}",
                    e
                ))
            })?;

            return Ok(Some(json));
        }

        Ok(None)
    }

    /// Retrieve translation overlay at or before a specific revision
    pub(crate) async fn get_translation_at_revision(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        locale: &str,
        target_revision: &HLC,
        cf_translation_data: &rocksdb::ColumnFamily,
    ) -> Result<Option<serde_json::Value>> {
        let prefix = format!(
            "{}\0{}\0{}\0{}\0translations\0{}\0{}\0",
            tenant_id, repo_id, branch, workspace, node_id, locale
        )
        .into_bytes();

        let iter = self
            .db
            .prefix_iterator_cf(cf_translation_data, prefix.clone());

        for item in iter {
            let (key, bytes) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            if !key.starts_with(&prefix) {
                break;
            }

            let rev_start = prefix.len();
            if key.len() < rev_start + 16 {
                continue;
            }
            let revision = keys::decode_descending_revision(&key[rev_start..rev_start + 16])
                .map_err(|e| {
                    raisin_error::Error::storage(format!("Failed to decode revision: {}", e))
                })?;

            if &revision > target_revision {
                continue;
            }

            if bytes.starts_with(b"TOMBSTONE") {
                return Ok(None);
            }

            let overlay: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| {
                raisin_error::Error::storage(format!(
                    "Failed to deserialize translation overlay for {}:{}: {}",
                    node_id, locale, e
                ))
            })?;

            return Ok(Some(overlay));
        }

        Ok(None)
    }
}
