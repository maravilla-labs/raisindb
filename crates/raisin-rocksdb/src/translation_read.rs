//! The one reader of translation versions (`TRANSLATION_DATA` and
//! `BLOCK_TRANSLATIONS`).
//!
//! The repository (`repositories/translations`), the transaction
//! (`transaction/context/translations`) and merge conflict detection each used
//! to decode these CFs by hand, and they disagreed: the repository let a
//! newest tombstone hide a locale, the transaction skipped the tombstone and
//! fell through to an OLDER live version — so a deleted translation came back
//! on every SQL/WS read — and the repository read HEAD whatever revision it
//! was asked for. All of them call these functions.
//!
//! Key: `{tenant}\0{repo}\0{branch}\0{ws}\0translations\0{node_id}\0{locale}\0{~revision}`
//! (block overlays: `…\0block_trans\0{node_id}\0{block_uuid}\0{locale}\0{~revision}`),
//! built by `repositories::translations::keys`. The revision is 16 binary
//! bytes (usually not UTF-8, may contain `\0`), so only the text segments are
//! ever decoded.
//!
//! Rule, for one locale and for the listing alike: the NEWEST version at or
//! before the bound decides. A tombstone (`b"T"`) there means "absent" — never
//! "look further back". The bound is the caller's read revision (plan Phase
//! 11 item 1); `None` reads HEAD. Whether this node's history reaches back
//! to that bound is the caller's question — `translation_history`.
//!
//! # A node delete ends its overlays — decided HERE, at read time
//!
//! A live overlay version — node or block — written at `R` is absent at
//! bound `B` when its node has a delete tombstone at some `Rd` with
//! `R <= Rd <= B`, OR when the node's newest `NODES` record at or before `R`
//! is a delete tombstone: the version was written into a dead generation (a
//! peer that had not seen the delete, a merge replaying a branch's
//! translation onto a node the target deleted, a write racing the delete),
//! and a later recreate under the same id does not bring it back
//! ([`ended_by_node_delete`]; `mvcc_read::NodeLifeline`, asked once per node
//! by every listing: one seek on the `NODE_DELETES` index once the branch's
//! is `Ready`, otherwise one bounded `NODES` walk, nothing decoded —
//! `crate::node_delete_index` has the equivalence). Block
//! overlays follow the rule since plan Phase 11c; before it a deleted node's
//! block overlays stayed live forever, and a node recreated under the same
//! id got them back.
//!
//! This used to be DERIVED at write time: a node delete staged a `T` for
//! every locale it could see live, and a version arriving after a delete it
//! preceded staged that delete's `T`. Both halves were a read followed by a
//! separate write, so the result depended on timing: a translation committed
//! below a delete that had already staged its tombstones stayed live on the
//! origin while every replica tombstoned it; two peers' batches applied
//! concurrently on one replica each missed the other; a checkpoint ingest ran
//! no derivation at all. As a read rule there is nothing to race and nothing
//! to forget — every node that holds the same versions answers the same.
//! History GC is the one place the rule's evidence disappears (it drops node
//! tombstones); it writes the equivalent `T` first
//! (`translation_write::materialize_node_deletion`).
//!
//! For block overlays the same `T` is also MATERIALIZED as storage hygiene
//! (Phase 11c, `translation_write::materialize_block_deletion`): by the delete
//! funnel, by the one writer when a block version
//! lands below a stored delete, and by the `block_overlay_tombstones` repair
//! — so retention GC can reclaim the versions below it and a raw scan of the
//! CF sees the deletion. A materialization that loses a race changes no
//! answer: the rule decides. A dead-generation version is NOT materialized at
//! write time: a live record of the node landing later between the delete
//! and the version (out-of-order replication) would make it live again, and
//! a stored `T` could not be taken back. History GC materializes it when it
//! drops the delete that ends it (`translation_write::materialize_dead_generation`).

use crate::repositories::nodes::helpers::is_tombstone;
use crate::repositories::translations::keys;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::translations::LocaleOverlay;
use rocksdb::DB;

mod blocks;
mod chain;
mod projected;
#[cfg(test)]
mod projected_tests;
mod scan;
#[cfg(test)]
mod tests;

pub(crate) use blocks::{
    live_block_overlays, live_block_versions, read_block_version, stored_live_block_overlays,
    LiveBlockVersion,
};
pub(crate) use chain::{node_chain_in, NodeChain};
pub(crate) use projected::decode_overlay_keeping;
pub(crate) use scan::{for_each_newest, for_each_newest_in};

/// `{tenant}\0{repo}\0{branch}\0{ws}\0translations\0{node_id}\0`
pub(crate) fn node_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
) -> Vec<u8> {
    keys::translation_node_prefix(tenant_id, repo_id, branch, workspace, node_id)
}

/// `{node_prefix}{locale}\0` — every version of one locale.
pub(crate) fn locale_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    locale: &str,
) -> Vec<u8> {
    keys::translation_locale_prefix(tenant_id, repo_id, branch, workspace, node_id, locale)
}

/// The version of one locale that a read sees.
pub(crate) struct TranslationVersion {
    /// The exact key read — what a transaction records for conflict detection.
    pub key: Vec<u8>,
    /// The revision that version is stored at.
    pub revision: HLC,
    /// `None` when that version is a tombstone (the translation is deleted),
    /// or a node-overlay version a delete of its node has ended.
    pub overlay: Option<LocaleOverlay>,
}

/// Decode one stored version (`T` is a tombstone).
pub(crate) fn decode_overlay(value: &[u8]) -> Result<Option<LocaleOverlay>> {
    if is_tombstone(value) {
        return Ok(None);
    }
    serde_json::from_slice(value).map(Some).map_err(|e| {
        raisin_error::Error::storage(format!("Failed to deserialize LocaleOverlay: {}", e))
    })
}

/// The newest version under `prefix` (one locale's versions) in `cf_name`,
/// at or before `max_revision`, decoded by `decode`.
fn read_at_in(
    src: &mut impl crate::mvcc_read::VersionedRead,
    cf_name: &'static str,
    prefix: &[u8],
    max_revision: Option<&HLC>,
    decode: &dyn Fn(&[u8]) -> Result<Option<LocaleOverlay>>,
) -> Result<Option<TranslationVersion>> {
    let found = src.newest_at_or_before_with(
        cf_name,
        prefix,
        max_revision,
        |revision, value| -> Result<TranslationVersion> {
            let mut key = prefix.to_vec();
            key.extend_from_slice(&revision.encode_descending());
            Ok(TranslationVersion {
                key,
                revision,
                overlay: decode(value)?,
            })
        },
    )?;
    found.transpose()
}

/// [`read_at_in`] on the live database, fully decoded.
fn read_at(
    db: &DB,
    cf_name: &'static str,
    prefix: &[u8],
    max_revision: Option<&HLC>,
) -> Result<Option<TranslationVersion>> {
    read_at_in(
        &mut crate::mvcc_read::DbRead(db),
        cf_name,
        prefix,
        max_revision,
        &decode_overlay,
    )
}

/// The newest version of `locale` at or before `max_revision` (the newest at
/// all when `None`). `Ok(None)` when the locale never had one.
#[allow(clippy::too_many_arguments)]
pub(crate) fn read_version(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    locale: &str,
    max_revision: Option<&HLC>,
) -> Result<Option<TranslationVersion>> {
    read_version_in(
        &mut crate::mvcc_read::DbRead(db),
        (tenant_id, repo_id, branch, workspace),
        node_id,
        locale,
        max_revision,
        &decode_overlay,
    )
}

/// [`read_version`] through a read source, decoding with `decode` (a full
/// overlay, or [`decode_overlay_keeping`] one field).
pub(crate) fn read_version_in(
    src: &mut impl crate::mvcc_read::VersionedRead,
    scope: (&str, &str, &str, &str),
    node_id: &str,
    locale: &str,
    max_revision: Option<&HLC>,
    decode: &dyn Fn(&[u8]) -> Result<Option<LocaleOverlay>>,
) -> Result<Option<TranslationVersion>> {
    let (tenant_id, repo_id, branch, workspace) = scope;
    let prefix = locale_prefix(tenant_id, repo_id, branch, workspace, node_id, locale);
    let Some(mut version) = read_at_in(
        src,
        crate::cf::TRANSLATION_DATA,
        &prefix,
        max_revision,
        decode,
    )?
    else {
        return Ok(None);
    };
    if version.overlay.is_some() {
        let lifeline = crate::mvcc_read::NodeLifeline::read_in(
            src,
            scope,
            node_id,
            &version.revision,
            max_revision,
        )?;
        if lifeline.ends_in(src, &version.revision)? {
            version.overlay = None;
        }
    }
    Ok(Some(version))
}

/// Whether a delete of `node_id` ends an overlay version (node or block)
/// stored at `version_revision`, as read at `bound` (`None` = HEAD): a delete
/// tombstone at a revision in `[version_revision, bound]`, or a version
/// written into a dead generation (`mvcc_read::NodeLifeline`). The module doc
/// has why this is a read rule. A reader deciding several versions of one
/// node builds the lifeline once instead.
pub(crate) fn ended_by_node_delete(
    db: &DB,
    scope: (&str, &str, &str, &str),
    node_id: &str,
    version_revision: &HLC,
    bound: Option<&HLC>,
) -> Result<bool> {
    crate::mvcc_read::NodeLifeline::read(db, scope, node_id, version_revision, bound)?
        .ends(db, version_revision)
}

/// `NodeLifeline::ends` for every candidate of one node at once: one walk
/// from the oldest candidate's revision.
pub(crate) fn retain_not_ended<T>(
    db: &DB,
    scope: (&str, &str, &str, &str),
    node_id: &str,
    bound: Option<&HLC>,
    candidates: &mut Vec<T>,
    revision_of: impl Fn(&T) -> HLC,
) -> Result<()> {
    let Some(oldest) = candidates.iter().map(&revision_of).min() else {
        return Ok(());
    };
    let lifeline = crate::mvcc_read::NodeLifeline::read(db, scope, node_id, &oldest, bound)?;
    let mut ended = Vec::with_capacity(candidates.len());
    for candidate in candidates.iter() {
        ended.push(lifeline.ends(db, &revision_of(candidate))?);
    }
    let mut ended = ended.into_iter();
    candidates.retain(|_| !ended.next().unwrap_or(false));
    Ok(())
}

/// The newest live overlay of `locale` at or before `max_revision`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn read_overlay(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    locale: &str,
    max_revision: Option<&HLC>,
) -> Result<Option<LocaleOverlay>> {
    Ok(read_version(
        db,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        locale,
        max_revision,
    )?
    .and_then(|version| version.overlay))
}

/// Every locale of `node_id` whose newest version at or before
/// `max_revision` is live, in key order (each listed once).
pub(crate) fn live_locales(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    max_revision: Option<&HLC>,
) -> Result<Vec<String>> {
    let mut locales = stored_live_locales(
        db,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        max_revision,
    )?;
    retain_not_ended(
        db,
        (tenant_id, repo_id, branch, workspace),
        node_id,
        max_revision,
        &mut locales,
        |(_, revision)| *revision,
    )?;
    Ok(locales.into_iter().map(|(locale, _)| locale).collect())
}

/// Every locale of `node_id` whose newest STORED version at or before
/// `max_revision` is not a `T`, with that version's revision — the stored
/// half of [`live_locales`], before a node delete is taken into account.
/// History GC needs exactly this, at the delete it is about to drop.
pub(crate) fn stored_live_locales(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    max_revision: Option<&HLC>,
) -> Result<Vec<(String, HLC)>> {
    let prefix = node_prefix(tenant_id, repo_id, branch, workspace, node_id);
    let mut locales = Vec::new();
    for_each_newest(
        db,
        crate::cf::TRANSLATION_DATA,
        &prefix,
        1,
        max_revision,
        |segments, revision, value| {
            if !is_tombstone(value) {
                locales.push((segments[0].to_string(), revision));
            }
        },
    )?;
    Ok(locales)
}
