//! Block-level translation CRUD operations.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::translations::{LocaleCode, LocaleOverlay, TranslationMeta};
use rocksdb::DB;
use std::sync::Arc;

use crate::error_ext::ResultExt;
use crate::translation_write::OverlayTarget;

use super::{keys, replication};

/// Get a block-level translation as of `revision` — the same reader and
/// rule as a node overlay (it used to take the first key: HEAD, and a
/// tombstone failed to decode).
#[allow(clippy::too_many_arguments)]
pub(super) async fn get_block_translation(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    block_uuid: &str,
    locale: &LocaleCode,
    revision: &HLC,
) -> Result<Option<LocaleOverlay>> {
    crate::translation_history::ensure_complete_at(db, tenant_id, repo_id, branch, revision)?;
    Ok(crate::translation_read::read_block_version(
        db,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        block_uuid,
        locale.as_str(),
        Some(revision),
    )?
    .and_then(|version| version.overlay))
}

/// Every live block overlay of this node in one of `locales` as of
/// `revision` — one scan and one `NODES` walk for the node
/// (`translation_read::live_block_versions`).
#[allow(clippy::too_many_arguments)]
pub(super) async fn get_block_translations_for_node(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    locales: &[LocaleCode],
    revision: &HLC,
) -> Result<Vec<(String, LocaleCode, LocaleOverlay)>> {
    if locales.is_empty() {
        return Ok(Vec::new());
    }
    crate::translation_history::ensure_complete_at(db, tenant_id, repo_id, branch, revision)?;
    let versions = crate::translation_read::live_block_versions(
        db,
        (tenant_id, repo_id, branch, workspace),
        node_id,
        Some(revision),
        |locale| locales.iter().any(|wanted| wanted.as_str() == locale),
    )?;
    Ok(versions
        .into_iter()
        .filter_map(|version| {
            LocaleCode::parse(&version.locale)
                .ok()
                .map(|locale| (version.block_uuid, locale, version.overlay))
        })
        .collect())
}

/// Store a block-level translation: version, snapshot and revision meta in
/// ONE `WriteBatch`, then capture it for replication.
#[allow(clippy::too_many_arguments)]
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
    let target = OverlayTarget {
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        block_uuid: Some(block_uuid),
        locale: locale.as_str(),
    };
    let batch = super::nodes::translation_batch(db, &target, overlay, meta)?;
    db.write(batch).rocksdb_err()?;

    replication::capture_version(
        operation_capture,
        &replication::TranslationVersionOp {
            target,
            overlay: Some(overlay),
            revision: meta.revision,
            history_complete_from: None,
        },
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
    // HEAD (the trait takes no revision); a block whose newest version is a
    // tombstone is not listed.
    let found = crate::translation_read::live_block_overlays(
        db, tenant_id, repo_id, branch, workspace, node_id, None,
    )?;
    Ok(found
        .into_iter()
        .filter_map(|(block_uuid, locale)| {
            LocaleCode::parse(&locale)
                .ok()
                .map(|locale| (block_uuid, locale))
        })
        .collect())
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
        let key = keys::block_orphan_key(
            tenant_id, repo_id, branch, workspace, node_id, block_uuid, revision,
        );

        db.put_cf(&cf, key, &orphaned_marker).rocksdb_err()?;
    }

    Ok(())
}
