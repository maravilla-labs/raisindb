//! Node-level translation CRUD operations.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::translations::{LocaleCode, LocaleOverlay, TranslationMeta};
use rocksdb::DB;
use std::sync::Arc;

use crate::error_ext::ResultExt;

use super::{keys, replication, revision, serialization};

/// Get a node-level translation
pub(super) async fn get_translation(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    locale: &LocaleCode,
    _revision: &HLC,
) -> Result<Option<LocaleOverlay>> {
    let cf = crate::cf_handle(db, crate::cf::TRANSLATION_DATA)?;

    // Use prefix iteration to find most recent translation at or before revision
    let prefix = keys::translation_prefix(
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        locale.as_str(),
    );

    let mut iter = db.prefix_iterator_cf(&cf, &prefix);

    // First key will be the most recent (descending revision order)
    if let Some(Ok((key, value))) = iter.next() {
        // Verify this key is within our prefix and at or before the requested revision
        if key.starts_with(&prefix) {
            // Check if this is a tombstone marker (deleted translation)
            if value.as_ref() == b"T" {
                return Ok(None);
            }

            // Deserialize the LocaleOverlay
            let overlay = serialization::deserialize_overlay(&value)?;
            return Ok(Some(overlay));
        }
    }

    Ok(None)
}

/// Store a node-level translation
pub(super) async fn store_translation(
    db: &Arc<DB>,
    operation_capture: Option<&Arc<crate::OperationCapture>>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    locale: &LocaleCode,
    overlay: &LocaleOverlay,
    meta: &TranslationMeta,
) -> Result<()> {
    let cf_data = crate::cf_handle(db, crate::cf::TRANSLATION_DATA)?;
    let cf_index = crate::cf_handle(db, crate::cf::TRANSLATION_INDEX)?;
    let cf_meta = crate::cf_handle(db, crate::cf::REVISIONS)?;

    // Serialize overlay and metadata
    let overlay_bytes = serialization::serialize_overlay(overlay)?;
    let meta_bytes = serialization::serialize_translation_meta(meta)?;

    // Build keys
    let data_key = keys::translation_key(
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        locale.as_str(),
        &meta.revision,
    );

    let index_key =
        keys::translation_index_key(tenant_id, repo_id, locale.as_str(), &meta.revision, node_id);

    let meta_key = keys::translation_meta_key(
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        locale.as_str(),
        &meta.revision,
    );

    // Write to all three CFs
    db.put_cf(&cf_data, data_key, &overlay_bytes)
        .rocksdb_err()?;
    db.put_cf(&cf_index, index_key, b"").rocksdb_err()?; // Index entry (empty value)
    db.put_cf(&cf_meta, meta_key, meta_bytes).rocksdb_err()?;

    // Store RevisionMeta so translation changes appear in revision history
    revision::store_node_revision_meta(
        db,
        tenant_id,
        repo_id,
        branch,
        node_id,
        workspace,
        locale.as_str(),
        overlay,
        meta,
    )?;

    // Store translation snapshot for time-travel queries and rollback
    revision::store_snapshot(
        db,
        tenant_id,
        repo_id,
        node_id,
        locale.as_str(),
        &meta.revision,
        &overlay_bytes,
    )?;

    // Capture operation for replication
    replication::capture_node_translation(
        operation_capture,
        tenant_id,
        repo_id,
        branch,
        node_id,
        locale.as_str(),
        overlay,
        &meta.actor,
    )
    .await;

    Ok(())
}

/// List all translations for a node
pub(super) async fn list_translations_for_node(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    _revision: &HLC,
) -> Result<Vec<LocaleCode>> {
    let cf = crate::cf_handle(db, crate::cf::TRANSLATION_DATA)?;

    // Build prefix for all translations of this node
    let prefix = format!(
        "{}\0{}\0{}\0{}\0translations\0{}\0",
        tenant_id, repo_id, branch, workspace, node_id
    )
    .into_bytes();

    let mut locales = std::collections::HashSet::new();
    let iter = db.prefix_iterator_cf(&cf, &prefix);

    for item in iter {
        let (key, _value) = item.rocksdb_err()?;

        // Parse locale from key
        // Key format: {prefix}{locale}\0{~revision}
        if let Some(suffix) = key.strip_prefix(prefix.as_slice()) {
            if let Ok(suffix_str) = std::str::from_utf8(suffix) {
                // Extract locale (before the next \0)
                if let Some(locale_str) = suffix_str.split('\0').next() {
                    if let Ok(locale) = LocaleCode::parse(locale_str) {
                        locales.insert(locale);
                    }
                }
            }
        }
    }

    Ok(locales.into_iter().collect())
}
