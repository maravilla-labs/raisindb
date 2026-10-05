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
use crate::translation_write::OverlayTarget;

/// Store a translation (locale overlay) for a node
///
/// # Translation Storage
///
/// Staged through `translation_write::stage_version` — the version in
/// TRANSLATION_DATA and its TRANSLATION_INDEX entry, in the same format every
/// other writer uses. The commit captures it for replication.
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

    // Localized node name uniqueness (plan Phase 13c), when the repository
    // enforces it: checked HERE, before anything is staged, against stored
    // state and everything this transaction wrote so far, so a refused
    // overlay leaves the transaction untouched (a package install rejects
    // that one entry, not its whole batch). The commit checks again under
    // the branch lock, against the final view.
    tx.check_overlay_unique(
        (tenant_id.as_str(), repo_id.as_str(), branch.as_str()),
        workspace,
        node_id,
        locale,
        &overlay,
        &revision,
    )?;

    // 3. Stage the version and its index entry through the one writer.
    let key = {
        let mut batch = tx
            .batch
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        let target = OverlayTarget {
            tenant_id: &tenant_id,
            repo_id: &repo_id,
            branch: &branch,
            workspace,
            node_id,
            block_uuid: None,
            locale,
        };
        crate::translation_write::stage_version(
            &tx.db,
            &mut batch,
            &target,
            Some(&overlay),
            &revision,
        )?
    };

    // 6. Update read cache for read-your-writes
    {
        let mut cache = tx
            .read_cache
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        cache.put_translation(workspace, node_id, locale, Some(overlay));
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

/// The translation half of deleting `node_id` inside this transaction, for
/// the overlays (node and block) THIS transaction wrote. Stored overlays need nothing: the
/// node delete ends them by the read rule (`translation_read`). These sit in
/// the read cache, which later reads in this transaction and the commit's
/// capture replay, so each gets `T` at the delete's revision — the same key as
/// the version when both share the transaction revision, so the later put wins
/// and the commit holds the tombstone — and the cache records the deletion,
/// so what the commit captures for replication is `T` too.
pub(crate) fn tombstone_own_writes(
    tx: &RocksDBTransaction,
    batch: &mut rocksdb::WriteBatch,
    scope: (&str, &str, &str, &str),
    node_id: &str,
    revision: &raisin_hlc::HLC,
) -> Result<()> {
    let (tenant_id, repo_id, branch, workspace) = scope;
    let mut cache = tx
        .read_cache
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
    for ((ws, id, locale), overlay) in cache.translations.iter_mut() {
        if ws != workspace || id != node_id || overlay.is_none() {
            continue;
        }
        let target = OverlayTarget {
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            block_uuid: None,
            locale,
        };
        crate::translation_write::stage_version(&tx.db, batch, &target, None, revision)?;
        *overlay = None;
    }
    // Block overlays alike (plan Phase 11c): the delete funnel's block
    // materialization reads stored versions only, so the ones this
    // transaction wrote get their `T` here, and the commit captures `T`.
    for ((ws, id, block, locale), overlay) in cache.block_translations.iter_mut() {
        if ws != workspace || id != node_id || overlay.is_none() {
            continue;
        }
        let target = OverlayTarget {
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            block_uuid: Some(block),
            locale,
        };
        crate::translation_write::stage_version(&tx.db, batch, &target, None, revision)?;
        *overlay = None;
    }
    Ok(())
}
