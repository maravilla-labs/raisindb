//! Translation carry-over (node-level and block-level) for cross-branch copy.

use super::super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_models::translations::TranslationMeta;
use raisin_models::tree::ChangeOperation;
use raisin_storage::NodeChangeInfo;
use rocksdb::WriteBatch;

impl NodeRepositoryImpl {
    /// Copy the latest node-level and block-level translations of `node_id`
    /// from the source branch onto the target branch (same node id), writing
    /// TRANSLATION_DATA / TRANSLATION_INDEX / meta / snapshot entries at the
    /// shared revision — mirrors the single-branch tree-copy behavior.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn copy_translations_to_batch(
        &self,
        batch: &mut WriteBatch,
        node_id: &str,
        tenant_id: &str,
        repo_id: &str,
        source_branch: &str,
        target_branch: &str,
        workspace: &str,
        revision: &HLC,
        now: chrono::DateTime<chrono::Utc>,
        actor: &str,
        message: &str,
        is_system: bool,
        operation: ChangeOperation,
        change_infos: &mut Vec<NodeChangeInfo>,
    ) -> Result<()> {
        let cf_translation_data = cf_handle(&self.db, cf::TRANSLATION_DATA)?;
        let cf_translation_index = cf_handle(&self.db, cf::TRANSLATION_INDEX)?;
        let cf_block_translations = cf_handle(&self.db, cf::BLOCK_TRANSLATIONS)?;
        let cf_revisions = cf_handle(&self.db, cf::REVISIONS)?;

        let node_translations = self.collect_node_translations_for_copy(
            tenant_id,
            repo_id,
            source_branch,
            workspace,
            node_id,
        )?;

        for (locale, overlay, parent_translation_revision) in node_translations {
            let overlay_bytes = serde_json::to_vec(&overlay).map_err(|e| {
                raisin_error::Error::storage(format!(
                    "Failed to serialize translation overlay for locale {}: {}",
                    locale.as_str(),
                    e
                ))
            })?;
            let data_key = Self::translation_data_key(
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                node_id,
                locale.as_str(),
                revision,
            );
            batch.put_cf(&cf_translation_data, data_key, overlay_bytes.clone());

            let index_key =
                Self::translation_index_key(tenant_id, repo_id, locale.as_str(), revision, node_id);
            batch.put_cf(&cf_translation_index, index_key, b"");

            let translation_meta = TranslationMeta {
                locale: locale.clone(),
                revision: *revision,
                parent_revision: parent_translation_revision,
                timestamp: now,
                actor: actor.to_string(),
                message: message.to_string(),
                is_system,
            };
            let meta_bytes = serde_json::to_vec(&translation_meta).map_err(|e| {
                raisin_error::Error::storage(format!(
                    "Failed to serialize TranslationMeta for locale {}: {}",
                    locale.as_str(),
                    e
                ))
            })?;
            let meta_key = Self::translation_meta_key(
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                node_id,
                locale.as_str(),
                revision,
            );
            batch.put_cf(&cf_revisions, meta_key, meta_bytes);

            let snapshot_key = keys::translation_snapshot_key(
                tenant_id,
                repo_id,
                node_id,
                locale.as_str(),
                revision,
            );
            batch.put_cf(&cf_revisions, snapshot_key, overlay_bytes.clone());

            change_infos.push(NodeChangeInfo {
                node_id: node_id.to_string(),
                workspace: workspace.to_string(),
                operation,
                translation_locale: Some(locale.as_str().to_string()),
            });
        }

        let block_translations = self.collect_block_translations_for_copy(
            tenant_id,
            repo_id,
            source_branch,
            workspace,
            node_id,
        )?;

        for (block_uuid, locale, overlay, _parent_revision) in block_translations {
            let overlay_bytes = serde_json::to_vec(&overlay).map_err(|e| {
                raisin_error::Error::storage(format!(
                    "Failed to serialize block translation overlay {}::{}: {}",
                    locale.as_str(),
                    block_uuid,
                    e
                ))
            })?;

            let block_key = Self::block_translation_key(
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                node_id,
                &block_uuid,
                locale.as_str(),
                revision,
            );
            batch.put_cf(&cf_block_translations, block_key, overlay_bytes.clone());

            let snapshot_key = keys::translation_snapshot_key(
                tenant_id,
                repo_id,
                node_id,
                &format!("{}::{}", locale.as_str(), block_uuid),
                revision,
            );
            batch.put_cf(&cf_revisions, snapshot_key, overlay_bytes.clone());

            change_infos.push(NodeChangeInfo {
                node_id: node_id.to_string(),
                workspace: workspace.to_string(),
                operation,
                translation_locale: Some(format!("{}::{}", locale.as_str(), block_uuid)),
            });
        }

        Ok(())
    }
}
