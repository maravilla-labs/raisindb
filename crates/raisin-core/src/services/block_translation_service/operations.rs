//! Block translation operations: update, batch update, orphan handling, delete, get.

use chrono::Utc;
use raisin_error::{Error, Result};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::translations::{JsonPointer, LocaleCode, LocaleOverlay, TranslationMeta};
use raisin_storage::TranslationRepository;
use std::collections::{HashMap, HashSet};

use super::{
    BatchBlockTranslationUpdate, BatchBlockUpdateResult, BlockTranslationService,
    BlockTranslationUpdateResult,
};

impl<R: TranslationRepository> BlockTranslationService<R> {
    /// Update a translation for a single block within a Composite.
    ///
    /// The block is identified by its stable UUID, which persists across
    /// reordering operations. This ensures translations follow blocks
    /// when they move within the container.
    pub async fn update_block_translation(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        block_uuid: &str,
        locale: &LocaleCode,
        translations: HashMap<JsonPointer, PropertyValue>,
        actor: &str,
        message: Option<String>,
        revision: raisin_hlc::HLC,
    ) -> Result<BlockTranslationUpdateResult> {
        // Validate that we have at least one translation
        if translations.is_empty() {
            return Err(Error::Validation(
                "Block translation update must contain at least one property".to_string(),
            ));
        }

        // Create the translation overlay
        let overlay = LocaleOverlay::Properties { data: translations };

        // Create translation metadata
        let timestamp = Utc::now();
        let meta = TranslationMeta {
            locale: locale.clone(),
            revision,
            parent_revision: None,
            timestamp,
            actor: actor.to_string(),
            message: message.unwrap_or_else(|| {
                format!(
                    "Update {} translation for block {}",
                    locale.as_str(),
                    block_uuid
                )
            }),
            is_system: false,
        };

        // Store the block translation
        self.repository
            .store_block_translation(
                tenant_id, repo_id, branch, workspace, node_id, block_uuid, locale, &overlay, &meta,
            )
            .await?;

        Ok(BlockTranslationUpdateResult {
            block_uuid: block_uuid.to_string(),
            node_id: node_id.to_string(),
            locale: locale.clone(),
            revision,
            timestamp,
        })
    }

    /// Update multiple block translations in a batch operation.
    ///
    /// Applies all block translation updates, continuing on errors and
    /// collecting both successful and failed operations.
    ///
    /// Each update in the batch gets its own allocated HLC revision.
    pub async fn batch_update(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        batch: BatchBlockTranslationUpdate,
    ) -> Result<BatchBlockUpdateResult> {
        let mut succeeded = Vec::new();
        let mut failed = Vec::new();

        for update in batch.updates {
            let message = update
                .message
                .or_else(|| batch.message.clone())
                .or_else(|| {
                    Some(format!(
                        "Batch update {} translation for block {}",
                        update.locale.as_str(),
                        update.block_uuid
                    ))
                });

            // Note: Caller should allocate HLC revisions before calling batch_update
            // For now, we use a placeholder - this method signature needs revision
            let revision = raisin_hlc::HLC::now(); // TODO: Pass revisions from caller

            match self
                .update_block_translation(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &update.node_id,
                    &update.block_uuid,
                    &update.locale,
                    update.translations,
                    &batch.actor,
                    message,
                    revision,
                )
                .await
            {
                Ok(result) => {
                    succeeded.push(result);
                }
                Err(e) => {
                    failed.push((
                        update.node_id,
                        update.block_uuid,
                        update.locale,
                        format!("Block translation update failed: {}", e),
                    ));
                }
            }
        }

        Ok(BatchBlockUpdateResult { succeeded, failed })
    }

    /// Mark blocks as orphaned when they're deleted from the base node.
    ///
    /// When blocks are removed from a Composite in the master content,
    /// their translations should be marked as orphaned rather than deleted.
    /// This preserves the translation history and allows recovery if the
    /// block is re-added.
    pub async fn mark_blocks_orphaned(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        block_uuids: Vec<String>,
        revision: &raisin_hlc::HLC,
    ) -> Result<()> {
        if block_uuids.is_empty() {
            return Ok(());
        }

        self.repository
            .mark_blocks_orphaned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                node_id,
                &block_uuids,
                revision,
            )
            .await
    }

    /// Extract all block UUIDs from a node's Composite properties.
    ///
    /// Scans the node's properties to find all blocks with UUIDs.
    /// This is useful for detecting which blocks exist in the current
    /// version of the node, allowing comparison with translated blocks
    /// to identify orphans.
    pub fn extract_block_uuids(&self, node: &Node) -> HashSet<String> {
        let mut uuids = HashSet::new();
        let mut stack: Vec<&PropertyValue> = node.properties.values().collect();

        while let Some(value) = stack.pop() {
            match value {
                PropertyValue::Array(items) => {
                    for item in items {
                        if let PropertyValue::Object(obj) = item {
                            // Check if this object has a UUID (indicates it's a block)
                            if let Some(PropertyValue::String(uuid)) = obj.get("uuid") {
                                uuids.insert(uuid.clone());
                            }
                            // Also push nested values onto stack
                            stack.extend(obj.values());
                        } else {
                            stack.push(item);
                        }
                    }
                }
                PropertyValue::Object(obj) => {
                    stack.extend(obj.values());
                }
                _ => {}
            }
        }

        uuids
    }

    /// Delete a block translation for a specific locale.
    ///
    /// This removes the translation overlay, causing the block to fall back
    /// to the base content or next locale in the fallback chain.
    pub async fn delete_block_translation(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        block_uuid: &str,
        locale: &LocaleCode,
        actor: &str,
        message: Option<String>,
        revision: raisin_hlc::HLC,
    ) -> Result<BlockTranslationUpdateResult> {
        // Store an empty overlay to effectively delete the translation
        let overlay = LocaleOverlay::Properties {
            data: HashMap::new(),
        };

        let timestamp = Utc::now();
        let meta = TranslationMeta {
            locale: locale.clone(),
            revision,
            parent_revision: None,
            timestamp,
            actor: actor.to_string(),
            message: message.unwrap_or_else(|| {
                format!(
                    "Delete {} translation for block {}",
                    locale.as_str(),
                    block_uuid
                )
            }),
            is_system: false,
        };

        self.repository
            .store_block_translation(
                tenant_id, repo_id, branch, workspace, node_id, block_uuid, locale, &overlay, &meta,
            )
            .await?;

        Ok(BlockTranslationUpdateResult {
            block_uuid: block_uuid.to_string(),
            node_id: node_id.to_string(),
            locale: locale.clone(),
            revision,
            timestamp,
        })
    }

    /// Get a block translation for a specific block and locale.
    pub async fn get_block_translation(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        block_uuid: &str,
        locale: &LocaleCode,
        revision: &raisin_hlc::HLC,
    ) -> Result<Option<LocaleOverlay>> {
        self.repository
            .get_block_translation(
                tenant_id, repo_id, branch, workspace, node_id, block_uuid, locale, revision,
            )
            .await
    }
}
