//! Block overlays: the same newest-at-or-before rule over `BLOCK_TRANSLATIONS`,
//! and the same node-delete read rule as node overlays (plan Phase 11c).
//!
//! Both block readers used to take the FIRST key of the prefix — HEAD,
//! whatever revision was asked for — and the transaction's skipped a
//! tombstone to fall through to an older live version. Until Phase 11c a
//! node delete did not end its block overlays at all: they stayed live
//! forever, and a node recreated under the same id got them back.
//!
//! A reader that needs more than one block of a node — the resolver, the copy
//! collectors, merge resolutions — reads them all through
//! [`live_block_versions`]: one prefix scan and ONE `NODES` walk for the
//! node, where a [`read_block_version`] per block repeats that walk per
//! `(block, locale)`.

use super::{decode_overlay, ended_by_node_delete, for_each_newest, read_at, TranslationVersion};
use crate::repositories::nodes::helpers::is_tombstone;
use crate::repositories::translations::keys;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::translations::LocaleOverlay;
use rocksdb::DB;

/// The newest version of one block's `locale` at or before `max_revision`.
/// A live version a delete of its node has ended reads as absent
/// (`overlay: None`), exactly like a node overlay.
#[allow(clippy::too_many_arguments)]
pub(crate) fn read_block_version(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    block_uuid: &str,
    locale: &str,
    max_revision: Option<&HLC>,
) -> Result<Option<TranslationVersion>> {
    let prefix = keys::block_translation_prefix(
        tenant_id, repo_id, branch, workspace, node_id, block_uuid, locale,
    );
    let Some(mut version) = read_at(db, crate::cf::BLOCK_TRANSLATIONS, &prefix, max_revision)?
    else {
        return Ok(None);
    };
    if version.overlay.is_some()
        && ended_by_node_delete(
            db,
            (tenant_id, repo_id, branch, workspace),
            node_id,
            &version.revision,
            max_revision,
        )?
    {
        version.overlay = None;
    }
    Ok(Some(version))
}

/// One live block overlay of a node, as [`live_block_versions`] reads it.
pub(crate) struct LiveBlockVersion {
    pub block_uuid: String,
    pub locale: String,
    /// The revision the version is stored at.
    pub revision: HLC,
    pub overlay: LocaleOverlay,
}

/// Every live block overlay of `node_id` at or before `max_revision` whose
/// locale `wanted` accepts, decoded: the newest version of each
/// `(block, locale)` that is not a `T` and not ended by a delete of the node.
/// One scan of the node's `BLOCK_TRANSLATIONS` and one `NODES` walk for all
/// of them; only the wanted locales' values are decoded.
pub(crate) fn live_block_versions(
    db: &DB,
    scope: (&str, &str, &str, &str),
    node_id: &str,
    max_revision: Option<&HLC>,
    wanted: impl Fn(&str) -> bool,
) -> Result<Vec<LiveBlockVersion>> {
    let mut stored = Vec::new();
    scan_newest(
        db,
        scope,
        node_id,
        max_revision,
        |block, locale, revision, value| {
            if wanted(locale) {
                stored.push((
                    block.to_string(),
                    locale.to_string(),
                    revision,
                    value.to_vec(),
                ));
            }
        },
    )?;
    super::retain_not_ended(db, scope, node_id, max_revision, &mut stored, |s| s.2)?;
    let mut out = Vec::with_capacity(stored.len());
    for (block_uuid, locale, revision, value) in stored {
        if let Some(overlay) = decode_overlay(&value)? {
            out.push(LiveBlockVersion {
                block_uuid,
                locale,
                revision,
                overlay,
            });
        }
    }
    Ok(out)
}

/// Every `(block_uuid, locale)` of `node_id` whose newest version at or
/// before `max_revision` is live and not ended by a delete of the node.
/// Orphan markers sit in the locale position and are skipped.
pub(crate) fn live_block_overlays(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    max_revision: Option<&HLC>,
) -> Result<Vec<(String, String)>> {
    let scope = (tenant_id, repo_id, branch, workspace);
    let mut stored = stored_live_block_overlays(db, scope, node_id, max_revision)?;
    super::retain_not_ended(db, scope, node_id, max_revision, &mut stored, |s| s.2)?;
    Ok(stored
        .into_iter()
        .map(|(block, locale, _)| (block, locale))
        .collect())
}

/// Every `(block_uuid, locale)` of `node_id` whose newest STORED version at
/// or before `max_revision` is not a `T`, with that version's revision — the
/// stored half of [`live_block_overlays`], before a node delete is taken into
/// account. The block-overlay materializations (`translation_write`) need
/// exactly this, at the delete they stand for.
pub(crate) fn stored_live_block_overlays(
    db: &DB,
    scope: (&str, &str, &str, &str),
    node_id: &str,
    max_revision: Option<&HLC>,
) -> Result<Vec<(String, String, HLC)>> {
    let mut found = Vec::new();
    scan_newest(
        db,
        scope,
        node_id,
        max_revision,
        |block, locale, revision, _| {
            found.push((block.to_string(), locale.to_string(), revision));
        },
    )?;
    Ok(found)
}

/// The newest stored version of each `(block, locale)` of `node_id` at or
/// before `max_revision` that is not a `T` (orphan markers skipped).
fn scan_newest(
    db: &DB,
    scope: (&str, &str, &str, &str),
    node_id: &str,
    max_revision: Option<&HLC>,
    mut visit: impl FnMut(&str, &str, HLC, &[u8]),
) -> Result<()> {
    let (tenant_id, repo_id, branch, workspace) = scope;
    let prefix =
        keys::block_translations_node_prefix(tenant_id, repo_id, branch, workspace, node_id);
    for_each_newest(
        db,
        crate::cf::BLOCK_TRANSLATIONS,
        &prefix,
        2,
        max_revision,
        |segments, revision, value| {
            if segments[1] != keys::BLOCK_ORPHAN_MARKER && !is_tombstone(value) {
                visit(segments[0], segments[1], revision, value);
            }
        },
    )
}
