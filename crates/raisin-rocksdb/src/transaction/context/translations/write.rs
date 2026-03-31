//! Translation write operations
//!
//! This module contains the implementation of translation write operations for transactions:
//! - `store_translation`: Store a locale overlay for a node
//!
//! # Key Features
//!
//! ## Translation Storage
//!
//! Translations are stored in two column families:
//! - TRANSLATION_DATA: The actual LocaleOverlay data
//! - TRANSLATION_INDEX: Reverse index for listing translations by locale

use raisin_error::Result;
use raisin_models::translations::LocaleOverlay;

use crate::transaction::change_types::TranslationChange;
use crate::transaction::RocksDBTransaction;
use crate::{cf, cf_handle, keys};

/// Store a translation (locale overlay) for a node
///
/// # Translation Storage
///
/// Translations are stored in two column families:
/// - TRANSLATION_DATA: The actual LocaleOverlay data
/// - TRANSLATION_INDEX: Reverse index for listing translations by locale
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `workspace` - The workspace containing the node
/// * `node_id` - The ID of the node
/// * `locale` - The locale code (e.g., "en", "fr")
/// * `overlay` - The locale overlay data
///
/// # Returns
///
/// Ok(()) on success
pub async fn store_translation(
    tx: &RocksDBTransaction,
    workspace: &str,
    node_id: &str,
    locale: &str,
    overlay: LocaleOverlay,
) -> Result<()> {
    // 1. Get metadata
    let (tenant_id, repo_id, branch) = {
        let meta = tx
            .metadata
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        (
            meta.tenant_id.clone(),
            meta.repo_id.clone(),
            meta.branch.clone().ok_or_else(|| {
                raisin_error::Error::Validation("Branch not set in transaction".into())
            })?,
        )
    };

    // 2. Get or allocate the single transaction HLC (all operations in tx share same revision)
    let revision = tx.get_or_allocate_transaction_revision()?;

    tracing::debug!(
        "TXN store_translation: workspace={}, node_id={}, locale={}, revision={}",
        workspace,
        node_id,
        locale,
        revision
    );

    // 3. Serialize LocaleOverlay to JSON
    let overlay_json = serde_json::to_vec(&overlay)
        .map_err(|e| raisin_error::Error::storage(format!("JSON serialization error: {}", e)))?;

    // 4. Build translation_data key
    // Format: {tenant}\0{repo}\0{branch}\0{workspace}\0translations\0{node_id}\0{locale}\0{~revision}
    let mut key = format!(
        "{}\0{}\0{}\0{}\0translations\0{}\0{}\0",
        tenant_id, repo_id, branch, workspace, node_id, locale
    )
    .into_bytes();
    key.extend_from_slice(&keys::encode_descending_revision(&revision));

    // 5. Add to WriteBatch
    {
        let mut batch = tx
            .batch
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

        let cf_translation_data = cf_handle(&tx.db, cf::TRANSLATION_DATA)?;
        batch.put_cf(cf_translation_data, &key, overlay_json);

        // Also add to translation_index for reverse lookups
        // Format: {tenant}\0{repo}\0translation_index\0{locale}\0{~revision}\0{node_id}
        let mut index_key = format!(
            "{}\0{}\0translation_index\0{}\0",
            tenant_id, repo_id, locale
        )
        .into_bytes();
        index_key.extend_from_slice(&keys::encode_descending_revision(&revision));
        index_key.push(b'\0');
        index_key.extend_from_slice(node_id.as_bytes());

        let cf_translation_index = cf_handle(&tx.db, cf::TRANSLATION_INDEX)?;
        batch.put_cf(cf_translation_index, index_key, node_id.as_bytes());
    }

    // 6. Update read cache for read-your-writes
    {
        let mut cache = tx
            .read_cache
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        cache.translations.insert(
            (
                workspace.to_string(),
                node_id.to_string(),
                locale.to_string(),
            ),
            Some(overlay),
        );
    }

    // 7. Track in changed_translations
    {
        let mut changed = tx
            .changed_translations
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        changed.insert(
            (node_id.to_string(), locale.to_string()),
            TranslationChange {
                workspace: workspace.to_string(),
                revision,
                operation: raisin_models::tree::ChangeOperation::Modified,
            },
        );
    }

    // 8. Record write for conflict detection
    tx.record_write(key)?;

    Ok(())
}
