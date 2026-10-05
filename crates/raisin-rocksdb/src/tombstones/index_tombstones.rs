//! Index tombstone functions: property, reference, relation, spatial, translation
//! (compound: `indexing::compound::delete`)

use super::helpers::{hash_property_value, parse_relation_from_forward_key};
use super::{TombstoneColumnFamilies, TombstoneContext, TOMBSTONE};
use crate::keys;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_models::nodes::{INDEXED_MIXIN_KEY, INDEXED_SUPERTYPE_KEY};
use rocksdb::{WriteBatch, DB};

/// Tombstone all property indexes (PROPERTY_INDEX CF): every entry the
/// node's version indexes — custom properties, pseudo-properties and IS_A /
/// HAS_MIXIN membership — through the ONE entry derivation the writer uses
/// (`indexing::property_delta`), so a delete can never miss an entry the
/// write path created.
///
/// A delete landing BELOW a stored version (a replicated delete older than a
/// local update) re-asserts every such successor's entries at its own
/// revision afterwards: a successor written with skip-unchanged keeps
/// unchanged entries below the delete, and the delete's tombstones would
/// otherwise mask them at HEAD.
pub(super) fn tombstone_property_indexes(
    batch: &mut WriteBatch,
    db: &DB,
    ctx: &TombstoneContext,
    cfs: &TombstoneColumnFamilies,
    node: &Node,
    revision: &HLC,
) -> Result<()> {
    let index_ctx =
        crate::indexing::IndexCtx::new(ctx.tenant_id, ctx.repo_id, ctx.branch, ctx.workspace);
    crate::indexing::tombstone_all_entries(batch, cfs.property_index, &index_ctx, node, revision);
    let successors = crate::mvcc_read::node_versions_above(
        db,
        ctx.tenant_id,
        ctx.repo_id,
        ctx.branch,
        ctx.workspace,
        &node.id,
        revision,
    )?;
    crate::indexing::reassert_successors(
        batch,
        cfs.property_index,
        &index_ctx,
        &node.id,
        &successors,
    );
    Ok(())
}

/// Tombstone reference indexes (REFERENCE_INDEX CF)
///
/// Extracts references from node properties and tombstones both forward and reverse indexes.
pub(super) fn tombstone_reference_indexes(
    batch: &mut WriteBatch,
    ctx: &TombstoneContext,
    cfs: &TombstoneColumnFamilies,
    node: &Node,
    revision: &HLC,
    is_published: bool,
) {
    // The ONE reference walker: the writers index exactly these paths, so a
    // tombstone written for any other path would shadow nothing.
    let refs = crate::repositories::walk_references(&node.properties);
    for (property_path, reference) in refs {
        // Tombstone forward index
        let forward_key = keys::reference_forward_key_versioned(
            ctx.tenant_id,
            ctx.repo_id,
            ctx.branch,
            ctx.workspace,
            &node.id,
            &property_path,
            revision,
            is_published,
        );
        batch.put_cf(cfs.reference_index, forward_key, TOMBSTONE);

        // Tombstone reverse index
        let reverse_key = keys::reference_reverse_key_versioned(
            ctx.tenant_id,
            ctx.repo_id,
            ctx.branch,
            ctx.workspace,
            &reference.workspace,
            &reference.id,
            &node.id,
            &property_path,
            revision,
            is_published,
        );
        batch.put_cf(cfs.reference_index, reverse_key, TOMBSTONE);
    }
}

/// Tombstone relation indexes (RELATION_INDEX CF)
///
/// NOTE: We must scan RELATION_INDEX to find actual relations, not use node.relations
/// which is always empty on read! This is critical for proper relation cleanup.
pub(super) fn tombstone_relation_indexes(
    batch: &mut WriteBatch,
    db: &DB,
    ctx: &TombstoneContext,
    cfs: &TombstoneColumnFamilies,
    node: &Node,
    revision: &HLC,
) -> Result<()> {
    // Scan for outgoing relations from this node
    let relation_prefix = keys::relation_forward_prefix(
        ctx.tenant_id,
        ctx.repo_id,
        ctx.branch,
        ctx.workspace,
        &node.id,
    );

    let mut seen_edges = std::collections::HashSet::new();
    let iter = crate::prefix_scan(&db, cfs.relation_index, &relation_prefix);
    for item in iter {
        let (key, value) = item.map_err(|e| {
            raisin_error::Error::storage(format!("Failed to iterate relations: {}", e))
        })?;

        // Stop when leaving prefix
        if !key.starts_with(&relation_prefix) {
            break;
        }

        // Dedupe on (relation_type, target) from the CHEAP key parse before
        // touching the value — the prefix scan yields every historical
        // revision of every edge, newest first.
        let Some((key_type, _, key_target)) =
            parse_relation_from_forward_key(&key, &relation_prefix)
        else {
            continue;
        };
        if !seen_edges.insert((key_type, key_target)) {
            continue;
        }

        // Newest entry is already a tombstone: edge is gone, nothing to write.
        if value.as_ref() == TOMBSTONE {
            continue;
        }

        // The forward VALUE is a serialized RelationRef carrying the target's
        // workspace — the key alone doesn't contain it (the old key-parse
        // helper returned an empty workspace, so reverse tombstones never
        // matched the real reverse keys and incoming edges survived deletion).
        let parsed = crate::repositories::deserialize_relation_ref(&value)
            .ok()
            .map(|r| (r.relation_type, r.workspace, r.target));
        if let Some((relation_type, target_workspace, target_id)) = parsed {
            // Tombstone forward relation
            let fwd_key = keys::relation_forward_key_versioned(
                ctx.tenant_id,
                ctx.repo_id,
                ctx.branch,
                ctx.workspace,
                &node.id,
                &relation_type,
                revision,
                &target_id,
            );
            batch.put_cf(cfs.relation_index, fwd_key, TOMBSTONE);

            // Tombstone reverse relation
            let rev_key = keys::relation_reverse_key_versioned(
                ctx.tenant_id,
                ctx.repo_id,
                ctx.branch,
                &target_workspace,
                &target_id,
                &relation_type,
                revision,
                &node.id,
            );
            batch.put_cf(cfs.relation_index, rev_key, TOMBSTONE);
        }
    }

    // INCOMING edges (source -> deleted node): tombstone the legacy per-edge
    // keys from both sides. Sources are enumerated via the shared collector,
    // which reads the SOURCE workspace from the reverse value (the key's
    // workspace segment is the target's).
    let incoming = crate::repositories::collect_incoming_relations(
        db,
        ctx.tenant_id,
        ctx.repo_id,
        ctx.branch,
        ctx.workspace,
        &node.id,
    )?;
    for (relation_type, source_workspace, source_id) in &incoming {
        let fwd_key = keys::relation_forward_key_versioned(
            ctx.tenant_id,
            ctx.repo_id,
            ctx.branch,
            source_workspace,
            source_id,
            relation_type,
            revision,
            &node.id,
        );
        batch.put_cf(cfs.relation_index, fwd_key, TOMBSTONE);

        let rev_key = keys::relation_reverse_key_versioned(
            ctx.tenant_id,
            ctx.repo_id,
            ctx.branch,
            ctx.workspace,
            &node.id,
            relation_type,
            revision,
            source_id,
        );
        batch.put_cf(cfs.relation_index, rev_key, TOMBSTONE);
    }

    // ALSO clear the PACKED adjacency lists (see
    // relations::helpers::packed_adjacency_cleanup_puts) — same batch, so the
    // packed rewrite stays atomic with the tombstones above.
    let puts = crate::repositories::packed_adjacency_cleanup_puts(
        db,
        revision,
        ctx.tenant_id,
        ctx.repo_id,
        ctx.branch,
        ctx.workspace,
        &node.id,
        incoming
            .into_iter()
            .map(|(_, src_ws, src_id)| (src_ws, src_id)),
    )?;
    for (key, value) in puts {
        batch.put_cf(cfs.relation_index, key, value);
    }

    Ok(())
}

/// Tombstone spatial indexes (SPATIAL_INDEX CF)
///
/// # Why this no longer scans
///
/// The previous implementation prefix-iterated the ENTIRE workspace spatial range
/// on EVERY node delete — whether or not the node carried any geometry at all —
/// matching keys with `extract_node_id_from_key`, which splits on `\0` and takes the
/// last fragment. That is O(all geometries in the workspace) per single-node delete,
/// the largest write cost in the subsystem, and directly at odds with the
/// 5k-writes/sec target on a workspace holding bulk-loaded geo data.
///
/// The cells are now **derived** from the node's own geometry properties, which the
/// node blob already carries: O(precisions) puts, zero reads, and an immediate exit
/// for the overwhelming majority of nodes that hold no geometry. It also routes key
/// construction through the one shared spatial writer, so a delete tombstones
/// exactly the cells a write produced.
pub(super) fn tombstone_spatial_indexes(
    batch: &mut WriteBatch,
    db: &DB,
    ctx: &TombstoneContext,
    cfs: &TombstoneColumnFamilies,
    node: &Node,
    revision: &HLC,
) -> Result<()> {
    // The whole property tree, not just the top level: a geometry nested in an
    // Element / Object / Array is indexed by the writer, so a delete that only
    // walked the top level would leave it live and the deleted node would keep
    // matching ST_DWITHIN forever. Same walker as the writer, so the paths — and
    // therefore the keys — line up exactly.
    let geometries = crate::indexing::walk_geometries(&node.properties);
    if geometries.is_empty() {
        return Ok(());
    }

    let index_ctx =
        crate::indexing::IndexCtx::new(ctx.tenant_id, ctx.repo_id, ctx.branch, ctx.workspace);
    let targets = crate::indexing::SpatialIndexTargets {
        spatial_index: cfs.spatial_index,
    };

    // The precision set comes from `configured ∪ indexed` when a state record
    // exists, and widens to every precision in `PRECISION_RANGE` when one does
    // not. The asymmetry is deliberately in favour of over-tombstoning: a
    // superfluous tombstone shadows nothing, while a missing one leaves a deleted
    // node matching forever.
    for (property_path, geometry) in geometries {
        let (policy, bounded) = crate::spatial_state::tombstone_policy_for_property(
            db,
            ctx.tenant_id,
            ctx.repo_id,
            ctx.branch,
            ctx.workspace,
            &property_path,
        );
        let precisions = match bounded {
            Some(set) => crate::indexing::TombstonePrecisions::bounded(&policy, set),
            None => crate::indexing::TombstonePrecisions::every(&policy),
        };
        crate::indexing::tombstone_spatial_property(
            batch,
            &targets,
            &index_ctx,
            &node.id,
            &property_path,
            geometry,
            revision,
            precisions,
        )?;
    }

    Ok(())
}
