//! The RevisionMeta of a repository translation write, staged into its batch.

use raisin_error::Result;
use raisin_models::translations::{LocaleOverlay, TranslationMeta};
use raisin_storage::{NodeChangeInfo, RevisionMeta};
use rocksdb::{WriteBatch, DB};

use crate::translation_write::OverlayTarget;

use super::serialization;

/// Create a RevisionMeta for a translation change
///
/// This allows translation changes to appear in revision lists with locale
/// information (`{locale}` for a node overlay, `{locale}::{block_uuid}` for
/// a block overlay).
fn create_revision_meta(
    meta: &TranslationMeta,
    target: &OverlayTarget<'_>,
    overlay: &LocaleOverlay,
) -> RevisionMeta {
    let change_operation = match overlay {
        LocaleOverlay::Hidden => raisin_models::tree::ChangeOperation::Deleted,
        LocaleOverlay::Properties { .. } => raisin_models::tree::ChangeOperation::Modified,
    };

    let node_change_info = NodeChangeInfo {
        node_id: target.node_id.to_string(),
        workspace: target.workspace.to_string(),
        operation: change_operation,
        translation_locale: Some(target.locale_key()),
    };

    RevisionMeta {
        revision: meta.revision,
        parent: meta.parent_revision,
        merge_parent: None,
        branch: target.branch.to_string(),
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

/// Stage the RevisionMeta of one translation write.
pub(super) fn stage_revision_meta(
    db: &DB,
    batch: &mut WriteBatch,
    target: &OverlayTarget<'_>,
    overlay: &LocaleOverlay,
    meta: &TranslationMeta,
) -> Result<()> {
    let cf_meta = crate::cf_handle(db, crate::cf::REVISIONS)?;
    let revision_meta = create_revision_meta(meta, target, overlay);
    let revision_key =
        crate::keys::revision_meta_key(target.tenant_id, target.repo_id, &meta.revision);
    batch.put_cf(
        cf_meta,
        revision_key,
        serialization::serialize_revision_meta(&revision_meta)?,
    );
    Ok(())
}
