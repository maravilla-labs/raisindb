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
            self.load_node_after(tenant_id, repo_id, branch, workspace, &node.id, revision)?
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

        crate::repositories::add_stale_property_tombstones(
            batch,
            cf_handle(&self.db, cf::PROPERTY_INDEX)?,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node,
            superseding,
            &newer_rev,
        );
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
            let same_place = match &newer {
                Some(n) => {
                    n.order_key == placement.label
                        && self
                            .resolve_parent_id_for_snapshot(
                                tenant_id, repo_id, branch, workspace, n,
                            )
                            .ok()
                            .flatten()
                            .as_deref()
                            == Some(placement.parent_id)
                }
                None => false,
            };
            if !same_place {
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
                batch.put_cf(cf_handle(&self.db, cf::ORDERED_CHILDREN)?, key, TOMBSTONE);
            }
        }
        Ok(())
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
    /// The OLDEST stored version of `node_id` strictly above `revision` in
    /// `workspace`: `Some((rev, Some(node)))`, `Some((rev, None))` when that
    /// version is a tombstone, or `None` when `revision` is the newest.
    ///
    /// An op applied out of order (older than what is already stored) must
    /// have its values superseded at exactly this revision — see
    /// [`Self::tombstone_superseded_by_newer`].
    #[allow(clippy::too_many_arguments)]
    fn load_node_after(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        revision: &HLC,
    ) -> Result<Option<(HLC, Option<Node>)>> {
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
            &self.db, &value, tenant_id, repo_id, branch, workspace, node_id, &found,
        )?;
        node.workspace = Some(workspace.to_string());
        Ok(Some((found, Some(node))))
    }
}
