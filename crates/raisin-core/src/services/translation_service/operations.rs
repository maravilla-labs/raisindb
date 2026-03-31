//! Translation operations: update, batch, hide/unhide, delete, query.

use chrono::Utc;
use raisin_error::{Error, Result};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::translations::{
    JsonPointer, LocaleCode, LocaleOverlay, TranslationHashRecord, TranslationMeta,
};
use raisin_storage::{
    BranchRepository, NodeRepository, RevisionRepository, Storage, StorageScope,
    TranslationRepository,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

use super::{
    BatchTranslationUpdate, BatchUpdateResult, TranslationService, TranslationUpdateResult,
};

impl<S: Storage> TranslationService<S> {
    /// Update a translation for a single node.
    ///
    /// Creates a LocaleOverlay with the provided translations and stores it
    /// in the repository, automatically creating a new revision.
    pub async fn update_translation(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        locale: &LocaleCode,
        translations: HashMap<JsonPointer, PropertyValue>,
        actor: &str,
        message: Option<String>,
    ) -> Result<TranslationUpdateResult> {
        // Validate that we have at least one translation
        if translations.is_empty() {
            return Err(Error::Validation(
                "Translation update must contain at least one property".to_string(),
            ));
        }

        // Allocate a new revision atomically (prevents race conditions)
        let new_revision = self.storage.revisions().allocate_revision();

        // Get current branch head for parent_revision tracking
        let current_revision = self
            .storage
            .branches()
            .get_head(tenant_id, repo_id, branch)
            .await?;

        // Create the translation overlay
        let overlay = LocaleOverlay::Properties {
            data: translations.clone(),
        };

        // Create translation metadata
        let timestamp = Utc::now();
        let meta = TranslationMeta {
            locale: locale.clone(),
            revision: new_revision,
            parent_revision: Some(current_revision),
            timestamp,
            actor: actor.to_string(),
            message: message.unwrap_or_else(|| format!("Update {} translation", locale.as_str())),
            is_system: false,
        };

        // Store the translation at new revision
        self.storage
            .translations()
            .store_translation(
                tenant_id, repo_id, branch, workspace, node_id, locale, &overlay, &meta,
            )
            .await?;

        // Update branch head to new revision
        self.storage
            .branches()
            .update_head(tenant_id, repo_id, branch, new_revision)
            .await?;

        // Record hash records for staleness detection
        // First, try to get the original node to compute hashes
        let scope = StorageScope::new(tenant_id, repo_id, branch, workspace);
        if let Ok(Some(node)) = self
            .storage
            .nodes()
            .get(scope, node_id, Some(&new_revision))
            .await
        {
            let hash_records = Self::compute_hash_records(&node, &translations, new_revision);
            if !hash_records.is_empty() {
                // Store hash records (best effort - don't fail if this fails)
                let _ = self
                    .storage
                    .translations()
                    .store_hash_records_batch(
                        tenant_id,
                        repo_id,
                        branch,
                        workspace,
                        node_id,
                        locale,
                        &hash_records,
                    )
                    .await;
            }
        }

        Ok(TranslationUpdateResult {
            node_id: node_id.to_string(),
            locale: locale.clone(),
            revision: new_revision,
            timestamp,
        })
    }

    /// Compute hash records for translated fields based on original node content.
    fn compute_hash_records(
        node: &Node,
        translations: &HashMap<JsonPointer, PropertyValue>,
        revision: raisin_hlc::HLC,
    ) -> HashMap<JsonPointer, TranslationHashRecord> {
        let mut records = HashMap::new();

        for pointer in translations.keys() {
            // Try to find the original value at this pointer
            if let Some(original_value) = Self::get_value_at_pointer(node, pointer) {
                let hash = Self::hash_property_value(&original_value);
                records.insert(pointer.clone(), TranslationHashRecord::new(hash, revision));
            }
        }

        records
    }

    /// Hash a PropertyValue for staleness detection.
    fn hash_property_value(value: &PropertyValue) -> String {
        let json = serde_json::to_string(value).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(json.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Get the value at a JSON pointer path in a node's properties.
    fn get_value_at_pointer(node: &Node, pointer: &JsonPointer) -> Option<PropertyValue> {
        let segments: Vec<&str> = pointer
            .as_str()
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();

        if segments.is_empty() {
            return None;
        }

        // Start with the top-level property
        let first_key = segments[0];
        let mut current = node.properties.get(first_key)?.clone();

        // Navigate through remaining segments
        for segment in segments.iter().skip(1) {
            current = match current {
                PropertyValue::Object(ref obj) => obj.get(*segment)?.clone(),
                PropertyValue::Array(ref arr) => {
                    let idx: usize = segment.parse().ok()?;
                    arr.get(idx)?.clone()
                }
                PropertyValue::Composite(ref composite) => {
                    // For composite, segment could be a UUID then a content key
                    for item in &composite.items {
                        if item.uuid == *segment {
                            // Return the content as an object for further traversal
                            return Some(PropertyValue::Object(item.content.clone()));
                        }
                    }
                    return None;
                }
                PropertyValue::Element(ref element) => element.content.get(*segment)?.clone(),
                _ => return None,
            };
        }

        Some(current)
    }

    /// Update multiple translations in a batch operation.
    ///
    /// Applies all translation updates, continuing on errors and collecting
    /// both successful and failed operations. Automatically manages revisions.
    pub async fn batch_update(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        batch: BatchTranslationUpdate,
    ) -> Result<BatchUpdateResult> {
        let mut succeeded = Vec::new();
        let mut failed = Vec::new();

        for update in batch.updates {
            let message = update
                .message
                .or_else(|| batch.message.clone())
                .or_else(|| {
                    Some(format!(
                        "Batch update {} translation",
                        update.locale.as_str()
                    ))
                });

            match self
                .update_translation(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &update.node_id,
                    &update.locale,
                    update.translations,
                    &batch.actor,
                    message,
                )
                .await
            {
                Ok(result) => {
                    succeeded.push(result);
                }
                Err(e) => {
                    failed.push((
                        update.node_id,
                        update.locale,
                        format!("Translation update failed: {}", e),
                    ));
                }
            }
        }

        Ok(BatchUpdateResult { succeeded, failed })
    }

    /// Hide a node in a specific locale.
    ///
    /// Creates a LocaleOverlay::Hidden tombstone that marks the node as
    /// hidden in the specified locale. When the translation resolver
    /// encounters a Hidden overlay, it returns None instead of the node.
    /// Automatically creates a new revision.
    pub async fn hide_node(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        locale: &LocaleCode,
        actor: &str,
        message: Option<String>,
    ) -> Result<TranslationUpdateResult> {
        // Allocate a new revision atomically (prevents race conditions)
        let new_revision = self.storage.revisions().allocate_revision();

        // Get current branch head for parent_revision tracking
        let current_revision = self
            .storage
            .branches()
            .get_head(tenant_id, repo_id, branch)
            .await?;

        // Create the Hidden overlay
        let overlay = LocaleOverlay::Hidden;

        // Create translation metadata
        let timestamp = Utc::now();
        let meta = TranslationMeta {
            locale: locale.clone(),
            revision: new_revision,
            parent_revision: Some(current_revision),
            timestamp,
            actor: actor.to_string(),
            message: message.unwrap_or_else(|| format!("Hide node in {} locale", locale.as_str())),
            is_system: false,
        };

        // Store the Hidden overlay at new revision
        self.storage
            .translations()
            .store_translation(
                tenant_id, repo_id, branch, workspace, node_id, locale, &overlay, &meta,
            )
            .await?;

        // Update branch head to new revision
        self.storage
            .branches()
            .update_head(tenant_id, repo_id, branch, new_revision)
            .await?;

        Ok(TranslationUpdateResult {
            node_id: node_id.to_string(),
            locale: locale.clone(),
            revision: new_revision,
            timestamp,
        })
    }

    /// Unhide a node in a specific locale.
    ///
    /// Removes the Hidden tombstone by storing an empty Properties overlay.
    /// This makes the node visible again in the specified locale.
    /// Automatically creates a new revision.
    pub async fn unhide_node(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        locale: &LocaleCode,
        actor: &str,
        message: Option<String>,
    ) -> Result<TranslationUpdateResult> {
        // Allocate a new revision atomically (prevents race conditions)
        let new_revision = self.storage.revisions().allocate_revision();

        // Get current branch head for parent_revision tracking
        let current_revision = self
            .storage
            .branches()
            .get_head(tenant_id, repo_id, branch)
            .await?;

        // Create an empty Properties overlay (effectively removing Hidden state)
        let overlay = LocaleOverlay::Properties {
            data: HashMap::new(),
        };

        // Create translation metadata
        let timestamp = Utc::now();
        let meta = TranslationMeta {
            locale: locale.clone(),
            revision: new_revision,
            parent_revision: Some(current_revision),
            timestamp,
            actor: actor.to_string(),
            message: message
                .unwrap_or_else(|| format!("Unhide node in {} locale", locale.as_str())),
            is_system: false,
        };

        // Store the empty overlay at new revision
        self.storage
            .translations()
            .store_translation(
                tenant_id, repo_id, branch, workspace, node_id, locale, &overlay, &meta,
            )
            .await?;

        // Update branch head to new revision
        self.storage
            .branches()
            .update_head(tenant_id, repo_id, branch, new_revision)
            .await?;

        Ok(TranslationUpdateResult {
            node_id: node_id.to_string(),
            locale: locale.clone(),
            revision: new_revision,
            timestamp,
        })
    }

    /// Delete a translation for a node in a specific locale.
    ///
    /// This is effectively the same as unhiding - it stores an empty
    /// Properties overlay, which removes all translations for the locale
    /// and falls back to the base content or next locale in the fallback chain.
    /// Automatically creates a new revision.
    pub async fn delete_translation(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        locale: &LocaleCode,
        actor: &str,
        message: Option<String>,
    ) -> Result<TranslationUpdateResult> {
        // Same as unhide - store empty overlay
        self.unhide_node(
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            locale,
            actor,
            message.or_else(|| Some(format!("Delete {} translation", locale.as_str()))),
        )
        .await
    }

    /// Get the translation metadata for a node in a specific locale.
    ///
    /// Returns information about the most recent translation update.
    pub async fn get_translation_meta(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        locale: &LocaleCode,
    ) -> Result<Option<TranslationMeta>> {
        self.storage
            .translations()
            .get_translation_meta(tenant_id, repo_id, branch, workspace, node_id, locale)
            .await
    }

    /// List all translations available for a node.
    ///
    /// Returns the set of locales that have translations for this node.
    pub async fn list_translations(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        revision: &raisin_hlc::HLC,
    ) -> Result<Vec<LocaleCode>> {
        self.storage
            .translations()
            .list_translations_for_node(tenant_id, repo_id, branch, workspace, node_id, revision)
            .await
    }

    /// List all nodes that have translations in a specific locale.
    ///
    /// Useful for finding translated content or generating translation reports.
    pub async fn list_translated_nodes(
        &self,
        tenant_id: &str,
        repo_id: &str,
        locale: &LocaleCode,
        revision: &raisin_hlc::HLC,
    ) -> Result<Vec<String>> {
        self.storage
            .translations()
            .list_nodes_with_translation(tenant_id, repo_id, locale, revision)
            .await
    }
}
