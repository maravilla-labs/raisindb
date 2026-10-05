//! An upsert that arrives OLDER than a version already stored.
//!
//! Replication does not deliver in revision order. When r2 has been applied
//! and r1 (< r2) arrives, r1's blob and index entries are still written at r1 —
//! a time-travel read at r1 must see them. But the diff that superseded the
//! previous version's entries ran when r2 was applied, against whatever was
//! stored THEN; r1's values did not exist yet. Written and left alone, an r1
//! value that r2 does not carry has nothing newer above it and matches at
//! HEAD forever (`title = 'B'` returning a node whose title is `C`).
//!
//! So every r1 entry the next-newer version does not carry is tombstoned at
//! that version's revision — through the same stale-entry helpers a normal
//! update uses, never a second implementation. When the next-newer version is
//! a delete, everything r1 wrote is superseded at the delete's revision.

use super::{OperationApplicator, TOMBSTONE};
use crate::indexing::{IndexCtx, NodeSpatialPolicies, SpatialIndexTargets};
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::WriteBatch;

/// Where the out-of-order version itself is placed among its siblings.
pub(super) struct OrderedPlacement<'a> {
    pub parent_id: &'a str,
    pub label: &'a str,
}

impl OperationApplicator {
    /// Tombstone, at the next newer stored revision, every entry `node` (being
    /// written at `revision`) has that the next newer version does not. A no-op
    /// when `revision` is the newest — the in-order case.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn tombstone_superseded_by_newer(
        &self,
        batch: &mut WriteBatch,
        ctx: &IndexCtx<'_>,
        spatial_targets: &SpatialIndexTargets<'_>,
        spatial_policies: &NodeSpatialPolicies,
        node: &Node,
        placement: Option<OrderedPlacement<'_>>,
        revision: &HLC,
    ) -> Result<()> {
        let IndexCtx {
            tenant_id,
            repo_id,
            branch,
            workspace,
            ..
        } = *ctx;
        let Some((newer_rev, newer)) =
            self.load_node_after(tenant_id, repo_id, branch, workspace, node, revision)?
        else {
            return Ok(());
        };
        tracing::debug!(
            node_id = %node.id,
            revision = %revision,
            newer_revision = %newer_rev,
            newer_is_delete = newer.is_none(),
            "replicated upsert is older than a stored version; superseding its entries"
        );

        // A delete supersedes every value: diff against an empty node.
        let emptied;
        let superseding: &Node = match &newer {
            Some(n) => n,
            None => {
                emptied = Node {
                    properties: Default::default(),
                    published_at: None,
                    ..node.clone()
                };
                &emptied
            }
        };

        // PROPERTY_INDEX is not handled here: the writer already ended this
        // version's values at the newer revision and re-asserted every newer
        // version's entries (`Baseline::OutOfOrder`, `write_successors`).
        crate::repositories::add_stale_reference_tombstones(
            batch,
            cf_handle(&self.db, cf::REFERENCE_INDEX)?,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node,
            superseding,
            &newer_rev,
        );
        crate::indexing::tombstone_superseded_spatial_indexes(
            batch,
            spatial_targets,
            ctx,
            node,
            newer.as_ref(),
            &newer_rev,
            spatial_policies,
        )?;

        if newer.as_ref().is_none_or(|n| n.path != node.path)
            && self.path_entry_would_be_live(ctx, node, revision, &newer_rev)?
        {
            let key = keys::path_index_key_versioned(
                tenant_id, repo_id, branch, workspace, &node.path, &newer_rev,
            );
            batch.put_cf(cf_handle(&self.db, cf::PATH_INDEX)?, key, TOMBSTONE);
        }

        if let Some(placement) = placement {
            // Still in this place at `newer_rev` exactly when the pair's
            // newest entry AT OR BELOW `newer_rev` is live — read the entry,
            // not the newer version's blob and path. Resolving the newer
            // version's PARENT by path at HEAD was wrong out of order: when an
            // ancestor's rename or move had not arrived yet, the new parent
            // path did not resolve, the placement looked changed, and the
            // node's only live entry was tombstoned at the newer revision —
            // gone from its parent's listing for good. (The blob's `order_key`
            // is no authority either: legacy blobs carry "".) Bounded by
            // revision, not the EXACT key at `newer_rev`: a newer version that
            // did not re-put an unchanged entry keeps it at an older revision,
            // and reading only the exact key tombstoned its only placement.
            let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
            let key = keys::ordered_child_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                placement.parent_id,
                placement.label,
                &newer_rev,
                &node.id,
            );
            let same_place = newer.is_some()
                && crate::repositories::nodes::live_entry_under_label(
                    &self.db,
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    placement.parent_id,
                    placement.label,
                    &node.id,
                    Some(&newer_rev),
                )?
                .is_some();
            if !same_place {
                batch.put_cf(cf_ordered, key, TOMBSTONE);
            }
        }
        Ok(())
    }

    /// Whether `path`'s newest PATH_INDEX entry at or before `at` maps to
    /// `node_id` — i.e. whether a tombstone at `at` would end THIS node's
    /// entry. Out of order, a node's old path may already belong to another
    /// node by `at` (it moved away in a version this replica has not seen
    /// yet, and someone else took the path); tombstoning it then deleted that
    /// other node's path.
    pub(super) fn path_owned_at(
        &self,
        ctx: &IndexCtx<'_>,
        path: &str,
        node_id: &str,
        at: &HLC,
    ) -> Result<bool> {
        // The one ownership check every old-path tombstone makes (origin
        // promotion and merge included): `indexing::key_owner`.
        crate::indexing::key_owner::path_owned_at(&self.db, ctx, path, node_id, at)
    }

    /// Whether `node`'s path entry written at `revision` would still be the
    /// newest entry for that path at `newer_rev`. PATH_INDEX is keyed by path,
    /// not id: if something else already claimed the path in between (another
    /// node, or a tombstone), a tombstone at `newer_rev` would wrongly end THAT
    /// entry instead.
    fn path_entry_would_be_live(
        &self,
        ctx: &IndexCtx<'_>,
        node: &Node,
        revision: &HLC,
        newer_rev: &HLC,
    ) -> Result<bool> {
        let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
        let prefix = keys::path_index_key_prefix(
            ctx.tenant_id,
            ctx.repo_id,
            ctx.branch,
            ctx.workspace,
            &node.path,
        );
        let newest =
            crate::mvcc_read::newest_at_or_before(&self.db, cf_path, &prefix, Some(newer_rev))?;
        Ok(match newest {
            None => true,
            Some((found, _)) if &found <= revision => true,
            Some((_, value)) => value == node.id.as_bytes(),
        })
    }
}

impl OperationApplicator {
    /// The OLDEST stored version of `incoming`'s node strictly above
    /// `revision` in `workspace`: `Some((rev, Some(node)))`, `Some((rev,
    /// None))` when that version is a tombstone, or `None` when `revision` is
    /// the newest.
    ///
    /// An op applied out of order (older than what is already stored) must
    /// have its values superseded at exactly this revision — see
    /// [`Self::tombstone_superseded_by_newer`].
    ///
    /// The newer version's PATH is its path as of its revision INCLUDING the
    /// `NODE_PATH` entry this apply is staging at `revision` (still in the
    /// batch, invisible to a committed read): a record that asserts no path —
    /// one a pre-v2 property-only op stored before those ops were removed
    /// (plan "Phase 11d"; data, so still read) — takes its
    /// path from the newest entry at or below it, and when that entry is not
    /// above `revision` it is this one. Read from committed state alone it was
    /// the pre-move path, so an out-of-order ancestor move tombstoned its own
    /// new PATH_INDEX entry at the newer revision.
    #[allow(clippy::too_many_arguments)]
    fn load_node_after(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        incoming: &Node,
        revision: &HLC,
    ) -> Result<Option<(HLC, Option<Node>)>> {
        let node_id = incoming.id.as_str();
        let cf_nodes = cf_handle(&self.db, cf::NODES)?;
        let prefix = keys::node_key_prefix(tenant_id, repo_id, branch, workspace, node_id);
        // Newest first: walk down until reaching `revision`; the last version
        // passed is the next newer one. Usually zero or one step.
        let mut next: Option<(HLC, Vec<u8>)> = None;
        for item in crate::prefix_scan(&self.db, cf_nodes, &prefix) {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
            if !key.starts_with(&prefix) {
                break;
            }
            let Ok(found) = keys::extract_revision_from_key(&key) else {
                continue;
            };
            if &found <= revision {
                break;
            }
            next = Some((found, value.to_vec()));
        }
        let Some((found, value)) = next else {
            return Ok(None);
        };
        if super::is_tombstone(&value) {
            return Ok(Some((found, None)));
        }
        let mut node = crate::mvcc_read::deserialize_node_with_path(
            &self.db, &value, tenant_id, repo_id, branch, workspace, node_id, &found, &found,
        )?;
        if crate::mvcc_read::embedded_path_of(&value).is_none() {
            let entry_prefix =
                keys::node_path_key_prefix(tenant_id, repo_id, branch, workspace, node_id);
            let indexed = crate::mvcc_read::newest_at_or_before(
                &self.db,
                cf_handle(&self.db, cf::NODE_PATH)?,
                &entry_prefix,
                Some(&found),
            )?;
            if indexed.is_none_or(|(at, _)| &at <= revision) {
                node.path = incoming.path.clone();
            }
        }
        node.workspace = Some(workspace.to_string());
        Ok(Some((found, Some(node))))
    }
}

/// Whether an upsert at `revision` is an in-place (`versionable=false`) write
/// OLDER than the version already stored at that very revision.
///
/// Two in-place writes of one node land on one key, so the revision carries no
/// order between them and the last one APPLIED would win. Delivered out of
/// order — a node's create applied after its in-place refresh, say — the
/// older content silently overwrote the newer, for good. Between versions at
/// one revision, `updated_at` (stamped by every write) decides; equal stamps
/// (a replayed duplicate) apply, which is idempotent.
pub(super) fn stale_in_place(at: &HLC, stored: &Node, revision: &HLC, incoming: &Node) -> bool {
    at == revision
        && match (stored.updated_at, incoming.updated_at) {
            (Some(stored_at), Some(incoming_at)) => stored_at > incoming_at,
            _ => false,
        }
}
