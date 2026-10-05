//! One node's overlays across a whole fallback chain — node-level and block
//! level — through ONE read source and ONE node-delete walk.
//!
//! The per-locale readers ([`super::read_version_in`],
//! [`super::live_block_versions`]) each open their own iterators and walk the
//! node's `NODES` history once per live version they decide. A localized scan
//! row asked that for every chain locale plus the block inventory, so a tree
//! read in `fr` (chain `fr`, `de`) opened ~5 iterators per row and walked the
//! node's revisions above each overlay once per locale. Reading a page of rows
//! through one [`crate::mvcc_read::SnapshotRead`] makes it one iterator per
//! column family for the page, and the lifeline is built once per node from
//! its oldest live candidate — `NodeLifeline::ends` answers every newer
//! version from that one walk (the same argument as `retain_not_ended`).
//!
//! The answer is the per-locale readers' answer: the same newest-at-or-before
//! seek, the same decoder, the same `ends` rule.

use super::{decode_overlay, for_each_newest_in, locale_prefix, LiveBlockVersion};
use crate::mvcc_read::{NodeLifeline, VersionedRead};
use crate::repositories::nodes::helpers::is_tombstone;
use crate::repositories::translations::keys;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::translations::LocaleOverlay;

/// A node's overlays in a chain, as read at a bound.
pub(crate) struct NodeChain {
    /// The live node-level overlay of each chain locale, aligned with it.
    pub node: Vec<Option<LocaleOverlay>>,
    /// Every live block overlay in a chain locale, in key order.
    pub blocks: Vec<LiveBlockVersion>,
}

/// `node_id`'s overlays in `chain` (and its block overlays in `chain` when
/// `with_blocks`) at `bound`, through `src`.
pub(crate) fn node_chain_in(
    src: &mut impl VersionedRead,
    scope: (&str, &str, &str, &str),
    node_id: &str,
    chain: &[&str],
    bound: Option<&HLC>,
    with_blocks: bool,
) -> Result<NodeChain> {
    let (tenant_id, repo_id, branch, workspace) = scope;
    // The newest stored version of each chain locale, decoded; its revision
    // is kept only while the version is live (the lifeline's question).
    let mut node: Vec<(Option<HLC>, Option<LocaleOverlay>)> = Vec::with_capacity(chain.len());
    for locale in chain {
        let prefix = locale_prefix(tenant_id, repo_id, branch, workspace, node_id, locale);
        let found = src.newest_at_or_before_with(
            crate::cf::TRANSLATION_DATA,
            &prefix,
            bound,
            |revision, value| decode_overlay(value).map(|overlay| (revision, overlay)),
        )?;
        node.push(match found.transpose()? {
            Some((revision, Some(overlay))) => (Some(revision), Some(overlay)),
            _ => (None, None),
        });
    }

    let mut stored_blocks = Vec::new();
    if with_blocks {
        let prefix =
            keys::block_translations_node_prefix(tenant_id, repo_id, branch, workspace, node_id);
        for_each_newest_in(
            src,
            crate::cf::BLOCK_TRANSLATIONS,
            &prefix,
            2,
            bound,
            |segments, revision, value| {
                if segments[1] != keys::BLOCK_ORPHAN_MARKER
                    && !is_tombstone(value)
                    && chain.contains(&segments[1])
                {
                    stored_blocks.push((
                        segments[0].to_string(),
                        segments[1].to_string(),
                        revision,
                        value.to_vec(),
                    ));
                }
            },
        )?;
    }

    // ONE walk of the node's history, from its oldest live candidate.
    let oldest = node
        .iter()
        .filter_map(|(revision, _)| *revision)
        .chain(stored_blocks.iter().map(|b| b.2))
        .min();
    if let Some(oldest) = oldest {
        let lifeline = NodeLifeline::read_in(src, scope, node_id, &oldest, bound)?;
        for entry in node.iter_mut() {
            if entry.0.is_some_and(|revision| lifeline.ends(&revision)) {
                entry.1 = None;
            }
        }
        stored_blocks.retain(|b| !lifeline.ends(&b.2));
    }

    let mut blocks = Vec::with_capacity(stored_blocks.len());
    for (block_uuid, locale, revision, value) in stored_blocks {
        if let Some(overlay) = decode_overlay(&value)? {
            blocks.push(LiveBlockVersion {
                block_uuid,
                locale,
                revision,
                overlay,
            });
        }
    }
    Ok(NodeChain {
        node: node.into_iter().map(|(_, overlay)| overlay).collect(),
        blocks,
    })
}
