//! Tombstoning superseded spatial index entries.
//!
//! Split from [`super::spatial`] — the writer — because it answers the harder
//! question. Writing is a pure function of the geometry; tombstoning has to
//! decide **which entries could exist** for a geometry that is going away, and
//! getting that wrong in the permissive direction leaves a live stale entry that
//! survives every subsequent update and delete.
//!
//! Two rules carry that weight:
//!
//! * **Paths are compared path-keyed**, using the SAME walker the writer uses
//!   ([`super::spatial_walk::walk_geometries`]). A writer and a tombstoner that
//!   disagreed about the dot-path format would emit tombstones that shadow
//!   nothing.
//! * **Precision sets are bounded only by evidence.** `configured ∪ indexed`
//!   when the local state record was consulted; all twelve precisions when it
//!   was not. Over-tombstoning costs writes; under-tombstoning is a false
//!   positive in the subsystem that just spent a pass eliminating them.

use super::spatial::{SpatialIndexTargets, TombstonePrecisions, SPATIAL_TOMBSTONE};
use super::spatial_policy::NodeSpatialPolicies;
use super::spatial_walk::walk_geometries;
use super::IndexCtx;
use crate::keys;
use crate::spatial::cells_for_geometry;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::GeoJson;
use raisin_models::nodes::Node;
use rocksdb::WriteBatch;

/// Tombstone the spatial entries that `old_node` holds and `new_node` supersedes.
///
/// `new_node == None` means the node is being deleted, so every geometry path is
/// tombstoned. Otherwise only paths whose geometry actually changed (or that were
/// removed) are tombstoned — an unchanged geometry keeps its entries and the
/// re-write reproduces identical bytes, so the write is a no-op.
///
/// # Nested paths are compared PATH-KEYED, and that is the whole correctness
/// argument
///
/// Old and new are both walked with [`walk_geometries`], and the comparison is
/// keyed on the resulting dot path. So:
///
/// * a geometry edited in place at `venue.geo` tombstones `venue.geo`;
/// * a geometry that moved from `stops.0.geo` to `stops.1.geo` tombstones
///   `stops.0.geo` (absent from the new set at that path) and writes `stops.1.geo`;
/// * an element deleted from an array shortens the paths of everything after it,
///   so the trailing path disappears from the new set and is tombstoned.
///
/// Using the same walker on both sides is not a convenience — a writer and a
/// tombstoner that disagreed about the path format would leave entries that can
/// never be shadowed, i.e. a stale spatial hit surviving every subsequent update
/// and delete.
///
/// # Why this must derive cells instead of scanning
///
/// The previous implementation prefix-iterated the ENTIRE workspace spatial range
/// on every update and every delete, matching keys by `String::from_utf8_lossy`
/// plus `ends_with`, then re-splitting on `\0` and trusting `parts[6]` (unsafe
/// against the null bytes the descending HLC can contain). That is
/// O(all geometries in the workspace) per single-node write — the largest write
/// cost in the subsystem, and directly at odds with the 5k-writes/sec goal on a
/// workspace holding bulk-loaded geo data. Deriving from the old geometry is O(8)
/// with zero reads.
#[allow(clippy::too_many_arguments)]
pub fn tombstone_superseded_spatial_indexes(
    batch: &mut WriteBatch,
    targets: &SpatialIndexTargets<'_>,
    ctx: &IndexCtx<'_>,
    old_node: &Node,
    new_node: Option<&Node>,
    revision: &HLC,
    policies: &NodeSpatialPolicies,
) -> Result<()> {
    // The NEW node is walked WITHOUT the cap: a path the cap dropped from the
    // write is a path with no live entry, so treating it as "still present" and
    // skipping the tombstone would be correct only by accident. Comparing against
    // the full new set can at worst skip a tombstone for a path whose geometry is
    // unchanged, which is exactly the no-op case.
    let new_geometries: std::collections::HashMap<String, &GeoJson> = new_node
        .map(|node| walk_geometries(&node.properties).into_iter().collect())
        .unwrap_or_default();

    for (property_path, old_geometry) in walk_geometries(&old_node.properties) {
        // Keep the entries when the new revision carries the identical geometry at
        // the SAME path — the re-write is byte-identical, so a tombstone plus an
        // identical put at the same revision would be pure churn.
        if new_geometries
            .get(property_path.as_str())
            .is_some_and(|new_geometry| *new_geometry == old_geometry)
        {
            continue;
        }

        let policy = policies.for_property(&property_path);
        tombstone_spatial_property(
            batch,
            targets,
            ctx,
            &old_node.id,
            &property_path,
            old_geometry,
            revision,
            policies.tombstone_precisions(&property_path, policy),
        )?;
    }
    Ok(())
}

/// Tombstone the entries for one geometry-valued property path.
///
/// `precisions` is the set to tombstone at — see
/// [`NodeSpatialPolicies::tombstone_precisions`] for how it is chosen and why it
/// is no longer unconditionally all twelve. Superfluous tombstones (for cells
/// that were never written) are harmless — they shadow nothing — while a MISSING
/// tombstone leaves a live stale entry, so wherever the answer is unknown the
/// asymmetry stays in favour of over-tombstoning.
#[allow(clippy::too_many_arguments)]
pub fn tombstone_spatial_property(
    batch: &mut WriteBatch,
    targets: &SpatialIndexTargets<'_>,
    ctx: &IndexCtx<'_>,
    node_id: &str,
    property_name: &str,
    old_geometry: &GeoJson,
    revision: &HLC,
    precisions: TombstonePrecisions<'_>,
) -> Result<()> {
    let mut widened = precisions.policy.clone();
    widened.precisions = precisions.precisions;

    // Deliberately LENIENT where `write_spatial_property` is strict. An
    // un-normalisable SRID means no entry was ever written for this geometry, so
    // there is nothing to tombstone — and failing here would make a node holding
    // pre-normalisation data impossible to DELETE from, turning a historical
    // write bug into a permanent inability to clean it up.
    let computed = match cells_for_geometry(old_geometry, &widened) {
        Ok(Some(computed)) => computed,
        Ok(None) => return Ok(()),
        Err(e) => {
            tracing::warn!(
                node_id,
                property_name,
                srid = old_geometry.srid(),
                error = %e,
                "skipping spatial tombstones: the superseded geometry's SRID is not indexable, \
                 so it has no index entries to supersede"
            );
            return Ok(());
        }
    };

    for cell in &computed.cells {
        let key = keys::spatial_index_key_versioned(
            ctx.tenant_id,
            ctx.repo_id,
            ctx.branch,
            ctx.workspace,
            property_name,
            cell,
            revision,
            node_id,
        );
        batch.put_cf(targets.spatial_index, key, SPATIAL_TOMBSTONE);
    }

    Ok(())
}
