// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Spatial index write -> read round trip.
//!
//! Zero tests covered this before, which is how all of the following survived:
//!
//! * **Stale entries.** `find_within_radius` skipped a tombstoned entry with
//!   `continue` *without* recording the node as resolved. Revisions sort
//!   descending, so the iterator then reached an older live entry and emitted it:
//!   a deleted node still matched, and a moved node matched at BOTH its old and
//!   its new cell. `scan_cells` had the same defect.
//! * **The radius window.** The geohash prefix is `\0`-terminated, so it matches
//!   only entries at exactly that precision, while `precision_for_radius` walked
//!   12 -> 1 over precisions that were never indexed. Radii outside roughly
//!   4.8 m - 39 km returned nothing.
//! * **Dead KNN.** `find_nearest` computed its cell list and never used it, and
//!   its stopping rule answered "found enough" rather than "found the nearest".
//!
//! Every one of those has a test below, plus the byte-stability property the
//! reindex job's idempotency rests on and the legacy-value fallback that keeps an
//! upgraded server answering against a not-yet-reindexed index.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::{GeoJson, PropertyValue, SpatialPolicy};
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use raisin_rocksdb::repositories::spatial_index::{
    SpatialEntry, SpatialGeometryKind, SpatialIndexRepository, SPATIAL_ENTRY_VERSION,
};
use raisin_rocksdb::{fractional_index, RocksDBConfig, RocksDBStorage};
use raisin_storage::spatial::SpatialPreFilter;
use raisin_storage::{
    BranchRepository, CreateNodeOptions, DeleteNodeOptions, NodeRepository, RegistryRepository,
    RepositoryManagementRepository, Storage, StorageScope, UpdateNodeOptions,
};
use tempfile::TempDir;

const TENANT: &str = "spatial-test";
const REPO: &str = "repo";
const BRANCH: &str = "main";
const WS: &str = "places";
const PROP: &str = "location";

/// Far-future read snapshot, so every write is visible.
fn max_rev() -> HLC {
    HLC::new(u64::MAX / 2, 0)
}

struct Env {
    _dir: TempDir,
    storage: Arc<RocksDBStorage>,
}

impl Env {
    async fn new() -> Result<Self> {
        let temp_dir = TempDir::new().unwrap();
        let mut config = RocksDBConfig::default();
        config.path = temp_dir.path().to_path_buf();
        let storage = Arc::new(RocksDBStorage::with_config(config)?);

        storage
            .registry()
            .register_tenant(TENANT, HashMap::new())
            .await?;
        storage
            .repository_management()
            .create_repository(
                TENANT,
                REPO,
                RepositoryConfig {
                    default_language: "en".to_string(),
                    supported_languages: vec!["en".to_string()],
                    locale_fallback_chains: HashMap::new(),
                    default_branch: BRANCH.to_string(),
                    description: None,
                    tags: HashMap::new(),
                },
            )
            .await?;
        storage
            .branches()
            .create_branch(TENANT, REPO, BRANCH, "system", None, None, false, false)
            .await?;

        let workspace_service = WorkspaceService::new(storage.clone());
        let mut workspace = Workspace::new(WS.to_string());
        workspace.config.default_branch = BRANCH.to_string();
        workspace_service.put(TENANT, REPO, workspace).await?;

        Ok(Self {
            _dir: temp_dir,
            storage,
        })
    }

    fn spatial(&self) -> SpatialIndexRepository {
        SpatialIndexRepository::new(self.storage.db().clone())
    }

    fn scope(&self) -> StorageScope<'_> {
        StorageScope::new(TENANT, REPO, BRANCH, WS)
    }

    /// Radius query against the default precision set.
    fn within(&self, lon: f64, lat: f64, radius: f64) -> Vec<String> {
        let mut ids: Vec<String> = self
            .spatial()
            .find_within_radius(
                TENANT,
                REPO,
                BRANCH,
                WS,
                PROP,
                lon,
                lat,
                radius,
                &max_rev(),
                1000,
                raisin_rocksdb::spatial::INDEX_PRECISIONS,
                &SpatialPreFilter::default(),
            )
            .expect("radius query must not error")
            .into_iter()
            .map(|r| r.node_id)
            .collect();
        ids.sort();
        ids
    }
}

fn relaxed_create() -> CreateNodeOptions {
    CreateNodeOptions {
        validate_schema: false,
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        operation_meta: None,
    }
}

fn place(id: &str, lon: f64, lat: f64) -> Node {
    let mut properties = HashMap::new();
    properties.insert(
        PROP.to_string(),
        PropertyValue::Geometry(GeoJson::point(lon, lat)),
    );
    Node {
        id: id.to_string(),
        name: id.to_string(),
        path: format!("/{}", id),
        node_type: "test:Place".to_string(),
        archetype: None,
        properties,
        children: Vec::new(),
        order_key: fractional_index::first(),
        has_children: Some(false),
        parent: Some("/".to_string()),
        version: 1,
        created_at: Some(chrono::Utc::now()),
        updated_at: None,
        published_at: None,
        published_by: None,
        updated_by: Some("tester".to_string()),
        created_by: Some("tester".to_string()),
        translations: None,
        tenant_id: Some(TENANT.to_string()),
        workspace: Some(WS.to_string()),
        owner_id: None,
        relations: Vec::new(),
    }
}

// Zurich Hauptbahnhof, and points at controlled offsets from it.
const ZRH_LON: f64 = 8.5402;
const ZRH_LAT: f64 = 47.3782;

/// Offset a point by `east_m` / `north_m` metres.
fn offset(lon: f64, lat: f64, east_m: f64, north_m: f64) -> (f64, f64) {
    let per_deg_lat = 111_195.0_f64;
    let per_deg_lon = per_deg_lat * lat.to_radians().cos();
    (lon + east_m / per_deg_lon, lat + north_m / per_deg_lat)
}

// ---------------------------------------------------------------------------
// Automatic indexing via the ordinary write path
// ---------------------------------------------------------------------------

/// Indexing must be driven by the property TYPE, with no opt-in: writing a node
/// with a `PropertyValue::Geometry` through the normal repository API is enough.
///
/// This is also the regression test for the *repository* write path
/// (`storage.nodes().create`), which wrote node blob, path, node_path, property,
/// reference and relation indexes and NO spatial index — so every caller of it
/// produced geometry invisible to `ST_DWITHIN`, with no error.
#[tokio::test]
async fn geometry_is_indexed_automatically_on_create() -> Result<()> {
    let env = Env::new().await?;

    env.storage
        .nodes()
        .create(env.scope(), place("hb", ZRH_LON, ZRH_LAT), relaxed_create())
        .await?;

    assert_eq!(env.within(ZRH_LON, ZRH_LAT, 50.0), vec!["hb".to_string()]);
    Ok(())
}

// ---------------------------------------------------------------------------
// The stale-entry bug
// ---------------------------------------------------------------------------

/// A DELETED node must stop matching.
///
/// It did not before: the tombstone branch `continue`d without recording the node
/// as resolved, so the descending-revision iterator fell through to the older live
/// entry and emitted it.
#[tokio::test]
async fn deleted_node_stops_matching() -> Result<()> {
    let env = Env::new().await?;

    env.storage
        .nodes()
        .create(env.scope(), place("hb", ZRH_LON, ZRH_LAT), relaxed_create())
        .await?;
    assert_eq!(env.within(ZRH_LON, ZRH_LAT, 50.0), vec!["hb".to_string()]);

    env.storage
        .nodes()
        .delete(env.scope(), "hb", DeleteNodeOptions::default())
        .await?;

    assert!(
        env.within(ZRH_LON, ZRH_LAT, 50.0).is_empty(),
        "a deleted node must not match any radius query"
    );
    // Also at a wide radius, in case the delete only cleaned the fine cells.
    assert!(env.within(ZRH_LON, ZRH_LAT, 5_000.0).is_empty());
    Ok(())
}

/// A MOVED node must match at its NEW location and NOT at its old one.
///
/// This is the harder half of the same bug, and the reason the fix had to resolve
/// revisions GLOBALLY across the scanned cells rather than per cell: an update
/// writes tombstones at the old geometry's cells and live entries at the new
/// geometry's cells, and a radius query routinely scans both. A per-cell fix would
/// have made whichever cell the iterator reached first decide the node's fate —
/// old cell first and the node vanishes entirely, a false negative strictly worse
/// than the original bug.
#[tokio::test]
async fn moved_node_matches_only_at_its_new_location() -> Result<()> {
    let env = Env::new().await?;

    env.storage
        .nodes()
        .create(env.scope(), place("hb", ZRH_LON, ZRH_LAT), relaxed_create())
        .await?;

    // Move 400 m east — far enough to leave the fine cells, close enough that a
    // wide query still scans both old and new cells together.
    let (new_lon, new_lat) = offset(ZRH_LON, ZRH_LAT, 400.0, 0.0);
    let mut moved = place("hb", new_lon, new_lat);
    moved.version = 2;
    env.storage
        .nodes()
        .update(env.scope(), moved, UpdateNodeOptions::default())
        .await?;

    assert_eq!(
        env.within(new_lon, new_lat, 50.0),
        vec!["hb".to_string()],
        "the node must match at its new location"
    );
    assert!(
        env.within(ZRH_LON, ZRH_LAT, 50.0).is_empty(),
        "the node must NOT match at its old location"
    );

    // And exactly once when a single query covers both cells.
    let both = env.within(ZRH_LON, ZRH_LAT, 1_000.0);
    assert_eq!(
        both,
        vec!["hb".to_string()],
        "a query covering old and new cells must return the node exactly once"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// The radius window
// ---------------------------------------------------------------------------

/// Sub-metre through global radii must all work. The old implementation silently
/// returned zero rows outside roughly 4.8 m - 39 km, which made indoor queries
/// unusable and city-scale ones unreliable.
#[tokio::test]
async fn radii_from_sub_metre_to_global_all_work() -> Result<()> {
    let env = Env::new().await?;

    // A point 0.4 m east of the query centre, and one 300 km away.
    let (near_lon, near_lat) = offset(ZRH_LON, ZRH_LAT, 0.4, 0.0);
    let (far_lon, far_lat) = offset(ZRH_LON, ZRH_LAT, 300_000.0, 0.0);

    env.storage
        .nodes()
        .create(
            env.scope(),
            place("near", near_lon, near_lat),
            relaxed_create(),
        )
        .await?;
    env.storage
        .nodes()
        .create(
            env.scope(),
            place("far", far_lon, far_lat),
            relaxed_create(),
        )
        .await?;

    // Sub-metre: only the 0.4 m neighbour.
    assert_eq!(
        env.within(ZRH_LON, ZRH_LAT, 1.0),
        vec!["near".to_string()],
        "a 1 m radius must find the 0.4 m neighbour"
    );
    // Tighter than the neighbour: nothing.
    assert!(
        env.within(ZRH_LON, ZRH_LAT, 0.2).is_empty(),
        "a 0.2 m radius must exclude a point 0.4 m away"
    );
    // Mid-range.
    assert_eq!(
        env.within(ZRH_LON, ZRH_LAT, 100.0),
        vec!["near".to_string()]
    );
    assert_eq!(
        env.within(ZRH_LON, ZRH_LAT, 20_000.0),
        vec!["near".to_string()]
    );
    // Continental: both.
    let global = env.within(ZRH_LON, ZRH_LAT, 400_000.0);
    assert_eq!(global, vec!["far".to_string(), "near".to_string()]);
    Ok(())
}

/// Cell boundaries must not drop rows. A point just across a geohash cell edge is
/// exactly where a fixed 3x3 neighbourhood at a mismatched precision fails.
#[tokio::test]
async fn cell_boundary_neighbours_are_complete() -> Result<()> {
    let env = Env::new().await?;

    // Ring of 16 points at 30 m in every direction: some inevitably land in a
    // different cell from the centre at several precisions.
    let mut expected = Vec::new();
    for i in 0..16 {
        let bearing = (i as f64) * std::f64::consts::TAU / 16.0;
        let (lon, lat) = offset(ZRH_LON, ZRH_LAT, 30.0 * bearing.sin(), 30.0 * bearing.cos());
        let id = format!("p{:02}", i);
        env.storage
            .nodes()
            .create(env.scope(), place(&id, lon, lat), relaxed_create())
            .await?;
        expected.push(id);
    }
    expected.sort();

    // 35 m must find all 16 — no false negatives at any cell edge.
    assert_eq!(env.within(ZRH_LON, ZRH_LAT, 35.0), expected);
    Ok(())
}

// ---------------------------------------------------------------------------
// KNN
// ---------------------------------------------------------------------------

/// KNN must return the *nearest* k, not merely k results.
///
/// The old stopping rule was `candidates.len() >= k`, which is unsound: a geohash
/// cell is a rectangle and the query point sits somewhere inside it, so a node one
/// ring further out can be nearer than one already collected.
#[tokio::test]
async fn knn_returns_the_actual_nearest() -> Result<()> {
    let env = Env::new().await?;

    // Points at 10, 20, 40, 80, 160, 320 m due east.
    let distances = [10.0, 20.0, 40.0, 80.0, 160.0, 320.0];
    for (i, d) in distances.iter().enumerate() {
        let (lon, lat) = offset(ZRH_LON, ZRH_LAT, *d, 0.0);
        env.storage
            .nodes()
            .create(
                env.scope(),
                place(&format!("d{}", i), lon, lat),
                relaxed_create(),
            )
            .await?;
    }

    let results = env.spatial().find_nearest(
        TENANT,
        REPO,
        BRANCH,
        WS,
        PROP,
        ZRH_LON,
        ZRH_LAT,
        3,
        &max_rev(),
        raisin_rocksdb::spatial::INDEX_PRECISIONS,
        &SpatialPreFilter::default(),
    )?;

    let ids: Vec<&str> = results.iter().map(|r| r.node_id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["d0", "d1", "d2"],
        "KNN must return the three nearest in ascending distance order"
    );
    for w in results.windows(2) {
        assert!(w[0].distance_meters <= w[1].distance_meters);
    }
    Ok(())
}

#[tokio::test]
async fn knn_excludes_deleted_nodes() -> Result<()> {
    let env = Env::new().await?;

    for (i, d) in [10.0_f64, 20.0, 40.0].iter().enumerate() {
        let (lon, lat) = offset(ZRH_LON, ZRH_LAT, *d, 0.0);
        env.storage
            .nodes()
            .create(
                env.scope(),
                place(&format!("d{}", i), lon, lat),
                relaxed_create(),
            )
            .await?;
    }
    env.storage
        .nodes()
        .delete(env.scope(), "d0", DeleteNodeOptions::default())
        .await?;

    let results = env.spatial().find_nearest(
        TENANT,
        REPO,
        BRANCH,
        WS,
        PROP,
        ZRH_LON,
        ZRH_LAT,
        3,
        &max_rev(),
        raisin_rocksdb::spatial::INDEX_PRECISIONS,
        &SpatialPreFilter::default(),
    )?;
    let ids: Vec<&str> = results.iter().map(|r| r.node_id.as_str()).collect();
    assert_eq!(ids, vec!["d1", "d2"]);
    Ok(())
}

#[tokio::test]
async fn knn_with_k_zero_is_empty() -> Result<()> {
    let env = Env::new().await?;
    env.storage
        .nodes()
        .create(env.scope(), place("hb", ZRH_LON, ZRH_LAT), relaxed_create())
        .await?;
    let results = env.spatial().find_nearest(
        TENANT,
        REPO,
        BRANCH,
        WS,
        PROP,
        ZRH_LON,
        ZRH_LAT,
        0,
        &max_rev(),
        raisin_rocksdb::spatial::INDEX_PRECISIONS,
        &SpatialPreFilter::default(),
    )?;
    assert!(results.is_empty());
    Ok(())
}

// ---------------------------------------------------------------------------
// Value record: byte stability and the legacy fallback
// ---------------------------------------------------------------------------

/// The reindex job must be a pure no-op on re-run, which requires the value
/// encoding to be byte-stable. `format!("{}", f64)` is not, which is why the value
/// went from an ad-hoc comma string to a MessagePack record with IEEE-bit f64s.
#[test]
fn entry_encoding_is_byte_stable() {
    let entry = SpatialEntry {
        v: SPATIAL_ENTRY_VERSION,
        lon: 8.540217183823,
        lat: 47.378177777777,
        bbox: [8.5, 47.3, 8.6, 47.4],
        z: Some((408.0, 412.5)),
        srid: 2056,
        gtype: SpatialGeometryKind::Polygon,
        bucket: Some("L2".to_string()),
        policy_hash: 0xdead_beef_cafe_f00d,
    };

    let a = entry.encode();
    let b = entry.encode();
    assert_eq!(a, b, "the same entry must encode to identical bytes");

    let decoded = SpatialEntry::decode(&a).expect("must decode");
    assert_eq!(decoded, entry, "encode/decode must round-trip exactly");
}

/// An upgraded server must keep answering against a not-yet-reindexed index.
/// Without this fallback, upgrading would silently empty every spatial query —
/// exactly the failure mode this pass exists to remove.
#[test]
fn legacy_comma_value_still_parses() {
    let legacy = b"8.5402,47.3782";
    let entry = SpatialEntry::decode(legacy).expect("legacy value must parse");
    assert!(entry.is_legacy());
    assert!((entry.lon - 8.5402).abs() < 1e-9);
    assert!((entry.lat - 47.3782).abs() < 1e-9);
    // No bbox selectivity, but the centroid is intact and the bbox collapses to it,
    // so an envelope pre-filter stays correct for point data.
    assert_eq!(entry.bbox, [8.5402, 47.3782, 8.5402, 47.3782]);
    // And it can never be rejected by a bucket filter it knows nothing about.
    assert!(entry.matches_bucket(Some("L2")));
}

#[test]
fn tombstone_and_garbage_values_decode_to_none() {
    assert!(SpatialEntry::decode(b"T").is_none());
    assert!(SpatialEntry::decode(b"").is_none());
    assert!(SpatialEntry::decode(b"not a value").is_none());
}

// ---------------------------------------------------------------------------
// Derived tombstoning: the O(1) unindex
// ---------------------------------------------------------------------------

/// Tombstoning must be derived from the OLD geometry rather than discovered by a
/// prefix scan of the whole workspace. Asserted behaviourally: after tombstoning one
/// node's entries, every OTHER node in the same workspace still matches — a
/// scan-based implementation that mis-identified keys (the old one split on `\0` and
/// trusted `parts[6]`, which the descending HLC's embedded nulls can break) would
/// take neighbours down with it.
#[tokio::test]
async fn derived_tombstoning_does_not_disturb_neighbours() -> Result<()> {
    let env = Env::new().await?;

    for i in 0..5 {
        let (lon, lat) = offset(ZRH_LON, ZRH_LAT, (i as f64) * 15.0, 0.0);
        env.storage
            .nodes()
            .create(
                env.scope(),
                place(&format!("n{}", i), lon, lat),
                relaxed_create(),
            )
            .await?;
    }

    let before = env.within(ZRH_LON, ZRH_LAT, 200.0);
    assert_eq!(before.len(), 5);

    // Tombstone n2's entries directly, at a newer revision.
    let (lon2, lat2) = offset(ZRH_LON, ZRH_LAT, 30.0, 0.0);
    env.spatial().unindex_geometry(
        TENANT,
        REPO,
        BRANCH,
        WS,
        "n2",
        PROP,
        &GeoJson::point(lon2, lat2),
        &max_rev(),
        &SpatialPolicy::default(),
    )?;

    let after = env.within(ZRH_LON, ZRH_LAT, 200.0);
    assert_eq!(
        after,
        vec![
            "n0".to_string(),
            "n1".to_string(),
            "n3".to_string(),
            "n4".to_string()
        ],
        "only the tombstoned node must disappear"
    );
    Ok(())
}
