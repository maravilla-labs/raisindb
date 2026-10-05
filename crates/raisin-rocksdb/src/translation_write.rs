//! The one writer of translation versions (plan Phase 11 item 2).
//!
//! Every write of a `TRANSLATION_DATA` / `BLOCK_TRANSLATIONS` version — the
//! repository, the transaction, a copy, a merge resolution, the replication
//! apply arm, history GC — stages it into the caller's `WriteBatch`
//! through [`stage_version`], so the version and its `TRANSLATION_INDEX`
//! entry land together and in ONE format. The repository used to issue five
//! separate `put_cf`s (a crash between them left an overlay without its index
//! entry, or a revision meta naming a change that was never stored), and the
//! transaction wrote a different index value than the repository.
//!
//! `TRANSLATION_INDEX` (`{tenant}\0{repo}\0translation_index\0{locale}\0{~rev}\0{node_id}`)
//! gets `keys::INDEX_LIVE` for a live version and `T` for a tombstone, so the
//! "nodes translated into L" listing is revision-correct too. Block overlays
//! have no index entry (they never had one).
//!
//! A LIVE block version stored below a delete of its node also gets that
//! delete's `T` here ([`block_deletion`], plan Phase 11c), so the stored
//! history agrees with the read rule whatever order the two arrived in.

use crate::repositories::translations::keys;
use crate::{cf, cf_handle};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::translations::{LocaleOverlay, TranslationMeta};
use rocksdb::{WriteBatch, DB};

mod block_deletion;
mod dead_generation;

pub(crate) use block_deletion::{block_deletion_keys, materialize_block_deletion};
pub(crate) use dead_generation::materialize_dead_generation;

/// Which overlay a version belongs to.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OverlayTarget<'a> {
    pub tenant_id: &'a str,
    pub repo_id: &'a str,
    pub branch: &'a str,
    pub workspace: &'a str,
    pub node_id: &'a str,
    /// `Some` for a block overlay.
    pub block_uuid: Option<&'a str>,
    pub locale: &'a str,
}

impl OverlayTarget<'_> {
    /// The version key at `revision`.
    pub(crate) fn data_key(&self, revision: &HLC) -> Vec<u8> {
        match self.block_uuid {
            None => keys::translation_key(
                self.tenant_id,
                self.repo_id,
                self.branch,
                self.workspace,
                self.node_id,
                self.locale,
                revision,
            ),
            Some(block) => keys::block_translation_key(
                self.tenant_id,
                self.repo_id,
                self.branch,
                self.workspace,
                self.node_id,
                block,
                self.locale,
                revision,
            ),
        }
    }

    /// `{locale}` or `{locale}::{block_uuid}` — how revision metas and
    /// snapshots name a translation change.
    pub(crate) fn locale_key(&self) -> String {
        match self.block_uuid {
            None => self.locale.to_string(),
            Some(block) => format!("{}::{}", self.locale, block),
        }
    }
}

/// The stored bytes of a version: JSON, or `T` for a tombstone.
pub(crate) fn encode_overlay(overlay: Option<&LocaleOverlay>) -> Result<Vec<u8>> {
    match overlay {
        Some(overlay) => serde_json::to_vec(overlay).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to serialize LocaleOverlay: {}", e))
        }),
        None => Ok(crate::keys::TOMBSTONE_VALUE.to_vec()),
    }
}

/// Stage one version of `target` at `revision` (`None` = tombstone): the
/// data key and, for a node overlay, its index entry. Returns the data key
/// (what a transaction records as its write).
pub(crate) fn stage_version(
    db: &DB,
    batch: &mut WriteBatch,
    target: &OverlayTarget<'_>,
    overlay: Option<&LocaleOverlay>,
    revision: &HLC,
) -> Result<Vec<u8>> {
    let key = target.data_key(revision);
    let bytes = encode_overlay(overlay)?;
    match target.block_uuid {
        None => {
            batch.put_cf(cf_handle(db, cf::TRANSLATION_DATA)?, &key, bytes);
            let index_key = keys::translation_index_key(
                target.tenant_id,
                target.repo_id,
                target.locale,
                revision,
                target.node_id,
            );
            let index_value = match overlay {
                Some(_) => keys::INDEX_LIVE,
                None => crate::keys::TOMBSTONE_VALUE,
            };
            batch.put_cf(
                cf_handle(db, cf::TRANSLATION_INDEX)?,
                index_key,
                index_value,
            );
            // A `/__node_name` overlay (or a `Hidden` one) changes the node's
            // localized segment: the localized name index rides on the
            // version (plan Phase 12).
            crate::localized_name::sync::sync_after_overlay(
                db,
                batch,
                crate::localized_name::keys::NameScope::new(
                    target.tenant_id,
                    target.repo_id,
                    target.branch,
                    target.workspace,
                ),
                target.node_id,
                target.locale,
                overlay,
                revision,
            )?;
        }
        Some(_) => {
            batch.put_cf(cf_handle(db, cf::BLOCK_TRANSLATIONS)?, &key, bytes);
            if overlay.is_some() {
                // After the version: at the same revision the `T` wins.
                block_deletion::stage_late_version(db, batch, target, revision)?;
            }
        }
    }
    Ok(key)
}

/// Stage the history records of a version: its `TranslationMeta` (node
/// overlays; `get_translation_meta`) and its snapshot (live versions; the
/// `rev/{revision}` translation snapshot reads).
pub(crate) fn stage_history(
    db: &DB,
    batch: &mut WriteBatch,
    target: &OverlayTarget<'_>,
    overlay: Option<&LocaleOverlay>,
    meta: &TranslationMeta,
) -> Result<()> {
    let cf_revisions = cf_handle(db, cf::REVISIONS)?;
    if target.block_uuid.is_none() {
        let meta_bytes = serde_json::to_vec(meta).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to serialize TranslationMeta: {}", e))
        })?;
        let meta_key = keys::translation_meta_key(
            target.tenant_id,
            target.repo_id,
            target.branch,
            target.workspace,
            target.node_id,
            target.locale,
            &meta.revision,
        );
        batch.put_cf(cf_revisions, meta_key, meta_bytes);
    }
    if let Some(overlay) = overlay {
        let snapshot_key = crate::keys::translation_snapshot_key(
            target.tenant_id,
            target.repo_id,
            target.node_id,
            &target.locale_key(),
            &meta.revision,
        );
        batch.put_cf(cf_revisions, snapshot_key, encode_overlay(Some(overlay))?);
    }
    Ok(())
}

/// History GC is about to delete the `NODES` tombstone of `node_id` at
/// `deleted_at` — the evidence `translation_read`'s read rule ends an
/// overlay with. Stage what that delete means for the stored versions first:
/// `T` at `deleted_at` for every locale, and every block overlay
/// ([`materialize_block_deletion`]), whose newest stored version at or
/// before it is live — and `T` over every version written into the dead
/// generation that delete opened, up to `next_record` (the node's next
/// `NODES` record above it, as stored before this run;
/// [`materialize_dead_generation`]). Reads answer exactly what they did with
/// the tombstone, and retention then keeps the `T` as the newest version at
/// or below its cutoff. Only history GC calls this; no write path derives
/// NODE-overlay tombstones from a node delete (the delete funnel
/// materializes block overlays only — `tombstones` step 13).
pub(crate) fn materialize_node_deletion(
    db: &DB,
    batch: &mut WriteBatch,
    scope: (&str, &str, &str, &str),
    node_id: &str,
    deleted_at: &HLC,
    next_record: Option<&HLC>,
) -> Result<usize> {
    let (tenant_id, repo_id, branch, workspace) = scope;
    let locales = crate::translation_read::stored_live_locales(
        db,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        Some(deleted_at),
    )?;
    for (locale, _) in &locales {
        let target = OverlayTarget {
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            block_uuid: None,
            locale,
        };
        stage_version(db, batch, &target, None, deleted_at)?;
    }
    let blocks = materialize_block_deletion(db, batch, scope, node_id, deleted_at)?;
    let dead = materialize_dead_generation(db, batch, scope, node_id, deleted_at, next_record)?;
    Ok(locales.len() + blocks + dead)
}
