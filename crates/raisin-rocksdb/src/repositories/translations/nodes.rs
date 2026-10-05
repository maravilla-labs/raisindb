//! Node-level translation CRUD operations.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::translations::{LocaleCode, LocaleOverlay, TranslationMeta};
use rocksdb::{WriteBatch, DB};
use std::sync::Arc;

use crate::translation_write::{self, OverlayTarget};

use super::{replication, revision};

/// Get a node-level translation as of `revision` (plan Phase 11 item 1: it
/// used to read HEAD whatever it was asked for, so time travel with a locale
/// showed today's translation).
#[allow(clippy::too_many_arguments)]
pub(super) async fn get_translation(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    locale: &LocaleCode,
    revision: &HLC,
) -> Result<Option<LocaleOverlay>> {
    crate::translation_history::ensure_complete_at(db, tenant_id, repo_id, branch, revision)?;
    crate::translation_read::read_overlay(
        db,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        locale.as_str(),
        Some(revision),
    )
}

/// Store a node-level translation: the version, its index entry, its meta,
/// its snapshot and the revision meta in ONE `WriteBatch` (they were five
/// separate puts), then capture it for replication.
#[allow(clippy::too_many_arguments)]
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
    let target = OverlayTarget {
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        block_uuid: None,
        locale: locale.as_str(),
    };
    // Localized node name uniqueness, when the repository enforces it (plan
    // Phase 12): an overlay's `/__node_name` must not collide with a
    // sibling's. Checked now, and again at the commit step UNDER THE BRANCH
    // LOCK (`localized_name::unique::deferred`): every HTTP `/translations`,
    // `raisin:cmd/translate`, WS and `TranslationService` write funnels
    // through here, and a check outside the lock let two of them — or one and
    // a transaction commit — store the same name on two siblings.
    let names = crate::localized_name::keys::NameScope::new(tenant_id, repo_id, branch, workspace);
    let name_check = match crate::localized_name::sync::node_at(db, names, node_id, None)? {
        Some((_, Some(node))) => crate::localized_name::unique::NameCheck::staged(
            db,
            names,
            &node,
            None,
            &meta.revision,
            crate::localized_name::sync::Overrides::from([(
                locale.as_str().to_string(),
                Some(overlay.clone()),
            )]),
        )?,
        _ => None,
    };
    let batch = translation_batch(db, &target, overlay, meta)?;
    // The node is locked like any write of it; the branch lock is taken
    // only when a name check is carried.
    let mut commit = crate::indexing::NodeCommit::new(tenant_id, repo_id, branch);
    commit.touch(node_id).check_name(name_check);
    commit.write(db, batch).await?;

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

/// Everything one repository translation write stores, as one batch.
pub(super) fn translation_batch(
    db: &DB,
    target: &OverlayTarget<'_>,
    overlay: &LocaleOverlay,
    meta: &TranslationMeta,
) -> Result<WriteBatch> {
    let mut batch = WriteBatch::default();
    translation_write::stage_version(db, &mut batch, target, Some(overlay), &meta.revision)?;
    translation_write::stage_history(db, &mut batch, target, Some(overlay), meta)?;
    revision::stage_revision_meta(db, &mut batch, target, overlay, meta)?;
    Ok(batch)
}

/// List the locales of a node as of `revision`: one entry per locale whose
/// newest version at or before it is live — the rule [`get_translation`]
/// applies to a single locale, through the same reader.
pub(super) async fn list_translations_for_node(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    revision: &HLC,
) -> Result<Vec<LocaleCode>> {
    crate::translation_history::ensure_complete_at(db, tenant_id, repo_id, branch, revision)?;
    let locales = crate::translation_read::live_locales(
        db,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        Some(revision),
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
