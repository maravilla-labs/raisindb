//! Translation staleness detection service.
//!
//! This service detects when original content has changed since a translation was created,
//! allowing the UI to show stale/fresh/missing indicators for each translated field.
//!
//! # Problem
//!
//! When the original language content changes (fields added, removed, or modified),
//! existing translations become stale but the system needs to detect this:
//!
//! - A new field is added → translator doesn't see it needs translation
//! - A field is edited → existing translation may no longer match the original's intent
//!
//! # Solution
//!
//! Store a hash of the original content alongside each translation pointer.
//! On read, compare stored hash vs current original hash to detect staleness.
//!
//! # Example
//!
//! ```ignore
//! use raisin_core::TranslationStalenessService;
//!
//! let service = TranslationStalenessService::new(storage);
//!
//! // Check staleness for a node's translations
//! let report = service.check_staleness(
//!     tenant_id, repo_id, branch, workspace,
//!     &node, locale, None,
//! ).await?;
//!
//! for stale in report.stale_fields {
//!     println!("Field {} is stale - original changed", stale.pointer);
//! }
//! ```

use raisin_error::Result;
use raisin_models::nodes::properties::schema::PropertyValueSchema;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::translations::{
    JsonPointer, LocaleCode, LocaleOverlay, MissingFieldInfo, StaleFieldInfo, StalenessReport,
    TranslationHashRecord,
};
use raisin_storage::{BranchRepository, Storage, TranslationRepository};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;

/// Service for detecting translation staleness via content hashing.
pub struct TranslationStalenessService<S: Storage> {
    storage: Arc<S>,
}

impl<S: Storage> TranslationStalenessService<S> {
    /// Create a new staleness detection service.
    pub fn new(storage: Arc<S>) -> Self {
        Self { storage }
    }

    /// Compute a stable hash for a PropertyValue.
    ///
    /// Uses canonical JSON serialization followed by SHA-256 hashing.
    /// This ensures consistent hashes across different serialization orders.
    pub fn hash_value(value: &PropertyValue) -> String {
        // Use serde_json for canonical serialization
        let json = serde_json::to_string(value).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(json.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Extract translatable field pointers and their values from a node's properties.
    ///
    /// If `schema` is provided, only fields marked with `is_translatable: true` are extracted.
    /// If `schema` is `None`, falls back to extracting all string fields for backwards compatibility.
    ///
    /// Returns a map of JsonPointer -> PropertyValue for all translatable fields.
    pub fn extract_translatable_fields(
        node: &Node,
        schema: Option<&[PropertyValueSchema]>,
    ) -> HashMap<JsonPointer, PropertyValue> {
        let mut fields = HashMap::new();

        // Build a lookup map from schema if provided
        let schema_map: Option<HashMap<&str, &PropertyValueSchema>> = schema.map(|props| {
            props
                .iter()
                .filter_map(|p| p.name.as_ref().map(|n| (n.as_str(), p)))
                .collect()
        });

        // Walk through properties and extract translatable string/text fields
        for (key, value) in &node.properties {
            let pointer = JsonPointer::new(&format!("/{}", key));

            // Check if this field is marked translatable in the schema
            let is_schema_translatable = if let Some(ref map) = schema_map {
                map.get(key.as_str())
                    .and_then(|p| p.is_translatable)
                    .unwrap_or(false)
            } else {
                // No schema provided - fall back to type-based check
                true
            };

            // For top-level fields, check schema translatability OR fallback to type check
            if is_schema_translatable && Self::is_translatable_value(value) {
                fields.insert(pointer.clone(), value.clone());
            }

            // For composite arrays, extract nested translatable fields
            // Note: For nested structures, we currently don't have nested schema info,
            // so we still use type-based extraction for nested content
            if let PropertyValue::Array(items) = value {
                for (idx, item) in items.iter().enumerate() {
                    Self::extract_nested_translatable(
                        &format!("/{}/{}", key, idx),
                        item,
                        &mut fields,
                    );
                }
            }

            // For objects, extract nested translatable fields
            if let PropertyValue::Object(obj) = value {
                for (nested_key, nested_value) in obj {
                    Self::extract_nested_translatable(
                        &format!("/{}/{}", key, nested_key),
                        nested_value,
                        &mut fields,
                    );
                }
            }
        }

        fields
    }

    /// Helper to extract nested translatable fields
    fn extract_nested_translatable(
        prefix: &str,
        value: &PropertyValue,
        fields: &mut HashMap<JsonPointer, PropertyValue>,
    ) {
        if Self::is_translatable_value(value) {
            fields.insert(JsonPointer::new(prefix), value.clone());
        }

        match value {
            PropertyValue::Array(items) => {
                for (idx, item) in items.iter().enumerate() {
                    Self::extract_nested_translatable(&format!("{}/{}", prefix, idx), item, fields);
                }
            }
            PropertyValue::Object(obj) => {
                for (key, nested_value) in obj {
                    Self::extract_nested_translatable(
                        &format!("{}/{}", prefix, key),
                        nested_value,
                        fields,
                    );
                }
            }
            PropertyValue::Composite(composite) => {
                // For composites, each item has a uuid and content
                for item in &composite.items {
                    if !item.uuid.is_empty() {
                        for (key, nested_value) in &item.content {
                            Self::extract_nested_translatable(
                                &format!("{}/{}/{}", prefix, item.uuid, key),
                                nested_value,
                                fields,
                            );
                        }
                    }
                }
            }
            PropertyValue::Element(element) => {
                // For elements, content is translatable
                for (key, nested_value) in &element.content {
                    Self::extract_nested_translatable(
                        &format!("{}/{}", prefix, key),
                        nested_value,
                        fields,
                    );
                }
            }
            _ => {}
        }
    }

    /// Check if a value type is translatable (text content).
    fn is_translatable_value(value: &PropertyValue) -> bool {
        matches!(value, PropertyValue::String(_))
    }

    /// Check staleness for all translations of a node in a specific locale.
    ///
    /// Compares stored hash records against current original hashes to detect:
    /// - Stale fields: original has changed since translation
    /// - Fresh fields: original matches the translation
    /// - Missing fields: original exists but no translation
    /// - Unknown fields: translation exists but no hash record (legacy)
    ///
    /// If `schema` is provided, only fields marked `is_translatable: true` are considered.
    pub async fn check_staleness(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node: &Node,
        locale: &LocaleCode,
        schema: Option<&[PropertyValueSchema]>,
    ) -> Result<StalenessReport> {
        let mut report = StalenessReport::new();

        // Get the translation overlay for this node/locale
        let revision = self
            .storage
            .branches()
            .get_head(tenant_id, repo_id, branch)
            .await?;

        let overlay = self
            .storage
            .translations()
            .get_translation(
                tenant_id, repo_id, branch, workspace, &node.id, locale, &revision,
            )
            .await?;

        // Get stored hash records
        let hash_records = self
            .storage
            .translations()
            .get_hash_records(tenant_id, repo_id, branch, workspace, &node.id, locale)
            .await?;

        // Extract current translatable fields from the original node
        let original_fields = Self::extract_translatable_fields(node, schema);

        // Get translated pointers from overlay
        let translated_pointers: std::collections::HashSet<JsonPointer> = match &overlay {
            Some(LocaleOverlay::Properties { data }) => data.keys().cloned().collect(),
            _ => std::collections::HashSet::new(),
        };

        // Check each original field
        for (pointer, value) in &original_fields {
            let current_hash = Self::hash_value(value);

            if translated_pointers.contains(pointer) {
                // Field has a translation
                if let Some(hash_record) = hash_records.get(pointer) {
                    // We have a hash record - compare
                    if hash_record.is_stale(&current_hash) {
                        report.stale_fields.push(StaleFieldInfo {
                            pointer: pointer.as_str().to_string(),
                            original_hash_at_translation: hash_record.original_hash.clone(),
                            current_original_hash: current_hash,
                            translated_at: hash_record.recorded_at,
                        });
                    } else {
                        report.fresh_fields.push(pointer.as_str().to_string());
                    }
                } else {
                    // No hash record - translation was created before hash tracking
                    report.unknown_fields.push(pointer.as_str().to_string());
                }
            } else {
                // No translation for this field
                report.missing_fields.push(MissingFieldInfo {
                    pointer: pointer.as_str().to_string(),
                    current_original_hash: current_hash,
                });
            }
        }

        Ok(report)
    }

    /// Record hashes for translated fields.
    ///
    /// Called when a translation is saved to record the current state of
    /// the original content for future staleness detection.
    ///
    /// If `schema` is provided, only fields marked `is_translatable: true` are considered.
    pub async fn record_hashes(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node: &Node,
        locale: &LocaleCode,
        translated_pointers: &[JsonPointer],
        schema: Option<&[PropertyValueSchema]>,
    ) -> Result<()> {
        if translated_pointers.is_empty() {
            return Ok(());
        }

        // Get current revision
        let revision = self
            .storage
            .branches()
            .get_head(tenant_id, repo_id, branch)
            .await?;

        // Extract original fields
        let original_fields = Self::extract_translatable_fields(node, schema);

        // Build hash records for translated fields
        let mut records = HashMap::new();
        for pointer in translated_pointers {
            if let Some(value) = original_fields.get(pointer) {
                let hash = Self::hash_value(value);
                records.insert(pointer.clone(), TranslationHashRecord::new(hash, revision));
            }
        }

        // Store the hash records
        if !records.is_empty() {
            self.storage
                .translations()
                .store_hash_records_batch(
                    tenant_id, repo_id, branch, workspace, &node.id, locale, &records,
                )
                .await?;
        }

        Ok(())
    }

    /// Clear hash records when a translation is deleted.
    pub async fn clear_hashes(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        locale: &LocaleCode,
    ) -> Result<()> {
        self.storage
            .translations()
            .delete_hash_records(tenant_id, repo_id, branch, workspace, node_id, locale)
            .await
    }

    /// Mark a stale translation as acknowledged.
    ///
    /// Re-records the hash with the current original value, clearing the stale status
    /// without requiring the translation to be re-done.
    ///
    /// If `schema` is provided, only fields marked `is_translatable: true` are considered.
    pub async fn acknowledge_staleness(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node: &Node,
        locale: &LocaleCode,
        pointer: &JsonPointer,
        schema: Option<&[PropertyValueSchema]>,
    ) -> Result<()> {
        let revision = self
            .storage
            .branches()
            .get_head(tenant_id, repo_id, branch)
            .await?;

        let original_fields = Self::extract_translatable_fields(node, schema);

        if let Some(value) = original_fields.get(pointer) {
            let hash = Self::hash_value(value);
            let record = TranslationHashRecord::new(hash, revision);

            self.storage
                .translations()
                .store_hash_record(
                    tenant_id, repo_id, branch, workspace, &node.id, locale, pointer, &record,
                )
                .await?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::properties::PropertyValue;

    #[test]
    fn test_hash_value_consistency() {
        let value = PropertyValue::String("Hello World".to_string());

        let hash1 =
            TranslationStalenessService::<raisin_storage_memory::InMemoryStorage>::hash_value(
                &value,
            );
        let hash2 =
            TranslationStalenessService::<raisin_storage_memory::InMemoryStorage>::hash_value(
                &value,
            );

        assert_eq!(hash1, hash2, "Same value should produce same hash");
    }

    #[test]
    fn test_hash_value_different_content() {
        let value1 = PropertyValue::String("Hello".to_string());
        let value2 = PropertyValue::String("World".to_string());

        let hash1 =
            TranslationStalenessService::<raisin_storage_memory::InMemoryStorage>::hash_value(
                &value1,
            );
        let hash2 =
            TranslationStalenessService::<raisin_storage_memory::InMemoryStorage>::hash_value(
                &value2,
            );

        assert_ne!(
            hash1, hash2,
            "Different values should produce different hashes"
        );
    }

    #[test]
    fn test_is_translatable_value() {
        assert!(TranslationStalenessService::<
            raisin_storage_memory::InMemoryStorage,
        >::is_translatable_value(&PropertyValue::String(
            "text".to_string()
        )));

        assert!(!TranslationStalenessService::<
            raisin_storage_memory::InMemoryStorage,
        >::is_translatable_value(&PropertyValue::Integer(
            42
        )));

        assert!(!TranslationStalenessService::<
            raisin_storage_memory::InMemoryStorage,
        >::is_translatable_value(&PropertyValue::Boolean(
            true
        )));
    }
}
