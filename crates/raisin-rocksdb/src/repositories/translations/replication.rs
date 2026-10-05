//! Replicating translation versions (plan Phase 11 item 4) — the ONE builder
//! of translation ops, used by the repository, the transaction commit, copies,
//! merge resolutions and the resync job.
//!
//! Every version travels as [`OpType::UpsertTranslationOverlay`] at its
//! original revision: Hidden as Hidden, a deletion as `Deleted`. There is no
//! legacy shape and no emission flag — no cluster ran on the pre-Phase-11
//! `set_translation` / `delete_translation` ops (which no binary ever
//! applied), so they were removed; one still in a saved oplog decodes as
//! `OpType::Unknown` and is skipped.

use crate::translation_write::OverlayTarget;
use raisin_hlc::HLC;
use raisin_models::translations::LocaleOverlay;
use raisin_replication::{OpType, ReplicatedOverlay};

/// One stored version, as replication sees it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TranslationVersionOp<'a> {
    pub target: OverlayTarget<'a>,
    /// `None` is a tombstone.
    pub overlay: Option<&'a LocaleOverlay>,
    pub revision: HLC,
    /// The sender's floor (resync only).
    pub history_complete_from: Option<HLC>,
}

/// The op for one version.
pub(crate) fn translation_op(version: &TranslationVersionOp<'_>) -> OpType {
    let target = &version.target;
    OpType::UpsertTranslationOverlay {
        workspace: target.workspace.to_string(),
        node_id: target.node_id.to_string(),
        locale: target.locale.to_string(),
        block_uuid: target.block_uuid.map(str::to_string),
        overlay: ReplicatedOverlay::from_stored(version.overlay),
        revision: version.revision,
        history_complete_from: version.history_complete_from,
    }
}

/// A received translation op, decoded: what to store, where, and the
/// sender's floor.
pub(crate) struct ReceivedVersion {
    pub workspace: String,
    pub node_id: String,
    pub block_uuid: Option<String>,
    pub locale: String,
    pub overlay: Option<LocaleOverlay>,
    pub revision: HLC,
    pub history_complete_from: Option<HLC>,
}

/// Decode a translation op; `None` for anything that is not one.
pub(crate) fn received_version(op_type: &OpType) -> Option<ReceivedVersion> {
    match op_type {
        OpType::UpsertTranslationOverlay {
            workspace,
            node_id,
            locale,
            block_uuid,
            overlay,
            revision,
            history_complete_from,
        } => Some(ReceivedVersion {
            workspace: workspace.clone(),
            node_id: node_id.clone(),
            block_uuid: block_uuid.clone(),
            locale: locale.clone(),
            overlay: overlay.clone().into_stored(),
            revision: *revision,
            history_complete_from: *history_complete_from,
        }),
        _ => None,
    }
}

/// Capture one version for replication (no-op when capture is disabled).
/// Failures are logged: the version is durable locally either way, and the
/// resync job re-emits it.
pub(crate) async fn capture_version(
    capture: Option<&std::sync::Arc<crate::OperationCapture>>,
    version: &TranslationVersionOp<'_>,
    actor: &str,
) {
    let Some(capture) = capture else {
        return;
    };
    if !capture.is_enabled() {
        return;
    }
    let target = &version.target;
    if let Err(e) = capture
        .capture_operation_with_revision(
            target.tenant_id.to_string(),
            target.repo_id.to_string(),
            target.branch.to_string(),
            translation_op(version),
            actor.to_string(),
            None,
            false,
            Some(version.revision),
        )
        .await
    {
        tracing::warn!(
            error = %e,
            node_id = target.node_id,
            locale = target.locale,
            "failed to capture a translation version for replication"
        );
    }
}
