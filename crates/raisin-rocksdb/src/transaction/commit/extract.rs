//! Data extraction methods for the commit phase

use super::super::RocksDBTransaction;
use crate::transaction::change_types::{ChangedNodesMap, ChangedTranslationsMap, CommitMetadata};
use raisin_error::Result;
use rocksdb::WriteBatch;

impl RocksDBTransaction {
    /// Extract changed nodes from the transaction for snapshot creation and event emission
    pub(in crate::transaction) fn extract_changed_nodes(&self) -> Result<ChangedNodesMap> {
        let changed = self.changed_nodes.lock().map_err(|e| {
            raisin_error::Error::storage(format!("Failed to lock changed_nodes: {}", e))
        })?;
        Ok(changed.clone())
    }

    /// Extract changed translations from the transaction for snapshot creation
    pub(in crate::transaction) fn extract_changed_translations(
        &self,
    ) -> Result<ChangedTranslationsMap> {
        let changed = self.changed_translations.lock().map_err(|e| {
            raisin_error::Error::storage(format!("Failed to lock changed_translations: {}", e))
        })?;
        Ok(changed.clone())
    }

    /// Extract metadata needed for commit operations
    ///
    /// Returns Arc-wrapped strings for cheap cloning. Use `as_ref()` to get `&str` when needed.
    pub(in crate::transaction) fn extract_commit_metadata(&self) -> Result<CommitMetadata> {
        let metadata = self
            .metadata
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Failed to lock metadata: {}", e)))?;
        Ok(CommitMetadata {
            tenant_id: metadata.tenant_id.clone(),
            repo_id: metadata.repo_id.clone(),
            branch: metadata.branch.clone(),
            transaction_revision: metadata.transaction_revision,
            actor: metadata.actor.clone(),
            message: metadata.message.clone(),
            is_system: metadata.is_system,
        })
    }

    /// Extract and replace the write batch
    pub(in crate::transaction) fn extract_batch(&self) -> Result<WriteBatch> {
        let mut batch = self
            .batch
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Failed to lock batch: {}", e)))?;
        Ok(std::mem::take(&mut *batch))
    }

    /// Build NodeChangeInfo list from changed nodes and translations
    pub(in crate::transaction) fn build_node_change_infos(
        &self,
        changed_nodes: &ChangedNodesMap,
        changed_translations: &ChangedTranslationsMap,
    ) -> Vec<raisin_storage::NodeChangeInfo> {
        let mut changed_node_infos: Vec<raisin_storage::NodeChangeInfo> = changed_nodes
            .iter()
            .map(|(node_id, change)| raisin_storage::NodeChangeInfo {
                node_id: node_id.clone(),
                workspace: change.workspace.clone(),
                operation: change.operation,
                translation_locale: None,
            })
            .collect();

        // Add translation changes
        for ((node_id, locale), change) in changed_translations.iter() {
            changed_node_infos.push(raisin_storage::NodeChangeInfo {
                node_id: node_id.clone(),
                workspace: change.workspace.clone(),
                operation: change.operation,
                translation_locale: Some(locale.clone()),
            });
        }

        changed_node_infos
    }
}
