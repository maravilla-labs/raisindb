//! Applying a replicated translation version (plan Phase 11).
//!
//! `UpsertTranslationOverlay` decodes to one stored version
//! (`repositories::translations::replication`), written by the one translation
//! writer at the ORIGIN's revision: the version, its index entry, its meta and
//! snapshot. Same key, same bytes, so a redelivery is idempotent and an
//! out-of-order version lands beneath a newer one exactly where the origin
//! has it. A delete of the node at or above the version's revision ends it
//! by the read rule (`translation_read::ended_by_node_delete`), whichever
//! arrived first — and whether or not the two were applied concurrently. For
//! a live BLOCK version below a stored delete the one writer also stores that
//! delete's `T` (plan Phase 11c, `translation_write::block_deletion`), the
//! hygiene half of the same rule; nothing else is derived here. A resync op carries
//! the sender's history floor for the op's branch, recorded here as this
//! node's `translation_history_complete_from` for that branch.

use raisin_error::Result;
use raisin_models::translations::{LocaleCode, TranslationMeta};
use raisin_replication::Operation;
use rocksdb::WriteBatch;

use super::applicator::OperationApplicator;
use crate::repositories::translations::replication::received_version;
use crate::translation_write::{self, OverlayTarget};

pub(super) async fn apply_translation_version(
    applicator: &OperationApplicator,
    op: &Operation,
) -> Result<()> {
    let Some(received) = received_version(&op.op_type) else {
        return Ok(());
    };

    let db = applicator.db();
    let target = OverlayTarget {
        tenant_id: &op.tenant_id,
        repo_id: &op.repo_id,
        branch: &op.branch,
        workspace: &received.workspace,
        node_id: &received.node_id,
        block_uuid: received.block_uuid.as_deref(),
        locale: &received.locale,
    };
    let overlay = received.overlay.as_ref();

    let mut batch = WriteBatch::default();
    translation_write::stage_version(db, &mut batch, &target, overlay, &received.revision)?;
    match LocaleCode::parse(&received.locale) {
        Ok(locale) => {
            let timestamp = chrono::DateTime::from_timestamp_millis(op.timestamp_ms as i64)
                .unwrap_or_else(chrono::Utc::now);
            let meta = TranslationMeta {
                locale,
                revision: received.revision,
                parent_revision: None,
                timestamp,
                actor: op.actor.clone(),
                message: op
                    .message
                    .clone()
                    .unwrap_or_else(|| "replicated translation".to_string()),
                is_system: op.is_system,
            };
            translation_write::stage_history(db, &mut batch, &target, overlay, &meta)?;
        }
        Err(e) => tracing::warn!(
            locale = %received.locale,
            error = %e,
            "replicated translation has an unparseable locale; stored without meta"
        ),
    }
    db.write(batch)
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

    if let Some(floor) = received.history_complete_from {
        // The sender's floor for THIS branch only; ignored when it is this
        // node's own GC floor coming back (`accept_received_floor`).
        crate::translation_history::accept_received_floor(
            db,
            &op.tenant_id,
            &op.repo_id,
            &op.branch,
            floor,
        )?;
    }
    Ok(())
}
