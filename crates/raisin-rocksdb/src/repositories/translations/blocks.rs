//! Block-level translation CRUD operations.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::translations::{LocaleCode, LocaleOverlay, TranslationMeta};
use rocksdb::DB;
use std::sync::Arc;

use crate::error_ext::ResultExt;

use super::{keys, replication, revision, serialization};

/// Get a block-level translation
pub(super) async fn get_block_translation(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    block_uuid: &str,
    locale: &LocaleCode,
    _revision: &HLC,
) -> Result<Option<LocaleOverlay>> {
    let cf = crate::cf_handle(db, crate::cf::BLOCK_TRANSLATIONS)?;

    let prefix = keys::block_translation_prefix(
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        block_uuid,
        locale.as_str(),
    );

    let mut iter = db.prefix_iterator_cf(&cf, &prefix);

    if let Some(Ok((_key, value))) = iter.next() {
        let overlay = serialization::deserialize_overlay(&value)?;
        return Ok(Some(overlay));
    }

    Ok(None)
}

/// Store a block-level translation
pub(super) async fn store_block_translation(
    db: &Arc<DB>,
    operation_capture: Option<&Arc<crate::OperationCapture>>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    block_uuid: &str,
    locale: &LocaleCode,
    overlay: &LocaleOverlay,
    meta: &TranslationMeta,
) -> Result<()> {
    let cf = crate::cf_handle(db, crate::cf::BLOCK_TRANSLATIONS)?;

    let overlay_bytes = serialization::serialize_overlay(overlay)?;

    let key = keys::block_translation_key(
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        block_uuid,
        locale.as_str(),
        &meta.revision,
    );

    db.put_cf(&cf, key, &overlay_bytes).rocksdb_err()?;

    // Store RevisionMeta to track this block translation change
    revision::store_block_revision_meta(
        db,
        tenant_id,
        repo_id,
        branch,
        node_id,
        workspace,
        locale.as_str(),
        block_uuid,
        overlay,
        meta,
    )?;

    // Store block translation snapshot for time-travel queries
    // Block translations use "{locale}::{block_uuid}" format to track specific block changes
    let locale_key = format!("{}::{}", locale.as_str(), block_uuid);
    revision::store_snapshot(
        db,
        tenant_id,
        repo_id,
        node_id,
        &locale_key,
        &meta.revision,
        &overlay_bytes,
    )?;

    // Capture operation for replication
    replication::capture_block_translation(
        operation_capture,
        tenant_id,
        repo_id,
        branch,
        node_id,
        locale.as_str(),
        block_uuid,
        overlay,
        &meta.actor,
    )
    .await;

    Ok(())
}

/// List every `(block_uuid, locale)` that has a block overlay for this node.
///
/// One prefix scan over the node's block-translation key space. It replaces the
/// resolver's old shape — walk the whole property tree, then issue a point read
/// per block uuid found, per locale in the fallback chain — which cost N reads on
/// every localized read of every node, including the overwhelmingly common case of
/// a node with no block overlays at all, where the answer was always "none".
///
/// Orphan markers (`…\0{block_uuid}\0orphaned\0…`) are skipped: `orphaned` sits in
/// the locale position but is a marker, not a locale.
pub(super) async fn list_block_translations_for_node(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
) -> Result<Vec<(String, LocaleCode)>> {
    let cf = crate::cf_handle(db, crate::cf::BLOCK_TRANSLATIONS)?;
    let prefix =
        keys::block_translations_node_prefix(tenant_id, repo_id, branch, workspace, node_id);

    let mut found = std::collections::HashSet::new();
    for item in db.prefix_iterator_cf(&cf, &prefix) {
        let (key, _value) = item.rocksdb_err()?;
        // `prefix_iterator_cf` can run past the prefix; stop when it does.
        let Some(suffix) = key.strip_prefix(prefix.as_slice()) else {
            break;
        };
        // {block_uuid}\0{locale}\0{~revision} — split on BYTES. The trailing
        // encoded revision is not valid UTF-8, so decoding the whole suffix first
        // throws the entry away; only the two leading segments are text.
        let mut parts = suffix.splitn(3, |b| *b == 0);
        let (Some(block_uuid), Some(locale)) = (parts.next(), parts.next()) else {
            continue;
        };
        let (Ok(block_uuid), Ok(locale)) =
            (std::str::from_utf8(block_uuid), std::str::from_utf8(locale))
        else {
            continue;
        };
        // `orphaned` occupies the locale position but is a marker, not a locale.
        if locale == "orphaned" {
            continue;
        }
        if let Ok(locale) = LocaleCode::parse(locale) {
            found.insert((block_uuid.to_string(), locale));
        }
    }

    Ok(found.into_iter().collect())
}

/// Mark blocks as orphaned
pub(super) async fn mark_blocks_orphaned(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    block_uuids: &[String],
    revision: &HLC,
) -> Result<()> {
    let cf = crate::cf_handle(db, crate::cf::BLOCK_TRANSLATIONS)?;

    // Store an orphaned marker for each block UUID
    let orphaned_marker = serde_json::to_vec(&serde_json::json!({
        "orphaned": true,
        "orphaned_at_revision": revision
    }))
    .map_err(|e| {
        raisin_error::Error::storage(format!("Failed to serialize orphan marker: {}", e))
    })?;

    for block_uuid in block_uuids {
        // We'll store an orphan marker with a special key suffix
        let mut key = format!(
            "{}\0{}\0{}\0{}\0block_trans\0{}\0{}\0orphaned\0",
            tenant_id, repo_id, branch, workspace, node_id, block_uuid
        )
        .into_bytes();

        key.extend_from_slice(&crate::keys::encode_descending_revision(revision));

        db.put_cf(&cf, key, &orphaned_marker).rocksdb_err()?;
    }

    Ok(())
}
