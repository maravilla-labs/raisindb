//! Translation helpers for COPY: collecting a node's overlays through the one
//! translation reader (`crate::translation_read`), and staging the copied
//! versions through the one writer (`crate::translation_write`).
//!
//! The collectors used to scan `TRANSLATION_DATA` / `BLOCK_TRANSLATIONS` by
//! hand — a second reader — taking the newest key with no revision bound, and
//! the block scan decoded the orphan marker (`…\0{block}\0orphaned\0{~rev}`,
//! not an overlay) as a `LocaleOverlay`, so one `mark_blocks_orphaned` call
//! made every later publish or tree copy of that node fail.

use super::super::NodeRepositoryImpl;
use crate::repositories::translations::replication;
use crate::translation_write::OverlayTarget;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::translations::{LocaleCode, LocaleOverlay, TranslationMeta};

/// A single block translation entry: (block_uuid, locale, overlay, parent_revision).
type BlockTranslationEntry = (String, LocaleCode, LocaleOverlay, Option<HLC>);

impl NodeRepositoryImpl {
    /// Stage one copied version — data, index entry, meta and snapshot; `None`
    /// is a deletion — through the one translation writer, and return the op
    /// that replicates it (a copy wrote these and never captured them, so a
    /// replica's copy or publish arrived untranslated).
    pub(in crate::repositories::nodes) fn stage_copied_translation(
        &self,
        batch: &mut rocksdb::WriteBatch,
        target: &OverlayTarget<'_>,
        overlay: Option<&LocaleOverlay>,
        meta: &TranslationMeta,
    ) -> Result<raisin_replication::OpType> {
        crate::translation_write::stage_version(&self.db, batch, target, overlay, &meta.revision)?;
        crate::translation_write::stage_history(&self.db, batch, target, overlay, meta)?;
        Ok(replication::translation_op(
            &replication::TranslationVersionOp {
                target: *target,
                overlay,
                revision: meta.revision,
                history_complete_from: None,
            },
        ))
    }

    /// Every live node-level overlay of `node_id` as of `max_revision` (the
    /// newest at all when `None`): `(locale, overlay, revision it was read at)`.
    pub(in crate::repositories::nodes) fn collect_node_translations_for_copy(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<(LocaleCode, LocaleOverlay, Option<HLC>)>> {
        let mut out = Vec::new();
        for locale in crate::translation_read::live_locales(
            &self.db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            max_revision,
        )? {
            let Some(version) = crate::translation_read::read_version(
                &self.db,
                tenant_id,
                repo_id,
                branch,
                workspace,
                node_id,
                &locale,
                max_revision,
            )?
            else {
                continue;
            };
            let Some(overlay) = version.overlay else {
                continue;
            };
            out.push((
                parse_locale(&locale, node_id)?,
                overlay,
                crate::keys::extract_revision_from_key(&version.key).ok(),
            ));
        }
        Ok(out)
    }

    /// Every live block overlay of `node_id` as of `max_revision`:
    /// `(block_uuid, locale, overlay, revision it was read at)`. Orphan
    /// markers are not overlays and are never returned.
    pub(in crate::repositories::nodes) fn collect_block_translations_for_copy(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<BlockTranslationEntry>> {
        // One scan and one `NODES` walk for the whole node, not one per block.
        let mut out = Vec::new();
        for version in crate::translation_read::live_block_versions(
            &self.db,
            (tenant_id, repo_id, branch, workspace),
            node_id,
            max_revision,
            |_| true,
        )? {
            out.push((
                version.block_uuid,
                parse_locale(&version.locale, node_id)?,
                version.overlay,
                Some(version.revision),
            ));
        }
        Ok(out)
    }
}

fn parse_locale(locale: &str, node_id: &str) -> Result<LocaleCode> {
    LocaleCode::parse(locale).map_err(|e| {
        raisin_error::Error::storage(format!(
            "Invalid locale code {} on node {}: {}",
            locale, node_id, e
        ))
    })
}
