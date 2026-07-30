//! The single writer for spatial index entries.
//!
//! Every write path in the system — the transaction context (SQL INSERT/UPDATE,
//! bulk DML and `NodeService`), the repository `add`/`update` path, the replication
//! apply path, the delete/tombstone path and the reindex job — funnels through the
//! functions here and in [`super::spatial_tombstone`]. Nothing else may construct
//! a spatial index key.
//!
//! Geometries are found by [`super::spatial_walk::walk_geometries`], which
//! descends the whole property tree — so a geometry nested in an `Element`,
//! `Object` or `Array` is indexed under its dot path (`venue.geo`,
//! `stops.0.geo`) rather than being silently invisible to every spatial query.

use super::spatial_policy::NodeSpatialPolicies;
use super::spatial_walk::{walk_geometries, walk_geometries_capped};
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
/// The tombstone fallback when nothing is known about what was actually written:
/// a precision that has since been removed from the configuration still gets its
/// entry tombstoned. Mirrors `raisin_models::nodes::properties::PRECISION_RANGE`
/// (`1..=12`); kept as a local list because that range is not re-exported from the
/// `properties` module.
pub(super) const ALL_PRECISIONS: &[usize] = &[12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1];

/// The precision set one tombstone pass must cover, plus the policy whose
/// non-precision fields (`cover`, `srid`) decide which cells those precisions
/// map to.
///
/// A struct rather than two arguments because passing a precision list that does
/// not belong to the accompanying policy is a silent under-tombstone, and the two
/// are only ever produced together by
/// [`NodeSpatialPolicies::tombstone_precisions`].
pub struct TombstonePrecisions<'a> {
    pub policy: &'a SpatialPolicy,
    pub precisions: Vec<usize>,
}

impl<'a> TombstonePrecisions<'a> {
    /// Tombstone at EVERY precision the index could ever have used.
    ///
    /// The safe answer whenever nothing is known about what is physically
    /// present. Costs twelve puts.
    pub fn every(policy: &'a SpatialPolicy) -> Self {
        Self {
            policy,
            precisions: ALL_PRECISIONS.to_vec(),
        }
    }

    /// Tombstone at a known-sufficient set — `configured ∪ indexed`.
    ///
    /// Only correct when the local index-state record was consulted; see
    /// [`super::NodeSpatialPolicies::tombstone_precisions`].
    pub fn bounded(policy: &'a SpatialPolicy, precisions: Vec<usize>) -> Self {
        Self { policy, precisions }
    }
}

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

/// Write spatial index entries for every `PropertyValue::Geometry` on `node`,
/// **at any depth** in its property tree.
///
/// Staged into the caller's `batch`, so the entries commit atomically with the node
/// record. Zero reads: the cells are derived from the geometry.
///
/// Cost is exactly `|policy.precisions|` `put_cf` calls per geometry PATH under
/// the default `Centroid` cover — eight with the default precision set. A node
/// holding one top-level geometry and three nested ones therefore costs four
/// times that, which is why [`super::spatial_walk::MAX_GEOMETRY_PATHS_PER_NODE`]
/// caps the path count.
pub fn write_node_spatial_indexes(
    batch: &mut WriteBatch,
    targets: &SpatialIndexTargets<'_>,
    ctx: &IndexCtx<'_>,
    node: &Node,
    revision: &HLC,
    policies: &NodeSpatialPolicies,
) -> Result<()> {
    let walked = walk_geometries_capped(&node.properties, &node.id);
    for (property_path, geometry) in &walked.indexed {
        let policy = policies.for_property(property_path);
        let bucket = resolve_bucket(node, policy);
        write_spatial_property(
            batch,
            targets,
            ctx,
            &node.id,
            property_path,
            geometry,
            revision,
            policy,
            bucket.as_deref(),
        )?;
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
