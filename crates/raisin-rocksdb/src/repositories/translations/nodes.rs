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
    // HEAD read: the newest version decides, and a tombstone there means the
    // translation is deleted. One reader shared with the transaction path.
    crate::translation_read::read_overlay(
        db,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        locale.as_str(),
        None,
    )
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
///
/// One entry per locale whose NEWEST version is live — the same rule
/// [`get_translation`] applies to a single locale, and the same reader the
/// transaction path uses (`crate::translation_read`). Like `get_translation`,
/// this reads HEAD; the revision is unused.
pub(super) async fn list_translations_for_node(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    _revision: &HLC,
) -> Result<Vec<LocaleCode>> {
    let locales = crate::translation_read::live_locales(
        db, tenant_id, repo_id, branch, workspace, node_id, None,
    )?;

    Ok(locales
        .into_iter()
        .filter_map(|locale| match LocaleCode::parse(&locale) {
            Ok(code) => Some(code),
            Err(e) => {
                tracing::warn!(
                    node_id,
                    "Skipping translation with an unparseable locale '{}': {}",
                    locale,
                    e
                );
                None
            }
        })
        .collect())
}
