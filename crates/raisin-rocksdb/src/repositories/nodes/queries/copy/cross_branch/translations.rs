//! Translation carry-over (node-level and block-level) for cross-branch copy.

use super::super::super::super::NodeRepositoryImpl;
use super::CopyScope;
use crate::translation_write::OverlayTarget;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::translations::{LocaleCode, LocaleOverlay, TranslationMeta};
use raisin_models::tree::ChangeOperation;
use raisin_storage::NodeChangeInfo;
use rocksdb::WriteBatch;
use std::collections::HashMap;

/// Where the staged versions of one node's carry-over go.
struct Sink<'a> {
    batch: &'a mut WriteBatch,
    operation: ChangeOperation,
    change_infos: &'a mut Vec<NodeChangeInfo>,
    translation_ops: &'a mut Vec<raisin_replication::OpType>,
}

impl NodeRepositoryImpl {
    /// Make the target branch's overlays of `node_id` (same id) what the
    /// source branch holds, at the copy revision: every overlay live on the
    /// source is written, and every overlay live on the TARGET but not on the
    /// source — deleted there, or never there — is tombstoned. Without the
    /// tombstones a translation deleted on the draft branch stayed live on the
    /// published site forever, and no publish could remove it (the copy is
    /// replicated, so peers took the stale overlay as authoritative too).
    ///
    /// Both sides are read through the one reader: the source as of its pin or
    /// HEAD (`source_max_revision`), the target as of the copy revision.
    ///
    /// Returns **whether any overlay actually differed from what the target
    /// branch already held**, which is what the caller's no-op suppression
    /// turns on. Every live overlay is rewritten at the fresh revision on
    /// every run (that is what keeps the copy self-healing), so "wrote
    /// something" would be true for every translated node on every run and
    /// would defeat the suppression for the whole multilingual half of a site.
    /// The comparison is on the deserialized `LocaleOverlay` (key-order
    /// insensitive). A locale the target lacks, or one the copy tombstones,
    /// counts as differing.
    ///
    /// Also returns the node-level overlays it staged (`None`: tombstoned),
    /// for the localized name index sync the caller runs once the node and
    /// its overlays are both in the batch.
    pub(super) fn copy_translations_to_batch(
        &self,
        batch: &mut WriteBatch,
        node_id: &str,
        scope: &CopyScope<'_>,
        operation: ChangeOperation,
        change_infos: &mut Vec<NodeChangeInfo>,
        translation_ops: &mut Vec<raisin_replication::OpType>,
    ) -> Result<(bool, crate::localized_name::sync::Overrides)> {
        let mut staged = crate::localized_name::sync::Overrides::new();
        let mut sink = Sink {
            batch,
            operation,
            change_infos,
            translation_ops,
        };
        let source = Some(scope.source_max_revision);
        let target = Some(scope.revision);
        let mut differed = false;

        let mut target_nodes: HashMap<String, (LocaleCode, LocaleOverlay)> = self
            .collect_node_translations_for_copy(
                scope.tenant_id,
                scope.repo_id,
                scope.target_branch,
                scope.workspace,
                node_id,
                target,
            )?
            .into_iter()
            .map(|(locale, overlay, _)| (locale.as_str().to_string(), (locale, overlay)))
            .collect();
        for (locale, overlay, parent) in self.collect_node_translations_for_copy(
            scope.tenant_id,
            scope.repo_id,
            scope.source_branch,
            scope.workspace,
            node_id,
            source,
        )? {
            let held = target_nodes.remove(locale.as_str());
            differed |= held.as_ref().map(|(_, o)| o) != Some(&overlay);
            staged.insert(locale.as_str().to_string(), Some(overlay.clone()));
            self.stage_carried(
                &mut sink,
                scope,
                node_id,
                None,
                &locale,
                Some(&overlay),
                parent,
            )?;
        }
        for (_, (locale, _)) in target_nodes {
            differed = true;
            staged.insert(locale.as_str().to_string(), None);
            self.stage_carried(&mut sink, scope, node_id, None, &locale, None, None)?;
        }

        let mut target_blocks: HashMap<(String, String), (LocaleCode, LocaleOverlay)> = self
            .collect_block_translations_for_copy(
                scope.tenant_id,
                scope.repo_id,
                scope.target_branch,
                scope.workspace,
                node_id,
                target,
            )?
            .into_iter()
            .map(|(block, locale, overlay, _)| {
                ((block, locale.as_str().to_string()), (locale, overlay))
            })
            .collect();
        for (block, locale, overlay, parent) in self.collect_block_translations_for_copy(
            scope.tenant_id,
            scope.repo_id,
            scope.source_branch,
            scope.workspace,
            node_id,
            source,
        )? {
            let held = target_blocks.remove(&(block.clone(), locale.as_str().to_string()));
            differed |= held.as_ref().map(|(_, o)| o) != Some(&overlay);
            self.stage_carried(
                &mut sink,
                scope,
                node_id,
                Some(&block),
                &locale,
                Some(&overlay),
                parent,
            )?;
        }
        for ((block, _), (locale, _)) in target_blocks {
            differed = true;
            self.stage_carried(&mut sink, scope, node_id, Some(&block), &locale, None, None)?;
        }

        Ok((differed, staged))
    }

    /// Stage one carried version (`None`: a tombstone) on the target branch at
    /// the copy revision, with its replication op and change info.
    #[allow(clippy::too_many_arguments)]
    fn stage_carried(
        &self,
        sink: &mut Sink<'_>,
        scope: &CopyScope<'_>,
        node_id: &str,
        block_uuid: Option<&str>,
        locale: &LocaleCode,
        overlay: Option<&LocaleOverlay>,
        parent_revision: Option<HLC>,
    ) -> Result<()> {
        let target = OverlayTarget {
            tenant_id: scope.tenant_id,
            repo_id: scope.repo_id,
            branch: scope.target_branch,
            workspace: scope.workspace,
            node_id,
            block_uuid,
            locale: locale.as_str(),
        };
        let meta = TranslationMeta {
            locale: locale.clone(),
            revision: *scope.revision,
            parent_revision,
            timestamp: scope.now,
            actor: scope.meta_actor.to_string(),
            message: scope.meta_message.to_string(),
            is_system: scope.meta_is_system,
        };
        sink.translation_ops
            .push(self.stage_copied_translation(sink.batch, &target, overlay, &meta)?);
        sink.change_infos.push(NodeChangeInfo {
            node_id: node_id.to_string(),
            workspace: scope.workspace.to_string(),
            operation: match overlay {
                Some(_) => sink.operation,
                None => ChangeOperation::Deleted,
            },
            translation_locale: Some(target.locale_key()),
        });
        Ok(())
    }
}
