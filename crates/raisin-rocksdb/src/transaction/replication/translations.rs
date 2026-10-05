//! Replicating the translations a transaction wrote (plan Phase 11), and
//! their history records.
//!
//! SQL/WS translation writes (`UPDATE … TRANSLATE`, package installs, COPY)
//! go through the transaction, which never captured them: a replica got the
//! node and never its overlays. The read cache holds exactly this
//! transaction's translation writes (reads never populate it), so the commit
//! replays it through the one op builder
//! (`repositories::translations::replication`).
//!
//! The same versions get their `TranslationMeta` (and snapshot) in the commit
//! batch, through `translation_write::stage_history` — the records every
//! other writer stores, and the ones a replica's apply arm stores from the
//! captured op (same revision, actor and message). Without them
//! `get_translation_meta` answered the origin's previous writer while every
//! replica answered this one.

use super::super::RocksDBTransaction;
use crate::repositories::translations::replication::{translation_op, TranslationVersionOp};
use crate::translation_write::OverlayTarget;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::translations::{LocaleCode, LocaleOverlay, TranslationMeta};
use rocksdb::WriteBatch;

/// One translation version this transaction wrote.
struct StagedVersion {
    workspace: String,
    node_id: String,
    block_uuid: Option<String>,
    locale: String,
    overlay: Option<LocaleOverlay>,
    revision: HLC,
}

/// Which branch, and who wrote it, for every version of one commit.
pub(in super::super) struct TranslationCommit<'a> {
    pub tenant_id: &'a str,
    pub repo_id: &'a str,
    pub branch: &'a str,
    pub actor: &'a str,
    pub message: &'a str,
    pub is_system: bool,
}

impl StagedVersion {
    fn target<'a>(&'a self, commit: &TranslationCommit<'a>) -> OverlayTarget<'a> {
        OverlayTarget {
            tenant_id: commit.tenant_id,
            repo_id: commit.repo_id,
            branch: commit.branch,
            workspace: &self.workspace,
            node_id: &self.node_id,
            block_uuid: self.block_uuid.as_deref(),
            locale: &self.locale,
        }
    }
}

impl RocksDBTransaction {
    /// Every translation version this transaction wrote, at the revision it
    /// was written at.
    fn staged_translation_versions(&self) -> Result<Vec<StagedVersion>> {
        let lock_err = |e: String| raisin_error::Error::storage(format!("Lock error: {}", e));
        let tx_revision = self
            .metadata
            .lock()
            .map_err(|e| lock_err(e.to_string()))?
            .transaction_revision;
        let changed: std::collections::HashMap<(String, String), HLC> = self
            .changed_translations
            .lock()
            .map_err(|e| lock_err(e.to_string()))?
            .iter()
            .map(|((node, locale), change)| ((node.clone(), locale.clone()), change.revision))
            .collect();
        let cache = self
            .read_cache
            .lock()
            .map_err(|e| lock_err(e.to_string()))?;

        let nodes = cache
            .translations
            .iter()
            .map(|((ws, node, locale), overlay)| (ws, node, None, locale, overlay));
        let blocks = cache
            .block_translations
            .iter()
            .map(|((ws, node, block, locale), overlay)| (ws, node, Some(block), locale, overlay));
        let mut staged = Vec::new();
        for (workspace, node_id, block_uuid, locale, overlay) in nodes.chain(blocks) {
            let revision = match block_uuid {
                None => changed
                    .get(&(node_id.clone(), locale.clone()))
                    .copied()
                    .or(tx_revision),
                Some(_) => tx_revision,
            };
            let Some(revision) = revision else {
                tracing::warn!(
                    node_id = %node_id,
                    locale = %locale,
                    "translation write without a revision; not replicated"
                );
                continue;
            };
            staged.push(StagedVersion {
                workspace: workspace.clone(),
                node_id: node_id.clone(),
                block_uuid: block_uuid.cloned(),
                locale: locale.clone(),
                overlay: overlay.clone(),
                revision,
            });
        }
        Ok(staged)
    }

    /// Stage the `TranslationMeta` and snapshot of every version this
    /// transaction wrote into the commit batch — what the replica's apply arm
    /// stores for the op [`Self::capture_translation_changes`] sends.
    pub(in super::super) fn stage_translation_history(
        &self,
        batch: &mut WriteBatch,
        commit: &TranslationCommit<'_>,
    ) -> Result<()> {
        let timestamp = chrono::Utc::now();
        for version in self.staged_translation_versions()? {
            let locale = match LocaleCode::parse(&version.locale) {
                Ok(locale) => locale,
                Err(e) => {
                    // The apply arm stores no meta for it either.
                    tracing::warn!(
                        locale = %version.locale,
                        error = %e,
                        "translation with an unparseable locale; stored without meta"
                    );
                    continue;
                }
            };
            let meta = TranslationMeta {
                locale,
                revision: version.revision,
                parent_revision: None,
                timestamp,
                actor: commit.actor.to_string(),
                message: commit.message.to_string(),
                is_system: commit.is_system,
            };
            crate::translation_write::stage_history(
                &self.db,
                batch,
                &version.target(commit),
                version.overlay.as_ref(),
                &meta,
            )?;
        }
        Ok(())
    }

    /// Capture one op per translation version this transaction wrote, at the
    /// revision it was written at. Called after the commit is durable.
    pub(in super::super) async fn capture_translation_changes(
        &self,
        commit: &TranslationCommit<'_>,
    ) -> Result<()> {
        if !self.operation_capture.is_enabled() {
            return Ok(());
        }
        for version in self.staged_translation_versions()? {
            let op = TranslationVersionOp {
                target: version.target(commit),
                overlay: version.overlay.as_ref(),
                revision: version.revision,
                history_complete_from: None,
            };
            self.capture_operation_internal(
                commit.tenant_id.to_string(),
                commit.repo_id.to_string(),
                commit.branch.to_string(),
                translation_op(&op),
                commit.actor.to_string(),
                Some(commit.message.to_string()),
                commit.is_system,
                Some(version.revision),
            )
            .await;
        }
        Ok(())
    }
}
