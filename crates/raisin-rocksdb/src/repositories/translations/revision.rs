//! RevisionMeta creation and snapshot storage helpers.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::translations::{LocaleOverlay, TranslationMeta};
use raisin_storage::{NodeChangeInfo, RevisionMeta};
use rocksdb::DB;
use std::sync::Arc;

use crate::error_ext::ResultExt;

use super::serialization;

/// Create a RevisionMeta for a translation change
///
/// This allows translation changes to appear in revision lists with locale information.
fn create_revision_meta(
    meta: &TranslationMeta,
    branch: &str,
    node_id: &str,
    workspace: &str,
    translation_locale: String,
    overlay: &LocaleOverlay,
) -> RevisionMeta {
    let change_operation = match overlay {
        LocaleOverlay::Hidden => raisin_models::tree::ChangeOperation::Deleted,
        LocaleOverlay::Properties { .. } => raisin_models::tree::ChangeOperation::Modified,
    };

    let node_change_info = NodeChangeInfo {
        node_id: node_id.to_string(),
        workspace: workspace.to_string(),
        operation: change_operation,
        translation_locale: Some(translation_locale),
    };

    RevisionMeta {
        revision: meta.revision,
        parent: meta.parent_revision,
        merge_parent: None,
        branch: branch.to_string(),
        timestamp: meta.timestamp,
        actor: meta.actor.clone(),
        message: meta.message.clone(),
        is_system: meta.is_system,
        changed_nodes: vec![node_change_info],
        changed_node_types: Vec::new(),
        changed_archetypes: Vec::new(),
        changed_element_types: Vec::new(),
        operation: None, // Translation operations tracked separately via TranslationMeta
    }
}

/// Store RevisionMeta for a node-level translation change
pub(super) fn store_node_revision_meta(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node_id: &str,
    workspace: &str,
    locale: &str,
    overlay: &LocaleOverlay,
    meta: &TranslationMeta,
) -> Result<()> {
    let cf_meta = crate::cf_handle(db, crate::cf::REVISIONS)?;

    let revision_meta = create_revision_meta(
        meta,
        branch,
        node_id,
        workspace,
        locale.to_string(),
        overlay,
    );

    let revision_key = crate::keys::revision_meta_key(tenant_id, repo_id, &meta.revision);
    let revision_meta_bytes = serialization::serialize_revision_meta(&revision_meta)?;

    db.put_cf(&cf_meta, revision_key, revision_meta_bytes)
        .rocksdb_err()?;

    Ok(())
}

/// Store RevisionMeta for a block-level translation change
pub(super) fn store_block_revision_meta(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node_id: &str,
    workspace: &str,
    locale: &str,
    block_uuid: &str,
    overlay: &LocaleOverlay,
    meta: &TranslationMeta,
) -> Result<()> {
    let cf_meta = crate::cf_handle(db, crate::cf::REVISIONS)?;

    // Block translations use "{locale}::{block_uuid}" format to track specific block changes
    let translation_locale = format!("{}::{}", locale, block_uuid);

    let revision_meta = create_revision_meta(
        meta,
        branch,
        node_id,
        workspace,
        translation_locale,
        overlay,
    );

    let revision_key = crate::keys::revision_meta_key(tenant_id, repo_id, &meta.revision);
    let revision_meta_bytes = serialization::serialize_revision_meta(&revision_meta)?;

    db.put_cf(&cf_meta, revision_key, revision_meta_bytes)
        .rocksdb_err()?;

    Ok(())
}

/// Store translation snapshot for time-travel queries and rollback
///
/// Snapshots enable retrieving exact translation state at any past revision.
pub(super) fn store_snapshot(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    node_id: &str,
    locale_key: &str,
    revision: &HLC,
    overlay_bytes: &[u8],
) -> Result<()> {
    let cf_meta = crate::cf_handle(db, crate::cf::REVISIONS)?;

    let snapshot_key =
        crate::keys::translation_snapshot_key(tenant_id, repo_id, node_id, locale_key, revision);

    db.put_cf(&cf_meta, snapshot_key, overlay_bytes)
        .rocksdb_err()?;

    Ok(())
}
