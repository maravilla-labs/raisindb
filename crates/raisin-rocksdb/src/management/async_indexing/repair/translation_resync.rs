//! `resync_translations`: converge replicas that never received translations
//! (plan Phase 11 item 5).
//!
//! Before Phase 11 translations were captured but never applied, so every
//! replica's overlays are missing or stale. This re-emits EVERY stored version
//! of this node — each `TRANSLATION_DATA` and `BLOCK_TRANSLATIONS` version,
//! tombstones included — as a replication op at its ORIGINAL revision.
//! Re-emitting only the current overlay would falsify replica history: today's
//! translation stamped at an old revision, or a change that never happened at
//! a new one.
//!
//! - Every op of a branch carries this node's history floor FOR THAT BRANCH
//!   (`translation_history::sender_floor`) when it has one; a replica records
//!   it, and locale-scoped reads of that branch below it fail loudly there.
//! - Idempotent on the replica: a version lands on its origin's key, so a
//!   second run (or the fan-out making every node resync) rewrites the same
//!   bytes.
//! - Streaming and resumable like every repair: the per-node state record
//!   holds the last emitted key, committed after each chunk; a dry run counts.
//!   It writes the oplog, not data, and is admin-triggered only.

use super::branches::list_branches;
use super::cursor::{load_state, state_key, BoundedWriter, RepairState};
use super::translation_resync_scan::read_chunk;
pub use super::translation_resync_scan::TranslationResyncCounts;
use super::{check_headroom_assuming, repair_node_id, RepairKind, RepairOptions, RepairReport};
use crate::repositories::translations::key_parse::parse_version_key;
use crate::repositories::translations::replication::{translation_op, TranslationVersionOp};
use crate::translation_write::OverlayTarget;
use crate::{cf, keys};
use raisin_error::{Error, Result};
use raisin_hlc::HLC;

const PASS_DATA: &str = "translations";
const PASS_BLOCKS: &str = "blocks";

/// Resync one branch, or every branch of the repository.
pub(super) async fn resync_translations(
    storage: &crate::RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: Option<&str>,
    options: &RepairOptions,
) -> Result<Vec<RepairReport>> {
    let db = storage.db();
    let capture = storage.operation_capture();
    if !options.dry_run && !capture.is_enabled() {
        return Err(Error::Validation(
            "resync_translations re-emits translations for replication, which is not enabled \
             on this node"
                .to_string(),
        ));
    }
    if options.check_headroom && !options.dry_run {
        check_headroom_assuming(db, cf::TRANSLATION_DATA, options.free_bytes_override)?;
    }
    let branches = match branch {
        Some(branch) => vec![branch.to_string()],
        None => list_branches(db, tenant_id, repo_id)?,
    };
    let node_id = repair_node_id(storage);
    let mut reports = Vec::new();
    for branch in &branches {
        reports.push(resync_branch(storage, tenant_id, repo_id, branch, &node_id, options).await?);
    }
    Ok(reports)
}

async fn resync_branch(
    storage: &crate::RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node_id: &str,
    options: &RepairOptions,
) -> Result<RepairReport> {
    let db = storage.db();
    // Per branch: a branch whose history GC never touched ships no floor.
    let floor = crate::translation_history::sender_floor(db, tenant_id, repo_id, branch)?;
    let slug = RepairKind::ResyncTranslations.slug();
    let previous = load_state(db, tenant_id, repo_id, branch, slug, node_id)?;
    let resumed = !options.dry_run
        && previous
            .as_ref()
            .is_some_and(|s| s.status == "running" && s.cursor.is_some());
    let state = if resumed {
        previous.unwrap_or_default()
    } else {
        RepairState::default()
    };
    let mut writer = BoundedWriter::new(
        db,
        state_key(tenant_id, repo_id, branch, slug, node_id),
        state,
        options.batch_bytes,
        options.dry_run,
        options.stop_after_batches,
        options.max_bytes_per_sec,
        options.before_commit.clone(),
    );
    let mut report = RepairReport {
        branch: branch.to_string(),
        repair: slug.to_string(),
        dry_run: options.dry_run,
        resumed,
        ..RepairReport::default()
    };
    report.translations.history_complete_from = floor.map(|f| f.to_string());

    let mut run = Run {
        storage,
        tenant_id,
        repo_id,
        branch,
        floor,
        chunks: 0,
    };
    let skip_data = writer.state().pass == PASS_BLOCKS;
    let completed = (skip_data
        || run
            .pass(
                &mut writer,
                PASS_DATA,
                cf::TRANSLATION_DATA,
                options,
                &mut report,
            )
            .await?)
        && run
            .pass(
                &mut writer,
                PASS_BLOCKS,
                cf::BLOCK_TRANSLATIONS,
                options,
                &mut report,
            )
            .await?;
    if completed {
        writer.clear_cursor();
        writer.commit("done")?;
    }
    report.completed = completed;
    report.writes = writer.report.clone();
    tracing::info!(
        tenant_id,
        repo_id,
        branch,
        dry_run = options.dry_run,
        completed,
        versions = report.translations.versions,
        emitted = report.translations.emitted,
        "translation resync finished a branch"
    );
    Ok(report)
}

struct Run<'a> {
    storage: &'a crate::RocksDBStorage,
    tenant_id: &'a str,
    repo_id: &'a str,
    branch: &'a str,
    floor: Option<HLC>,
    /// Chunks committed (the crash hook counts these).
    chunks: usize,
}

impl Run<'_> {
    /// Emit every version of one CF of the branch. `false`: stopped early
    /// (the crash hook), resumable from the cursor.
    async fn pass(
        &mut self,
        writer: &mut BoundedWriter<'_>,
        pass: &str,
        cf_name: &str,
        options: &RepairOptions,
        report: &mut RepairReport,
    ) -> Result<bool> {
        writer.begin_pass(pass);
        let prefix = keys::branch_prefix(self.tenant_id, self.repo_id, self.branch);
        let mut after = writer
            .state()
            .cursor
            .as_deref()
            .and_then(|h| hex::decode(h).ok());
        loop {
            let chunk = read_chunk(
                self.storage.db(),
                cf_name,
                &prefix,
                after.as_deref(),
                options.batch_bytes,
            )?;
            let Some((last_key, _)) = chunk.last() else {
                return Ok(true);
            };
            let last_key = last_key.clone();
            let mut bytes = 0u64;
            for (key, value) in &chunk {
                bytes += value.len() as u64;
                self.emit(key, value, &prefix, options.dry_run, report)
                    .await?;
            }
            writer.note_read(bytes);
            writer.mark(pass, &last_key);
            writer.commit("running")?;
            self.chunks += 1;
            if options
                .stop_after_batches
                .is_some_and(|limit| self.chunks >= limit)
            {
                return Ok(false);
            }
            after = Some(last_key);
        }
    }

    async fn emit(
        &self,
        key: &[u8],
        value: &[u8],
        prefix: &[u8],
        dry_run: bool,
        report: &mut RepairReport,
    ) -> Result<()> {
        let Some(parsed) = parse_version_key(prefix, key) else {
            return Ok(());
        };
        let overlay = match crate::translation_read::decode_overlay(value) {
            Ok(overlay) => overlay,
            Err(e) => {
                tracing::warn!(error = %e, node_id = %parsed.node_id, "skipping an undecodable translation version");
                return Ok(());
            }
        };
        let counts = &mut report.translations;
        counts.versions += 1;
        match &overlay {
            None => counts.tombstones += 1,
            Some(raisin_models::translations::LocaleOverlay::Hidden) => counts.hidden += 1,
            Some(_) => counts.live += 1,
        }
        if parsed.block_uuid.is_some() {
            counts.blocks += 1;
        }
        if dry_run {
            return Ok(());
        }
        let version = TranslationVersionOp {
            target: OverlayTarget {
                tenant_id: self.tenant_id,
                repo_id: self.repo_id,
                branch: self.branch,
                workspace: &parsed.workspace,
                node_id: &parsed.node_id,
                block_uuid: parsed.block_uuid.as_deref(),
                locale: &parsed.locale,
            },
            overlay: overlay.as_ref(),
            revision: parsed.revision,
            history_complete_from: self.floor,
        };
        self.storage
            .operation_capture()
            .capture_operation_with_revision(
                self.tenant_id.to_string(),
                self.repo_id.to_string(),
                self.branch.to_string(),
                translation_op(&version),
                "system:resync_translations".to_string(),
                Some("resync translations".to_string()),
                true,
                Some(parsed.revision),
            )
            .await?;
        counts.emitted += 1;
        Ok(())
    }
}
