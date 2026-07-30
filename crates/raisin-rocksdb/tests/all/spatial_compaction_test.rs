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

//! The spatial index compaction filter, end to end.
//!
//! `src/spatial/compaction_tests.rs` pins the decision logic against synthetic
//! keys. This pins the thing that actually matters in production: a tracked
//! object updated hundreds of times accumulates entries in its COARSE cell
//! prefix without bound (the revision is part of the key, so nothing collapses),
//! and after a compaction that prefix must shrink dramatically **while the
//! spatial query still returns the correct CURRENT position and no stale one**.
//!
//! That last assertion is the point. Pruning that breaks correctness is worse
//! than no pruning: this is the same bug class — a stale spatial hit surviving
//! an update — that the whole spatial pass exists to eliminate.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::{GeoJson, PropertyValue};
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use raisin_rocksdb::repositories::spatial_index::SpatialIndexRepository;
use raisin_rocksdb::spatial::SpatialCompactionConfig;
use raisin_rocksdb::{cf, fractional_index, keys, RocksDBConfig, RocksDBStorage};
use raisin_storage::spatial::SpatialPreFilter;
use raisin_storage::{
    BranchRepository, CreateNodeOptions, NodeRepository, RegistryRepository,
    RepositoryManagementRepository, Storage, StorageScope, UpdateNodeOptions,
};
use tempfile::TempDir;

const TENANT: &str = "spatial-compaction";
const REPO: &str = "repo";
const BRANCH: &str = "main";
const WS: &str = "fleet";
const PROP: &str = "position";
const NODE_ID: &str = "veh1";

/// Number of position updates. Large enough that the accumulation is
/// unmistakable, small enough to keep the test under a few seconds.
const UPDATES: usize = 500;

/// Zurich Hauptbahnhof.
const ZRH_LON: f64 = 8.5402;
const ZRH_LAT: f64 = 47.3782;

/// Far-future read snapshot, so every write is visible.
fn max_rev() -> HLC {
    HLC::new(u64::MAX / 2, 0)
}

struct Env {
    _dir: TempDir,
    storage: Arc<RocksDBStorage>,
}

impl Env {
    async fn new(spatial_compaction: SpatialCompactionConfig) -> Result<Self> {
        let temp_dir = TempDir::new().unwrap();
        let config = RocksDBConfig::default()
            .with_path(temp_dir.path())
            .with_spatial_compaction(spatial_compaction);
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

    fn scope(&self) -> StorageScope<'_> {
        StorageScope::new(TENANT, REPO, BRANCH, WS)
    }

    fn within(&self, lon: f64, lat: f64, radius: f64) -> Vec<String> {
        let mut ids: Vec<String> = SpatialIndexRepository::new(self.storage.db().clone())
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

    /// Entries physically present under one geohash cell prefix.
    fn entries_in_cell(&self, cell: &str) -> usize {
        let db = self.storage.db();
        let handle = db.cf_handle(cf::SPATIAL_INDEX).expect("spatial CF exists");
        let prefix = keys::spatial_index_geohash_prefix(TENANT, REPO, BRANCH, WS, PROP, cell);
        db.prefix_iterator_cf(handle, &prefix)
            .filter_map(|item| item.ok())
            .take_while(|(key, _)| key.starts_with(&prefix))
            .count()
    }

    /// Every entry in the whole spatial CF, for the total-footprint number.
    fn total_entries(&self) -> usize {
        let db = self.storage.db();
        let handle = db.cf_handle(cf::SPATIAL_INDEX).expect("spatial CF exists");
        db.iterator_cf(handle, rocksdb::IteratorMode::Start).count()
    }

    /// Flush the memtable and compact the spatial CF over its whole range, so
    /// the filter actually runs. Without the flush the newest entries are still
    /// in memory and compaction never sees them.
    fn compact_spatial(&self) {
        let db = self.storage.db();
        let handle = db.cf_handle(cf::SPATIAL_INDEX).expect("spatial CF exists");
        db.flush_cf(handle).expect("flush must succeed");
        db.compact_range_cf(handle, None::<&[u8]>, None::<&[u8]>);
    }

    /// Median-ish query cost: the total time of `n` radius queries.
    fn query_time(&self, lon: f64, lat: f64, radius: f64, n: u32) -> Duration {
        let start = Instant::now();
        for _ in 0..n {
            let _ = self.within(lon, lat, radius);
        }
        start.elapsed()
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

fn vehicle(version: i32, lon: f64, lat: f64) -> Node {
    let mut properties = HashMap::new();
    properties.insert(
        PROP.to_string(),
        PropertyValue::Geometry(GeoJson::point(lon, lat)),
    );
    Node {
        id: NODE_ID.to_string(),
        name: NODE_ID.to_string(),
        path: format!("/{}", NODE_ID),
        node_type: "test:Vehicle".to_string(),
        archetype: None,
        properties,
        children: Vec::new(),
        order_key: fractional_index::first(),
        has_children: Some(false),
        parent: Some("/".to_string()),
        version,
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

/// Offset a point by `east_m` / `north_m` metres.
fn offset(lon: f64, lat: f64, east_m: f64, north_m: f64) -> (f64, f64) {
    let per_deg_lat = 111_195.0_f64;
    let per_deg_lon = per_deg_lat * lat.to_radians().cos();
    (lon + east_m / per_deg_lon, lat + north_m / per_deg_lat)
}

/// Drive one vehicle `UPDATES` times, drifting `step_m` east each time.
///
/// Returns its final position.
async fn drive(env: &Env, step_m: f64) -> Result<(f64, f64)> {
    env.storage
        .nodes()
        .create(env.scope(), vehicle(1, ZRH_LON, ZRH_LAT), relaxed_create())
        .await?;

    let mut pos = (ZRH_LON, ZRH_LAT);
    for i in 0..UPDATES {
        pos = offset(ZRH_LON, ZRH_LAT, step_m * (i + 1) as f64, 0.0);
        env.storage
            .nodes()
            .update(
                env.scope(),
                vehicle(i as i32 + 2, pos.0, pos.1),
                UpdateNodeOptions::default(),
            )
            .await?;
    }
    Ok(pos)
}

/// The coarse cell the vehicle never leaves. Precision 6 is ~1.2 km, and the
/// drive below covers ~10 m, so every update lands in this one prefix — the
/// distribution OPEN-ITEMS §2.99 calls counter-intuitive, and the one where read
/// cost actually concentrates.
fn coarse_cell() -> String {
    raisin_rocksdb::spatial::encode_point(ZRH_LON, ZRH_LAT, 6).expect("valid coordinates")
}

/// THE test. Numbers are printed rather than only asserted, because "how much
/// does it actually reclaim" is the question this work exists to answer.
#[tokio::test]
async fn compaction_prunes_superseded_entries_without_breaking_the_query() -> Result<()> {
    let env = Env::new(SpatialCompactionConfig::newest_only()).await?;
    let final_pos = drive(&env, 0.02).await?;
    let cell = coarse_cell();

    let cell_before = env.entries_in_cell(&cell);
    let total_before = env.total_entries();
    let t_before = env.query_time(final_pos.0, final_pos.1, 1_000.0, 20);

    // Precondition: the accumulation this filter exists to fix is real. Two
    // entries per update in the coarse cell (the new live entry and the
    // tombstone of the superseded one).
    assert!(
        cell_before > UPDATES,
        "precondition: the coarse cell must have accumulated more than one entry \
         per update, got {} for {} updates",
        cell_before,
        UPDATES
    );

    env.compact_spatial();

    let cell_after = env.entries_in_cell(&cell);
    let total_after = env.total_entries();
    let t_after = env.query_time(final_pos.0, final_pos.1, 1_000.0, 20);

    println!(
        "spatial compaction over {UPDATES} updates:\n  \
         coarse cell {cell}: {cell_before} -> {cell_after} entries ({:.1}x)\n  \
         whole CF:            {total_before} -> {total_after} entries ({:.1}x)\n  \
         20x radius query:    {:?} -> {:?} ({:.1}x)",
        cell_before as f64 / cell_after.max(1) as f64,
        total_before as f64 / total_after.max(1) as f64,
        t_before,
        t_after,
        t_before.as_secs_f64() / t_after.as_secs_f64().max(f64::MIN_POSITIVE),
    );

    // --- the reclamation claim -------------------------------------------
    assert!(
        cell_after * 10 < cell_before,
        "the coarse cell must shrink by at least 10x: {} -> {}",
        cell_before,
        cell_after
    );
    assert!(
        total_after < total_before / 2,
        "the whole CF must at least halve: {} -> {}",
        total_before,
        total_after
    );

    // --- the correctness claim, which matters more ------------------------
    assert_eq!(
        env.within(final_pos.0, final_pos.1, 5.0),
        vec![NODE_ID.to_string()],
        "after pruning, the vehicle must still be found at its CURRENT position"
    );
    // It drifted ~10 m east of the start, so a 2 m query at the start point must
    // find nothing. This is the stale-position assertion: pruning must not
    // unshadow an older revision.
    assert!(
        env.within(ZRH_LON, ZRH_LAT, 2.0).is_empty(),
        "a superseded position must not resurrect: the start point must be empty"
    );
    // And exactly once across a query covering the whole drive.
    assert_eq!(
        env.within(ZRH_LON, ZRH_LAT, 1_000.0),
        vec![NODE_ID.to_string()],
        "a wide query must return the vehicle exactly once"
    );
    Ok(())
}

/// The escape hatch has to actually work: with the filter disabled the CF is
/// byte-for-byte as before, so an operator who hits a problem can turn it off.
#[tokio::test]
async fn disabling_the_filter_prunes_nothing() -> Result<()> {
    let env = Env::new(SpatialCompactionConfig::disabled()).await?;
    let final_pos = drive(&env, 0.02).await?;
    let cell = coarse_cell();

    let before = env.entries_in_cell(&cell);
    env.compact_spatial();
    let after = env.entries_in_cell(&cell);

    assert_eq!(
        before, after,
        "a disabled filter must remove nothing (got {} -> {})",
        before, after
    );
    assert_eq!(
        env.within(final_pos.0, final_pos.1, 5.0),
        vec![NODE_ID.to_string()]
    );
    Ok(())
}

/// The shipped default retains a short history rather than collapsing to one
/// entry, so a near-HEAD `__revision` spatial read still has something to
/// resolve against — while still bounding the prefix at a constant.
#[tokio::test]
async fn the_default_config_retains_a_bounded_history() -> Result<()> {
    let default = SpatialCompactionConfig::default();
    let env = Env::new(default.clone()).await?;
    let final_pos = drive(&env, 0.02).await?;
    let cell = coarse_cell();

    let before = env.entries_in_cell(&cell);
    env.compact_spatial();
    let after = env.entries_in_cell(&cell);

    println!(
        "default config (keep_revisions={}, retention_secs={}): {} -> {} entries",
        default.keep_revisions, default.retention_secs, before, after
    );

    assert!(
        after > 1,
        "the default must retain more than the newest entry, got {}",
        after
    );
    // One node, so the bound is keep_revisions (plus the newest, which is always
    // kept) — a constant, not a function of UPDATES.
    assert!(
        after <= default.keep_revisions + 1,
        "the default must bound the prefix at keep_revisions ({}), got {}",
        default.keep_revisions,
        after
    );
    assert_eq!(
        env.within(final_pos.0, final_pos.1, 5.0),
        vec![NODE_ID.to_string()],
        "retention must not change what a query at HEAD returns"
    );
    assert!(env.within(ZRH_LON, ZRH_LAT, 2.0).is_empty());
    Ok(())
}
