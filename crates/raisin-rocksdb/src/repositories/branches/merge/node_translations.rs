//! A NODE resolution carries the kept side's translation overlays to the
//! merge revision (plan Phase 11; the Phase 2 item 10 gap).
//!
//! The copy after the resolutions replays the SOURCE's translation versions
//! at their original revisions. A node resolved by keeping the target's
//! version over a source DELETE therefore lost every overlay: the delete's
//! `T` versions are newer than the target's overlays. Now, like every index
//! of the node, the overlays get a full put at M — each locale and block
//! overlay live on the kept side at its head — and every overlay live on
//! either side but not on the kept one gets a tombstone at M (the union, as
//! for the property index). A deletion keeps nothing, so it tombstones the
//! union.

use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_models::translations::{LocaleCode, LocaleOverlay, TranslationMeta};
use rocksdb::{WriteBatch, DB};
use std::collections::BTreeMap;

use super::super::BranchRepositoryImpl;
use super::translations::MergeSides;
use crate::repositories::translations::replication;
use crate::translation_write::{self, OverlayTarget};

/// `(block_uuid, locale)` → overlay (`None`: tombstone).
type Overlays = BTreeMap<(Option<String>, String), Option<LocaleOverlay>>;

/// Every overlay of `node_id` live on `branch` at `head`.
fn live_overlays(
    db: &DB,
    scope: (&str, &str, &str),
    node_id: &str,
    (branch, head): (&str, &HLC),
) -> Result<Overlays> {
    let (tenant_id, repo_id, workspace) = scope;
    let mut found = Overlays::new();
    for locale in crate::translation_read::live_locales(
        db,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        Some(head),
    )? {
        let overlay = crate::translation_read::read_overlay(
            db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            &locale,
            Some(head),
        )?;
        found.insert((None, locale), overlay);
    }
    for version in crate::translation_read::live_block_versions(
        db,
        (tenant_id, repo_id, branch, workspace),
        node_id,
        Some(head),
        |_| true,
    )? {
        found.insert(
            (Some(version.block_uuid), version.locale),
            Some(version.overlay),
        );
    }
    Ok(found)
}

impl BranchRepositoryImpl {
    /// Write, at `merge_revision` on the target, the overlays of the side the
    /// resolution kept (`kept`; `None` for a deletion), and tombstone those
    /// either side has that it does not. A locale with its own translation
    /// resolution in this merge (`explicit`, `{locale}` / `{locale}::{block}`)
    /// is left to it.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn write_resolved_node_translations(
        &self,
        tenant_id: &str,
        repo_id: &str,
        workspace: &str,
        node_id: &str,
        sides: &MergeSides<'_>,
        kept: Option<(&str, &HLC)>,
        explicit: &std::collections::HashSet<(String, String)>,
        merge_revision: &HLC,
        actor: &str,
        message: &str,
    ) -> Result<()> {
        let scope = (tenant_id, repo_id, workspace);
        let mut versions = match kept {
            Some(side) => live_overlays(&self.db, scope, node_id, side)?,
            None => Overlays::new(),
        };
        for side in [sides.target, sides.source] {
            for key in live_overlays(&self.db, scope, node_id, side)?.into_keys() {
                versions.entry(key).or_insert(None);
            }
        }
        versions.retain(|(block, locale), _| {
            let key = match block {
                Some(block) => format!("{locale}::{block}"),
                None => locale.clone(),
            };
            !explicit.contains(&(node_id.to_string(), key))
        });
        if versions.is_empty() {
            return Ok(());
        }

        let target_branch = sides.target.0;
        let mut batch = WriteBatch::default();
        for ((block, locale), overlay) in &versions {
            let target = OverlayTarget {
                tenant_id,
                repo_id,
                branch: target_branch,
                workspace,
                node_id,
                block_uuid: block.as_deref(),
                locale,
            };
            translation_write::stage_version(
                &self.db,
                &mut batch,
                &target,
                overlay.as_ref(),
                merge_revision,
            )?;
            if let Ok(locale_code) = LocaleCode::parse(locale) {
                let meta = TranslationMeta {
                    locale: locale_code,
                    revision: *merge_revision,
                    parent_revision: Some(*sides.target.1),
                    timestamp: chrono::Utc::now(),
                    actor: actor.to_string(),
                    message: message.to_string(),
                    is_system: false,
                };
                translation_write::stage_history(
                    &self.db,
                    &mut batch,
                    &target,
                    overlay.as_ref(),
                    &meta,
                )?;
            }
        }
        self.db
            .write(batch)
            .map_err(|e| Error::storage(e.to_string()))?;

        for ((block, locale), overlay) in &versions {
            replication::capture_version(
                self.operation_capture.as_ref(),
                &replication::TranslationVersionOp {
                    target: OverlayTarget {
                        tenant_id,
                        repo_id,
                        branch: target_branch,
                        workspace,
                        node_id,
                        block_uuid: block.as_deref(),
                        locale,
                    },
                    overlay: overlay.as_ref(),
                    revision: *merge_revision,
                    history_complete_from: None,
                },
                actor,
            )
            .await;
        }
        Ok(())
    }
}
