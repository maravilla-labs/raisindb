//! Overlay versions written into a dead generation, materialized when the
//! delete that ends them is about to disappear.
//!
//! The read rule (`translation_read`, `mvcc_read::NodeLifeline`) ends a
//! version stored at `R` when the node's newest `NODES` record at or before
//! `R` is a delete tombstone `D`: it was written while the node was deleted
//! (a peer that had not seen the delete, a merge replaying a branch's
//! translation onto a node the target deleted). The evidence is `D` itself,
//! so history GC must not drop `D` without first storing what it meant:
//! every live version — node or block — stored in `(D, next record)` becomes
//! a `T` at its OWN revision. Such a version is never visible at any bound
//! (below `R` it does not exist yet; at or above `R` the rule ends it), so
//! replacing it changes no answer, and retention can then reclaim it.
//!
//! Only history GC does this. At write time a later-arriving live record of
//! the node between `D` and `R` (out-of-order replication) would make the
//! version live again, and a stored `T` cannot be taken back; below the GC
//! cutoff that window is assumed closed, as for every other version GC drops.

use super::{stage_version, OverlayTarget};
use crate::repositories::translations::keys;
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::{WriteBatch, DB};

/// Stage `T` over every live node and block overlay version of `node_id`
/// stored strictly between `deleted_at` and `next_record` (`None`: no upper
/// bound). Returns how many were staged.
pub(crate) fn materialize_dead_generation(
    db: &DB,
    batch: &mut WriteBatch,
    scope: (&str, &str, &str, &str),
    node_id: &str,
    deleted_at: &HLC,
    next_record: Option<&HLC>,
) -> Result<usize> {
    let (tenant_id, repo_id, branch, workspace) = scope;
    let in_range = |at: &HLC| at > deleted_at && next_record.is_none_or(|next| at < next);
    // (block, locale, revision) of every live version in range.
    let mut found: Vec<(Option<String>, String, HLC)> = Vec::new();
    for (cf_name, prefix, depth) in [
        (
            crate::cf::TRANSLATION_DATA,
            keys::translation_node_prefix(tenant_id, repo_id, branch, workspace, node_id),
            1,
        ),
        (
            crate::cf::BLOCK_TRANSLATIONS,
            keys::block_translations_node_prefix(tenant_id, repo_id, branch, workspace, node_id),
            2,
        ),
    ] {
        let cf = crate::cf_handle(db, cf_name)?;
        for item in crate::prefix_scan(db, cf, &prefix) {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
            let Some(suffix) = key.strip_prefix(prefix.as_slice()) else {
                break;
            };
            if crate::keys::is_tombstone_value(&value) || suffix.len() <= 16 {
                continue;
            }
            let Ok(at) = crate::keys::extract_revision_from_key(&key) else {
                continue;
            };
            if !in_range(&at) {
                continue;
            }
            // The text segments only: the 16-byte revision may contain `\0`.
            let Some(text) = suffix[..suffix.len() - 16].strip_suffix(b"\0") else {
                continue;
            };
            let segments = match text
                .split(|b| *b == 0)
                .map(std::str::from_utf8)
                .collect::<std::result::Result<Vec<&str>, _>>()
            {
                Ok(segments) if segments.len() == depth => segments,
                _ => continue,
            };
            match segments.as_slice() {
                [locale] => found.push((None, (*locale).to_string(), at)),
                [block, locale] if *locale != keys::BLOCK_ORPHAN_MARKER => {
                    found.push((Some((*block).to_string()), (*locale).to_string(), at))
                }
                _ => {}
            }
        }
    }
    for (block, locale, at) in &found {
        let target = OverlayTarget {
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            block_uuid: block.as_deref(),
            locale,
        };
        stage_version(db, batch, &target, None, at)?;
    }
    Ok(found.len())
}
