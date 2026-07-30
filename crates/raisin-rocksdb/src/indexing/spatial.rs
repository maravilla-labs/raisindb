//! The single writer for spatial index entries.
//!
//! Every write path in the system — the transaction context (SQL INSERT/UPDATE,
//! bulk DML and `NodeService`), the repository `add`/`update` path, the replication
//! apply path, the delete/tombstone path and the reindex job — funnels through the
//! two functions here. Nothing else may construct a spatial index key.

use super::spatial_policy::NodeSpatialPolicies;
use super::IndexCtx;
use crate::repositories::spatial_index::{
    SpatialEntry, SpatialGeometryKind, SPATIAL_ENTRY_VERSION,
};
use crate::spatial::cells_for_geometry;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::{GeoJson, PropertyValue, SpatialPolicy};
use raisin_models::nodes::Node;
use rocksdb::{ColumnFamily, WriteBatch, DB};

/// The tombstone marker written over a superseded spatial index entry.
///
/// Matches `crate::tombstones::TOMBSTONE`, because a reader cannot tell which path
/// produced an entry and must recognise one marker.
pub const SPATIAL_TOMBSTONE: &[u8] = b"T";

/// Every geohash precision the index may ever have used, finest first.
///
/// Tombstoning widens to this list rather than the currently configured one, so a
/// precision that has since been removed from the configuration still gets its
/// entry tombstoned. Mirrors `raisin_models::nodes::properties::PRECISION_RANGE`
/// (`1..=12`); kept as a local list because that range is not re-exported from the
/// `properties` module.
const ALL_PRECISIONS: &[usize] = &[12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1];

/// Column families the spatial writer needs.
///
/// A struct rather than loose arguments so a caller cannot pass the wrong handle,
/// which would corrupt an unrelated column family with spatial keys.
pub struct SpatialIndexTargets<'a> {
    pub spatial_index: &'a ColumnFamily,
}

impl<'a> SpatialIndexTargets<'a> {
    /// Resolve the handle from a database.
    pub fn from_db(db: &'a DB) -> Result<Self> {
        Ok(Self {
            spatial_index: cf_handle(db, cf::SPATIAL_INDEX)?,
        })
    }
}

/// Write spatial index entries for every `PropertyValue::Geometry` on `node`.
///
/// Staged into the caller's `batch`, so the entries commit atomically with the node
/// record. Zero reads: the cells are derived from the geometry.
///
/// Cost is exactly `|policy.precisions|` `put_cf` calls per geometry property under
/// the default `Centroid` cover — eight with the default precision set.
pub fn write_node_spatial_indexes(
    batch: &mut WriteBatch,
    targets: &SpatialIndexTargets<'_>,
    ctx: &IndexCtx<'_>,
    node: &Node,
    revision: &HLC,
    policies: &NodeSpatialPolicies,
) -> Result<()> {
    for (property_name, value) in &node.properties {
        if let PropertyValue::Geometry(geometry) = value {
            let policy = policies.for_property(property_name);
            let bucket = resolve_bucket(node, policy);
            write_spatial_property(
                batch,
                targets,
                ctx,
                &node.id,
                property_name,
                geometry,
                revision,
                policy,
                bucket.as_deref(),
            )?;
        }
    }
    Ok(())
}

/// Write the index entries for one geometry-valued property.
///
/// # Errors
///
/// The geometry carries an SRID the built-in projection tier cannot normalise to
/// WGS84 (see [`crate::spatial::normalize_geometry_for_index`]). The write is
/// FAILED rather than skipped: a geometry that is stored but absent from the
/// index is invisible to every `ST_DWITHIN` / `ST_DISTANCE` query, forever, with
/// no signal anywhere — silent success was the bug.
#[allow(clippy::too_many_arguments)]
pub fn write_spatial_property(
    batch: &mut WriteBatch,
    targets: &SpatialIndexTargets<'_>,
    ctx: &IndexCtx<'_>,
    node_id: &str,
    property_name: &str,
    geometry: &GeoJson,
    revision: &HLC,
    policy: &SpatialPolicy,
    bucket: Option<&str>,
) -> Result<()> {
    // Normalisation to WGS84 happens inside `cells_for_geometry`; an SRID it
    // cannot normalise propagates as an error and fails the whole write batch.
    let Some(computed) = cells_for_geometry(geometry, policy)? else {
        // No usable position (empty geometry, or coordinates outside the WGS84
        // domain). Nothing to index, and nothing a query could match.
        return Ok(());
    };

    let entry = SpatialEntry {
        v: SPATIAL_ENTRY_VERSION,
        lon: computed.centroid.0,
        lat: computed.centroid.1,
        bbox: computed.bbox,
        z: computed.z_range,
        srid: geometry.srid().unwrap_or(4326),
        gtype: SpatialGeometryKind::of(geometry),
        bucket: bucket.map(|b| b.to_string()),
        policy_hash: policy.policy_hash(),
    };
    let value = entry.encode();

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
        batch.put_cf(targets.spatial_index, key, &value);
    }

    Ok(())
}

/// Tombstone the spatial entries that `old_node` holds and `new_node` supersedes.
///
/// `new_node == None` means the node is being deleted, so every geometry property
/// is tombstoned. Otherwise only properties whose geometry actually changed (or
/// that were removed) are tombstoned — an unchanged geometry keeps its entries and
/// the re-write reproduces identical bytes, so the write is a no-op.
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
    for (property_name, old_value) in &old_node.properties {
        let PropertyValue::Geometry(old_geometry) = old_value else {
            continue;
        };

        // Keep the entries when the new revision carries the identical geometry —
        // the re-write is byte-identical, so a tombstone plus an identical put at
        // the same revision would be pure churn.
        if let Some(new_node) = new_node {
            if let Some(PropertyValue::Geometry(new_geometry)) =
                new_node.properties.get(property_name)
            {
                if new_geometry == old_geometry {
                    continue;
                }
            }
        }

        let policy = policies.for_property(property_name);
        tombstone_spatial_property(
            batch,
            targets,
            ctx,
            &old_node.id,
            property_name,
            old_geometry,
            revision,
            policy,
        )?;
    }
    Ok(())
}

/// Tombstone the entries for one geometry-valued property.
///
/// Tombstones are emitted at the **union** of the given policy's precisions and
/// every precision in [`raisin_models::nodes::properties::PRECISION_RANGE`] that
/// the geometry's centroid maps to. Superfluous tombstones (for cells that were
/// never written) are harmless — they shadow nothing — while a MISSING tombstone
/// leaves a live stale entry, so the asymmetry is deliberately in favour of
/// over-tombstoning. This is what keeps a configuration change from stranding
/// entries at a precision that is no longer configured.
#[allow(clippy::too_many_arguments)]
pub fn tombstone_spatial_property(
    batch: &mut WriteBatch,
    targets: &SpatialIndexTargets<'_>,
    ctx: &IndexCtx<'_>,
    node_id: &str,
    property_name: &str,
    old_geometry: &GeoJson,
    revision: &HLC,
    policy: &SpatialPolicy,
) -> Result<()> {
    let mut widened = policy.clone();
    widened.precisions = ALL_PRECISIONS.to_vec();

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

/// Read the configured discriminator value off a node.
///
/// The bucket lives in the index **value**, not the key — putting a floor/level
/// segment in the key would change every key, the CF prefix extractor and every
/// parse site, and it would spend the write budget reserved for the multi-scale
/// precision set. In the value it costs nothing extra and still lets a
/// floor-filtered proximity query reject candidates before any node fetch.
fn resolve_bucket(node: &Node, policy: &SpatialPolicy) -> Option<String> {
    let property = policy.bucket_property.as_deref()?;
    match node.properties.get(property)? {
        PropertyValue::String(s) => Some(s.clone()),
        PropertyValue::Integer(i) => Some(i.to_string()),
        PropertyValue::Float(f) => Some(f.to_string()),
        PropertyValue::Boolean(b) => Some(b.to_string()),
        _ => None,
    }
}
