//! Writing a resolved TRANSLATION conflict at the merge revision (plan
//! Phase 11; closes the Phase 2 item 10 gap).
//!
//! A translation conflict is keyed `(node_id, locale)` (`{locale}::{block}`
//! for a block overlay). Merge apply used to send it down the NODE path:
//! nothing was written for the overlay — the copy then replayed the source's
//! versions and the newest simply won, so `keep-ours` was a no-op whenever the
//! source edit was later — while the node itself was rewritten at M with
//! whichever side's CONTENT the resolution named, though nobody had disputed
//! the content. Now the resolution writes exactly one translation version at
//! M through the one translation writer (the chosen overlay, or a tombstone),
//! which shadows every version either side or the copy holds, and the node is
//! left alone.

use raisin_context::{ConflictResolution, ResolutionType};
use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::translations::{JsonPointer, LocaleCode, LocaleOverlay, TranslationMeta};
use rocksdb::WriteBatch;
use std::collections::HashMap;

use super::super::BranchRepositoryImpl;
use crate::repositories::translations::replication;
use crate::translation_write::{self, OverlayTarget};

/// The two sides of a merge: `(branch, head)`.
pub(super) struct MergeSides<'a> {
    pub target: (&'a str, &'a HLC),
    pub source: (&'a str, &'a HLC),
}

impl BranchRepositoryImpl {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn write_resolved_translation(
        &self,
        tenant_id: &str,
        repo_id: &str,
        workspace: &str,
        sides: &MergeSides<'_>,
        resolution: &ConflictResolution,
        locale_key: &str,
        merge_revision: &HLC,
        actor: &str,
        message: &str,
    ) -> Result<()> {
        let (locale, block_uuid) = match locale_key.split_once("::") {
            Some((locale, block)) => (locale, Some(block)),
            None => (locale_key, None),
        };
        let read_side = |(branch, head): (&str, &HLC)| -> Result<Option<LocaleOverlay>> {
            let version = match block_uuid {
                Some(block) => crate::translation_read::read_block_version(
                    &self.db,
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &resolution.node_id,
                    block,
                    locale,
                    Some(head),
                )?,
                None => crate::translation_read::read_version(
                    &self.db,
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &resolution.node_id,
                    locale,
                    Some(head),
                )?,
            };
            Ok(version.and_then(|v| v.overlay))
        };
        let chosen = match resolution.resolution_type {
            ResolutionType::KeepOurs => read_side(sides.target)?,
            ResolutionType::KeepTheirs => read_side(sides.source)?,
            ResolutionType::Manual => {
                manual_overlay(&resolution.resolved_properties).map_err(|e| {
                    Error::Validation(format!(
                        "Invalid translation overlay for node {} ({}): {}",
                        resolution.node_id, locale_key, e
                    ))
                })?
            }
        };

        let target = OverlayTarget {
            tenant_id,
            repo_id,
            branch: sides.target.0,
            workspace,
            node_id: &resolution.node_id,
            block_uuid,
            locale,
        };
        let mut batch = WriteBatch::default();
        translation_write::stage_version(
            &self.db,
            &mut batch,
            &target,
            chosen.as_ref(),
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
                chosen.as_ref(),
                &meta,
            )?;
        }
        self.db
            .write(batch)
            .map_err(|e| Error::storage(e.to_string()))?;

        replication::capture_version(
            self.operation_capture.as_ref(),
            &replication::TranslationVersionOp {
                target,
                overlay: chosen.as_ref(),
                revision: *merge_revision,
                history_complete_from: None,
            },
            actor,
        )
        .await;
        Ok(())
    }
}

/// A manual resolution's payload: `null` deletes the locale; a stored
/// overlay (`{"type": "properties", …}` / `{"type": "hidden"}`, what conflict
/// detection shows as each side) is taken as is; any other object is the
/// overlay's pointer → value map.
fn manual_overlay(value: &serde_json::Value) -> std::result::Result<Option<LocaleOverlay>, String> {
    if value.is_null() {
        return Ok(None);
    }
    if let Ok(overlay) = serde_json::from_value::<LocaleOverlay>(value.clone()) {
        return Ok(Some(overlay));
    }
    let data: HashMap<JsonPointer, PropertyValue> =
        serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    Ok(Some(LocaleOverlay::Properties { data }))
}
