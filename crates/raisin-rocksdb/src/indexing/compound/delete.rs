//! Compound tombstones for a node DELETE (`tombstones::add_node_tombstones*`,
//! shared by the local, cascade, merge and replicated deletes).
//!
//! With the node type's definitions cached, the deleted version's groups are
//! DERIVED (no read but the landing seek). The tombstoner is synchronous and
//! has no NodeType, so on a cold cache it falls back to finding the node's
//! live TYPE-index groups by a scan of the workspace's compound keyspace —
//! the old behaviour's cost, kept for the cold case only. The workspace's own
//! indexes (plan Phase 13e) are always derived: their declarations need no
//! NodeType, and their keyspaces hold every node of the workspace, so the
//! scan skips them (scanning them would cost every entry of the workspace per
//! deleted node). Either way the tombstone is a NEW key at the delete
//! revision; the old code wrote `T` over the live key itself, which erased the
//! entry from every historical read.

use super::defs::DefsSet;
use super::entries::{compound_entries, entry_key, parse_entry_key};
use super::group::tombstone_revision;
use super::writer::reassert_successors;
use crate::indexing::IndexCtx;
use crate::keys::{self, TOMBSTONE_VALUE as TOMBSTONE};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_storage::BranchScope;
use rocksdb::{WriteBatch, DB};
use std::collections::HashSet;

/// The first byte of every workspace-owned keyspace name (`@{name}`).
const WORKSPACE_INDEX_FIRST_BYTE: u8 =
    raisin_models::nodes::properties::schema::WORKSPACE_INDEX_PREFIX.as_bytes()[0];

/// Tombstone every compound entry of `node` (the deleted version) at
/// `revision`. A delete landing BELOW stored versions (a replicated delete
/// older than a local update) re-asserts them afterwards.
pub fn tombstone_compound_for_delete(
    batch: &mut WriteBatch,
    db: &DB,
    ctx: &IndexCtx<'_>,
    node: &Node,
    revision: &HLC,
) -> Result<()> {
    let cf = crate::cf_handle(db, crate::cf::COMPOUND_INDEX)?;
    let successors = crate::mvcc_read::node_versions_above(
        db,
        ctx.tenant_id,
        ctx.repo_id,
        ctx.branch,
        ctx.workspace,
        &node.id,
        revision,
    )?;
    // With successors stored, the caller's `node` may be the NEWEST version
    // (a replicated delete arriving late loads the latest), not the one this
    // delete ends: the version in force at the delete revision is the newest
    // at or below it. The cold scan below already decides that way.
    let replaced;
    let ended: Option<&Node> = if successors.is_empty() {
        Some(node)
    } else {
        // Nothing live at the delete revision: nothing for it to end.
        replaced = crate::mvcc_read::node_version_at_or_before(
            db,
            ctx.tenant_id,
            ctx.repo_id,
            ctx.branch,
            ctx.workspace,
            &node.id,
            Some(revision),
        )?
        .and_then(|(_, version)| version);
        replaced.as_ref()
    };
    let mut types: Vec<&str> = ended.iter().map(|n| n.node_type.as_str()).collect();
    types.extend(
        successors
            .iter()
            .filter_map(|(_, v)| v.as_ref().map(|n| n.node_type.as_str())),
    );
    let scope = BranchScope::new(ctx.tenant_id, ctx.repo_id, ctx.branch);
    // The workspace's own indexes hold every node of it (plan Phase 13e).
    let ws = super::workspace_defs::for_ctx(db, ctx)?;
    let (defs, cold) = match DefsSet::peek(db, scope, types.iter().copied()) {
        Some(defs) => (defs, false),
        None => (DefsSet::workspace_only(types.iter().copied()), true),
    };
    let ended_groups = ended
        .map(|n| compound_entries(defs.compound(&n.node_type).iter().chain(&ws[..]), ctx, n))
        .unwrap_or_default();
    for group in ended_groups {
        let at = if successors.is_empty() {
            tombstone_revision(db, cf, &group, &node.id, revision)?
        } else {
            *revision
        };
        batch.put_cf(cf, entry_key(&group, &at, &node.id), TOMBSTONE);
    }
    reassert_successors(batch, cf, ctx, &defs, &ws, &node.id, &successors);
    if cold {
        scan_fallback(batch, db, ctx, node, revision, successors.is_empty())?;
    }
    Ok(())
}

/// Cold cache: find the node's live TYPE-index groups by scanning the
/// workspace's compound keyspace (both tags, workspace-owned `@` keyspaces
/// skipped — derived by the caller) and end each at `revision` — or, with no
/// successors, on the group's newest entry when that is above `revision`.
/// With successors, groups whose newest entry is above `revision` are theirs
/// and left alone (no definitions to re-assert them with: a known gap only
/// when `index.skip_unchanged` is on and the cache is cold).
fn scan_fallback(
    batch: &mut WriteBatch,
    db: &DB,
    ctx: &IndexCtx<'_>,
    node: &Node,
    revision: &HLC,
    no_successors: bool,
) -> Result<()> {
    let cf = crate::cf_handle(db, crate::cf::COMPOUND_INDEX)?;
    let mut decided: HashSet<Vec<u8>> = HashSet::new();
    for published in [false, true] {
        let prefix = keys::compound_index_workspace_prefix(
            ctx.tenant_id,
            ctx.repo_id,
            ctx.branch,
            ctx.workspace,
            published,
        );
        let mut opts = rocksdb::ReadOptions::default();
        opts.set_total_order_seek(true);
        if let Some(upper) = crate::prefix_successor(&prefix) {
            opts.set_iterate_upper_bound(upper);
        }
        let mut iter = db.raw_iterator_cf_opt(cf, opts);
        iter.seek(&prefix);
        while iter.valid() {
            let (Some(key), Some(value)) = (iter.key(), iter.value()) else {
                break;
            };
            if !key.starts_with(&prefix) {
                break;
            }
            if key.get(prefix.len()) == Some(&WORKSPACE_INDEX_FIRST_BYTE) {
                // Every `@` keyspace sorts together: seek past all of them.
                let mut past = prefix.clone();
                past.push(WORKSPACE_INDEX_FIRST_BYTE + 1);
                iter.seek(&past);
                continue;
            }
            let (key, value) = (key.to_vec(), value.to_vec());
            iter.next();
            let Some((group, at, owner)) = parse_entry_key(&key) else {
                continue;
            };
            if owner != node.id || decided.contains(group) {
                continue;
            }
            if !no_successors && at > *revision {
                continue; // a successor's entry; the newest at or below decides
            }
            decided.insert(group.to_vec());
            if keys::is_tombstone_value(&value) {
                continue;
            }
            let land = if at > *revision { at } else { *revision };
            batch.put_cf(cf, entry_key(group, &land, &node.id), TOMBSTONE);
        }
        iter.status().map_err(|e| {
            raisin_error::Error::storage(format!("Failed to iterate compound index: {}", e))
        })?;
    }
    tracing::debug!(
        node_id = %node.id,
        "compound definitions not cached: delete tombstones found by a workspace scan"
    );
    Ok(())
}
