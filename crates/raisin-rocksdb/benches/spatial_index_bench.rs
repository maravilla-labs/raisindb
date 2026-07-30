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

//! Spatial index benchmarks.
//!
//! Run with `cargo bench -p raisin-rocksdb --bench spatial_index_bench`. Benches are
//! opt-in and are not built by `cargo test`, so this extra binary costs nothing in the
//! normal cycle.
//!
//! # What is measured, and why here rather than in a unit test
//!
//! 1. **Write cost with the legacy vs. the new precision set.** The set went from
//!    `[4,5,6,7,8]` to `[2,4,6,7,8,9,10,11]` — eight keys per geometry per revision
//!    instead of five. That is 1.6x on the *spatial-index key count* and much less on
//!    total node-write cost, since `SPATIAL_INDEX` is one of roughly eight column
//!    families a node write touches. This **measures** it rather than asserting it:
//!    the write-amplification budget was approved at about 2x and the honest thing is
//!    to report what was actually spent.
//! 2. **`find_within_radius` across six radii from 0.5 m to 50 km.** The target is
//!    O(cells x log N + candidates) with no dependence on N at any radius. The old
//!    code silently returned zero rows outside roughly 4.8 m - 39 km, so this sweep is
//!    the guard for the whole multi-scale design.
//! 3. **KNN k=10.**
//! 4. **UPDATE throughput at three values of N.** `unindex_geometry_to_batch` used to
//!    prefix-iterate the entire workspace's geometry range on *every* update to
//!    discover a node's old cells — O(all geometries in the workspace) per single-node
//!    write. Cells are now derived from the old geometry, so per-move cost must be
//!    FLAT across N. A rising curve means a scan crept back in. This is very likely a
//!    larger production win than the precision redesign; these numbers are what
//!    justify saying so.
//! 5. **`cover = EXTENT` on large polygons.** The one setting that can push write cost
//!    past 2x, which is why `MAX_COVER_CELLS` is capped and `Centroid` is the default.
//!    Quantified so the DDL docs can warn concretely.

use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

use raisin_hlc::HLC;
use raisin_models::nodes::properties::{
    GeoJson, SpatialCoverMode, SpatialPolicy, INDEX_PRECISIONS_DEFAULT,
};
use raisin_rocksdb::indexing::{IndexCtx, SpatialIndexTargets};
use raisin_rocksdb::repositories::spatial_index::SpatialIndexRepository;
use raisin_rocksdb::{RocksDBConfig, RocksDBStorage};
use raisin_storage::spatial::SpatialPreFilter;
use rocksdb::WriteBatch;
use tempfile::TempDir;

const TENANT: &str = "bench";
const REPO: &str = "repo";
const BRANCH: &str = "main";
const WS: &str = "places";
const PROP: &str = "location";

/// The old precision set, kept so the amplification comparison is against the real
/// previous behaviour rather than a guess.
const LEGACY_PRECISIONS: &[usize] = &[4, 5, 6, 7, 8];

const N: usize = 100_000;

fn policy_with(precisions: &[usize], cover: SpatialCoverMode) -> SpatialPolicy {
    let mut sorted = precisions.to_vec();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    SpatialPolicy {
        precisions: sorted,
        cover,
        ..SpatialPolicy::default()
    }
}

fn open() -> (TempDir, Arc<RocksDBStorage>) {
    let dir = TempDir::new().expect("temp dir");
    let mut config = RocksDBConfig::default();
    config.path = dir.path().to_path_buf();
    let storage = Arc::new(RocksDBStorage::with_config(config).expect("open storage"));
    (dir, storage)
}

/// Pseudo-random points in a ~40 km box around Zurich. A fixed LCG rather than a
/// dependency, so runs are reproducible.
struct Points {
    state: u64,
}

impl Points {
    fn new() -> Self {
        Self {
            state: 0x2545_F491_4F6C_DD1D,
        }
    }
    fn next_f64(&mut self) -> f64 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.state >> 11) as f64) / ((1u64 << 53) as f64)
    }
    fn next_point(&mut self) -> (f64, f64) {
        let a = self.next_f64();
        let b = self.next_f64();
        (8.34 + a * 0.4, 47.24 + b * 0.28)
    }
}

/// Write `count` point geometries under `policy`, returning elapsed time and the
/// number of index keys produced.
fn seed(
    storage: &RocksDBStorage,
    repo: &SpatialIndexRepository,
    policy: &SpatialPolicy,
    count: usize,
) -> (std::time::Duration, usize) {
    let mut points = Points::new();
    let started = Instant::now();
    let mut keys = 0usize;

    // Chunked, so the measurement reflects index-write cost rather than per-write WAL
    // fsync behaviour.
    const CHUNK: usize = 1000;
    let mut batch = WriteBatch::default();
    for i in 0..count {
        let (lon, lat) = points.next_point();
        let geometry = GeoJson::point(lon, lat);
        let revision = HLC::new(1_767_225_600_000 + i as u64, 0);
        repo.index_geometry_to_batch(
            &mut batch,
            TENANT,
            REPO,
            BRANCH,
            WS,
            &format!("n{}", i),
            PROP,
            &geometry,
            &revision,
            policy,
            None,
        )
        .expect("index");
        keys += policy.precisions.len();

        if (i + 1) % CHUNK == 0 {
            storage
                .db()
                .write(std::mem::take(&mut batch))
                .expect("write");
        }
    }
    storage.db().write(batch).expect("write");
    (started.elapsed(), keys)
}

fn bench_write_amplification() {
    println!("\n=== 1. Write cost: legacy [4,5,6,7,8] vs default [2,4,6,7,8,9,10,11] ===");

    for (label, precisions) in [
        ("legacy (5 precisions)", LEGACY_PRECISIONS),
        ("default (8 precisions)", INDEX_PRECISIONS_DEFAULT),
    ] {
        let (_dir, storage) = open();
        let repo = SpatialIndexRepository::new(storage.db().clone());
        let policy = policy_with(precisions, SpatialCoverMode::Centroid);
        let (elapsed, keys) = seed(&storage, &repo, &policy, N);
        println!(
            "  {label:<24} {N} geometries -> {keys:>9} keys in {elapsed:>10.2?}  \
             ({:>9.0} geom/s, {:.1} keys/geom)",
            N as f64 / elapsed.as_secs_f64(),
            keys as f64 / N as f64,
        );
    }
    println!(
        "  key-count ratio: {:.2}x  (budget ~2x; SPATIAL_INDEX is one of ~8 CFs a node write \
         touches, so total node-write amplification is materially lower)",
        INDEX_PRECISIONS_DEFAULT.len() as f64 / LEGACY_PRECISIONS.len() as f64
    );
}

fn bench_radius_sweep() {
    println!("\n=== 2. find_within_radius across scales (N = {N}) ===");

    let (_dir, storage) = open();
    let repo = SpatialIndexRepository::new(storage.db().clone());
    let policy = policy_with(INDEX_PRECISIONS_DEFAULT, SpatialCoverMode::Centroid);
    seed(&storage, &repo, &policy, N);

    let max_rev = HLC::new(u64::MAX / 2, 0);
    let (clon, clat) = (8.5402, 47.3782);

    for radius in [0.5_f64, 5.0, 50.0, 500.0, 5_000.0, 50_000.0] {
        const ITERS: usize = 20;
        let mut hits = 0usize;
        let started = Instant::now();
        for _ in 0..ITERS {
            let results = repo
                .find_within_radius(
                    TENANT,
                    REPO,
                    BRANCH,
                    WS,
                    PROP,
                    clon,
                    clat,
                    radius,
                    &max_rev,
                    10_000,
                    &policy.precisions,
                    &SpatialPreFilter::default(),
                )
                .expect("radius query must not error at any scale");
            hits = results.len();
            black_box(&results);
        }
        let per = started.elapsed() / ITERS as u32;
        println!("  radius {radius:>10.1} m -> {hits:>6} hits, {per:>10.2?} per query");
    }
}

fn bench_knn() {
    println!("\n=== 3. KNN k=10 (N = {N}) ===");

    let (_dir, storage) = open();
    let repo = SpatialIndexRepository::new(storage.db().clone());
    let policy = policy_with(INDEX_PRECISIONS_DEFAULT, SpatialCoverMode::Centroid);
    seed(&storage, &repo, &policy, N);

    let max_rev = HLC::new(u64::MAX / 2, 0);
    const ITERS: usize = 20;
    let started = Instant::now();
    let mut found = 0usize;
    for _ in 0..ITERS {
        let results = repo
            .find_nearest(
                TENANT,
                REPO,
                BRANCH,
                WS,
                PROP,
                8.5402,
                47.3782,
                10,
                &max_rev,
                &policy.precisions,
                &SpatialPreFilter::default(),
            )
            .expect("knn");
        found = results.len();
        black_box(&results);
    }
    println!(
        "  k=10 -> {found} results, {:>10.2?} per query",
        started.elapsed() / ITERS as u32
    );
}

fn bench_update_cost() {
    println!("\n=== 4. UPDATE cost: derived tombstoning must be independent of N ===");
    println!("  (the old unindex prefix-iterated the ENTIRE workspace geometry range per update)");

    for n in [1_000usize, 10_000, 100_000] {
        let (_dir, storage) = open();
        let repo = SpatialIndexRepository::new(storage.db().clone());
        let policy = policy_with(INDEX_PRECISIONS_DEFAULT, SpatialCoverMode::Centroid);
        seed(&storage, &repo, &policy, n);

        const MOVES: usize = 500;
        let mut points = Points::new();
        let started = Instant::now();
        for i in 0..MOVES {
            let old = GeoJson::point(8.5402, 47.3782);
            let (lon, lat) = points.next_point();
            let new = GeoJson::point(lon, lat);
            let revision = HLC::new(1_800_000_000_000 + i as u64, 0);
            let mut batch = WriteBatch::default();
            repo.unindex_geometry_to_batch(
                &mut batch,
                TENANT,
                REPO,
                BRANCH,
                WS,
                &format!("n{}", i),
                PROP,
                &old,
                &revision,
                &policy,
            )
            .expect("unindex");
            repo.index_geometry_to_batch(
                &mut batch,
                TENANT,
                REPO,
                BRANCH,
                WS,
                &format!("n{}", i),
                PROP,
                &new,
                &revision,
                &policy,
                None,
            )
            .expect("index");
            storage.db().write(batch).expect("write");
        }
        let elapsed = started.elapsed();
        println!(
            "  N = {n:>7}: {MOVES} moves in {elapsed:>10.2?}  ({:>10.2?} per move)",
            elapsed / MOVES as u32
        );
    }
    println!("  per-move cost should be FLAT across N; a rising curve means a scan crept back in");
}

fn bench_extent_cover() {
    println!("\n=== 5. cover = EXTENT on polygons: the one setting that can exceed 2x ===");

    // A polygon spanning ~2 km, which at fine precisions wants many cells.
    let polygon = GeoJson::Polygon {
        coordinates: vec![vec![
            [8.530, 47.370].into(),
            [8.555, 47.370].into(),
            [8.555, 47.385].into(),
            [8.530, 47.385].into(),
            [8.530, 47.370].into(),
        ]],
        srid: None,
    };

    for (label, cover) in [
        ("Centroid (default)", SpatialCoverMode::Centroid),
        ("Extent (opt-in)", SpatialCoverMode::Extent),
    ] {
        let (_dir, storage) = open();
        let policy = policy_with(INDEX_PRECISIONS_DEFAULT, cover);

        let ctx = IndexCtx::new(TENANT, REPO, BRANCH, WS);
        let targets = SpatialIndexTargets::from_db(storage.db().as_ref()).expect("cf");
        let mut batch = WriteBatch::default();
        raisin_rocksdb::indexing::write_spatial_property(
            &mut batch,
            &targets,
            &ctx,
            "poly",
            PROP,
            &polygon,
            &HLC::new(1_767_225_600_000, 0),
            &policy,
            None,
        )
        .expect("index");
        let keys = batch.len();
        storage.db().write(batch).expect("write");

        println!("  {label:<20} -> {keys:>5} keys for one 2 km polygon");
    }
    println!(
        "  MAX_COVER_CELLS caps Extent per precision, so the worst case is bounded; Centroid \
         remains the default, and Envelope-mode bbox pushdown keeps results CORRECT under it \
         either way"
    );
}

fn main() {
    println!("RaisinDB spatial index benchmarks");
    println!("(single-threaded, temp RocksDB per case, N = {N} geometries unless stated)");

    bench_write_amplification();
    bench_radius_sweep();
    bench_knn();
    bench_update_cost();
    bench_extent_cover();
}
