//! Tests for spatial indexing utilities.

use super::*;
use raisin_models::nodes::properties::GeoJson;

#[test]
fn test_encode_decode_roundtrip() {
    let lon = -122.4194; // San Francisco
    let lat = 37.7749;

    let hash = encode_point(lon, lat, 8).expect("should encode");
    let (decoded_lon, decoded_lat) = decode_geohash(&hash).unwrap();

    assert!((decoded_lon - lon).abs() < 0.001);
    assert!((decoded_lat - lat).abs() < 0.001);
}

#[test]
fn test_neighbors() {
    let hash = encode_point(-122.4194, 37.7749, 6).expect("should encode");
    let neighbor_hashes = neighbors(&hash);

    assert_eq!(neighbor_hashes.len(), 8);

    for n in &neighbor_hashes {
        assert_eq!(n.len(), hash.len());
    }
}

#[test]
fn test_center_and_neighbors() {
    let hash = "9q8yyk";
    let cells = center_and_neighbors(hash);

    assert_eq!(cells.len(), 9);
    assert_eq!(cells[0], hash);
}

#[test]
fn test_multi_precision_geohashes() {
    let hashes = multi_precision_geohashes(-122.4194, 37.7749);

    // The default precision set is `[2, 4, 6, 7, 8, 9, 10, 11]` (finest first in
    // the policy, ascending here because `INDEX_PRECISIONS` is declared ascending).
    assert_eq!(hashes.len(), INDEX_PRECISIONS.len());
    assert_eq!(hashes[0].0, 2);
    assert_eq!(hashes[hashes.len() - 1].0, 11);
    assert!(hashes[0].1.len() < hashes[hashes.len() - 1].1.len());
}

#[test]
fn test_geometry_centroid_point() {
    let point = GeoJson::point(-122.4194, 37.7749);

    let (lon, lat) = geometry_centroid(&point).unwrap();
    assert!((lon - (-122.4194)).abs() < 0.0001);
    assert!((lat - 37.7749).abs() < 0.0001);
}

#[test]
fn test_geometry_centroid_polygon() {
    let polygon = GeoJson::Polygon {
        coordinates: vec![vec![
            [0.0, 0.0].into(),
            [1.0, 0.0].into(),
            [1.0, 1.0].into(),
            [0.0, 1.0].into(),
            [0.0, 0.0].into(),
        ]],
        srid: None,
    };

    let (lon, lat) = geometry_centroid(&polygon).unwrap();
    assert!((lon - 0.4).abs() < 0.1);
    assert!((lat - 0.4).abs() < 0.1);
}

/// Precision selection is now restricted to precisions the index was ACTUALLY
/// built at, which is the whole point: the old unrestricted `precision_for_radius`
/// walked 12 -> 1 and happily returned a precision nothing was indexed at, and
/// because the geohash prefix is `\0`-terminated (matching exactly one precision)
/// the query then returned zero rows.
#[test]
fn test_precision_for_radius_in_respects_the_indexed_set() {
    // 100 m against the default set: precision 7 (153 m cells) is the finest that
    // still covers it.
    assert_eq!(precision_for_radius_in(100.0, INDEX_PRECISIONS), Some(7));
    // 1 km -> precision 6 (1.2 km cells).
    assert_eq!(precision_for_radius_in(1000.0, INDEX_PRECISIONS), Some(6));
    // 10 km -> precision 4 (39 km cells), because 5 is not in the default set.
    assert_eq!(precision_for_radius_in(10_000.0, INDEX_PRECISIONS), Some(4));

    // A radius larger than the coarsest indexed cell has no covering precision;
    // the caller must ring-expand rather than silently pick something too fine.
    assert_eq!(precision_for_radius_in(5_000_000.0, &[8]), None);
}

#[test]
fn test_cells_for_radius() {
    let cells = cells_for_radius(-122.4194, 37.7749, 500.0);

    assert_eq!(cells.len(), 9);

    let first_len = cells[0].len();
    for cell in &cells {
        assert_eq!(cell.len(), first_len);
    }
}

#[test]
fn test_geohashes_for_geometry() {
    let point = GeoJson::point(-122.4194, 37.7749);

    let hashes = geohashes_for_geometry(&point);
    // One cell per configured precision under the default `Centroid` cover.
    assert_eq!(hashes.len(), INDEX_PRECISIONS.len());
}

#[test]
fn test_encode_point_nan_returns_none() {
    assert!(encode_point(f64::NAN, 37.7749, 8).is_none());
    assert!(encode_point(-122.4194, f64::NAN, 8).is_none());
}

#[test]
fn test_encode_point_infinity_returns_none() {
    assert!(encode_point(f64::INFINITY, 37.7749, 8).is_none());
    assert!(encode_point(-122.4194, f64::NEG_INFINITY, 8).is_none());
}

#[test]
fn test_encode_point_out_of_bounds_returns_none() {
    assert!(encode_point(200.0, 37.7749, 8).is_none());
    assert!(encode_point(-181.0, 37.7749, 8).is_none());
    assert!(encode_point(-122.4194, 91.0, 8).is_none());
    assert!(encode_point(-122.4194, -91.0, 8).is_none());
}

#[test]
fn test_encode_point_boundary_values_succeed() {
    assert!(encode_point(180.0, 90.0, 8).is_some());
    assert!(encode_point(-180.0, -90.0, 8).is_some());
    assert!(encode_point(0.0, 0.0, 8).is_some());
}

#[test]
fn test_neighbors_empty_hash_returns_empty() {
    assert!(neighbors("").is_empty());
}

#[test]
fn test_cells_for_radius_invalid_coords() {
    assert!(cells_for_radius(f64::NAN, 37.7749, 500.0).is_empty());
}

#[test]
fn test_multi_precision_geohashes_invalid_coords() {
    assert!(multi_precision_geohashes(f64::NAN, 37.7749).is_empty());
}

// ---------------------------------------------------------------------------
// The cover guarantee (spatial/plan.rs)
// ---------------------------------------------------------------------------

/// Brute-force check that a plan's cell set really contains every point within the
/// radius. This is the property the whole design rests on: **the precision set is a
/// performance knob and must never change which rows a query returns.**
fn assert_plan_covers(lon: f64, lat: f64, radius_m: f64, precisions: &[usize]) {
    let plan = plan_radius_scan(lon, lat, radius_m, precisions);
    let cells: std::collections::HashSet<String> = match &plan {
        SpatialScanPlan::Covering { cells, .. } => cells.iter().cloned().collect(),
        SpatialScanPlan::NotCovering => panic!("expected a covering plan for {radius_m} m"),
    };
    let precision = match &plan {
        SpatialScanPlan::Covering { precision, .. } => *precision,
        SpatialScanPlan::NotCovering => unreachable!(),
    };

    // Sample the circle densely: the boundary is where a cover fails, so walk the
    // rim plus a few inner rings.
    let earth = 6_371_008.8_f64;
    for ring in [1.0_f64, 0.75, 0.5] {
        let r = radius_m * ring;
        for step in 0..72 {
            let bearing = (step as f64) * 5.0_f64.to_radians();
            let dlat = (r / earth) * bearing.cos();
            let dlon = (r / earth) * bearing.sin() / lat.to_radians().cos();
            let plon = lon + dlon.to_degrees();
            let plat = lat + dlat.to_degrees();
            let cell = encode_point(plon, plat, precision).expect("sample point must encode");
            assert!(
                cells.contains(&cell),
                "point ({plon}, {plat}) at {r} m from centre falls in cell {cell}, \
                 which the plan for radius {radius_m} m at precision {precision} does not scan"
            );
        }
    }
}

#[test]
fn test_plan_covers_every_radius_in_the_default_set() {
    // Zurich. Sub-metre through city scale — the band the old implementation
    // silently returned zero rows outside of (roughly 4.8 m to 39 km).
    for radius in [0.2, 0.5, 1.0, 5.0, 25.0, 100.0, 500.0, 2_000.0, 20_000.0] {
        assert_plan_covers(8.5417, 47.3769, radius, INDEX_PRECISIONS);
    }
}

/// The load-bearing decoupling: a sub-metre query against an index built ONLY at
/// precision 8 (38 m cells) must still return the right rows. It over-fetches
/// candidates and lets the Haversine post-filter do the work — correctness is
/// independent of configuration.
#[test]
fn test_plan_covers_sub_metre_radius_on_a_coarse_index() {
    assert_plan_covers(8.5417, 47.3769, 0.3, &[8]);
    let plan = plan_radius_scan(8.5417, 47.3769, 0.3, &[8]);
    assert!(plan.is_covering());
}

/// A radius larger than the coarsest indexed cell must ring-expand rather than
/// return a too-fine 3x3 block (which would drop rows).
#[test]
fn test_plan_ring_expands_past_the_coarsest_cell() {
    // Precision 8 cells are ~38 m x 19 m at this latitude, so 250 m needs ~14 rings.
    let plan = plan_radius_scan(8.5417, 47.3769, 250.0, &[8]);
    match plan {
        SpatialScanPlan::Covering { precision, cells } => {
            assert_eq!(precision, 8);
            assert!(
                cells.len() > 9,
                "expected ring expansion, got {} cells",
                cells.len()
            );
            assert!(cells.len() <= raisin_models::nodes::properties::MAX_SCAN_CELLS);
        }
        SpatialScanPlan::NotCovering => panic!("250 m at precision 8 should be coverable"),
    }
    assert_plan_covers(8.5417, 47.3769, 250.0, &[8]);
}

/// Ring expansion is bounded, so an index built ONLY at a fine precision genuinely
/// cannot answer a wide radius — and says so instead of guessing. The planner then
/// falls back to a scan with the predicate retained, which is slower but correct.
#[test]
fn test_plan_refuses_when_expansion_would_exceed_the_budget() {
    assert_eq!(
        plan_radius_scan(8.5417, 47.3769, 5_000.0, &[8]),
        SpatialScanPlan::NotCovering
    );
    // The same radius IS answerable once a coarser precision is in the set — which
    // is precisely why the default set spans 1250 km down to 0.15 m.
    assert!(plan_radius_scan(8.5417, 47.3769, 5_000.0, INDEX_PRECISIONS).is_covering());
}

/// Over the cell budget the plan must refuse rather than return a partial cover.
/// Partial results here would be the silent-wrong-answer bug this design removes.
#[test]
fn test_plan_refuses_rather_than_returning_a_partial_cover() {
    // 5000 km at precision 11 (0.15 m cells) is astronomically many cells.
    assert_eq!(
        plan_radius_scan(8.5417, 47.3769, 5_000_000.0, &[11]),
        SpatialScanPlan::NotCovering
    );
    // Invalid centre, empty precision set, negative radius: all refuse.
    assert_eq!(
        plan_radius_scan(f64::NAN, 47.3769, 100.0, INDEX_PRECISIONS),
        SpatialScanPlan::NotCovering
    );
    assert_eq!(
        plan_radius_scan(8.5417, 47.3769, 100.0, &[]),
        SpatialScanPlan::NotCovering
    );
    assert_eq!(
        plan_radius_scan(8.5417, 47.3769, -1.0, INDEX_PRECISIONS),
        SpatialScanPlan::NotCovering
    );
}

/// Global-scale radii work through the coarse end of the default set, which is why
/// precision 2 (1250 km cells) is in it.
#[test]
fn test_plan_handles_global_radius() {
    let plan = plan_radius_scan(8.5417, 47.3769, 800_000.0, INDEX_PRECISIONS);
    assert!(
        plan.is_covering(),
        "800 km must be coverable by the default set"
    );
}

// ---------------------------------------------------------------------------
// Ring expansion
// ---------------------------------------------------------------------------

#[test]
fn test_ring_expand_shapes() {
    let hash = encode_point(8.5417, 47.3769, 7).expect("encode");
    assert_eq!(ring_expand(&hash, 0), vec![hash.clone()]);
    assert_eq!(ring_expand(&hash, 1).len(), 9);
    // 2 rings = 5x5 = 25 cells; the centre is first so callers scan nearest-first.
    let two = ring_expand(&hash, 2);
    assert_eq!(two.len(), 25);
    assert_eq!(two[0], hash);
    // No duplicates.
    let unique: std::collections::HashSet<_> = two.iter().collect();
    assert_eq!(unique.len(), two.len());
}

#[test]
fn test_ring_expand_empty_hash() {
    assert!(ring_expand("", 3).is_empty());
}

// ---------------------------------------------------------------------------
// Cover computation
// ---------------------------------------------------------------------------

#[test]
fn test_centroid_cover_writes_one_cell_per_precision() {
    use raisin_models::nodes::properties::SpatialPolicy;
    let policy = SpatialPolicy::default();
    let point = GeoJson::point(8.5417, 47.3769);
    let computed = cells_for_geometry(&point, &policy)
        .expect("4326 is always normalisable")
        .expect("point must be indexable");
    assert_eq!(computed.cells.len(), policy.precisions.len());
    // The bbox of a point collapses to the point.
    assert_eq!(computed.bbox, [8.5417, 47.3769, 8.5417, 47.3769]);
    assert!(computed.z_range.is_none());
}

/// The write-amplification number the owner approved, asserted rather than claimed:
/// eight keys per geometry per revision, up from five. That is 1.6x, inside the 2x
/// budget — and it buys correct radii from 0.15 m to 1250 km.
#[test]
fn test_default_policy_costs_eight_keys_per_geometry() {
    use raisin_models::nodes::properties::SpatialPolicy;
    let computed = cells_for_geometry(&GeoJson::point(8.5417, 47.3769), &SpatialPolicy::default())
        .expect("4326 is always normalisable")
        .expect("indexable");
    assert_eq!(computed.cells.len(), 8);
}

#[test]
fn test_extent_cover_indexes_more_than_the_centroid_and_stays_capped() {
    use raisin_models::nodes::properties::{SpatialCoverMode, SpatialPolicy};

    // A polygon whose interior spans many fine cells.
    let polygon = GeoJson::Polygon {
        coordinates: vec![vec![
            [8.53, 47.37].into(),
            [8.55, 47.37].into(),
            [8.55, 47.38].into(),
            [8.53, 47.38].into(),
            [8.53, 47.37].into(),
        ]],
        srid: None,
    };

    let centroid_policy = SpatialPolicy::default();
    let mut extent_policy = SpatialPolicy::default();
    extent_policy.cover = SpatialCoverMode::Extent;

    let centroid = cells_for_geometry(&polygon, &centroid_policy)
        .expect("4326 is always normalisable")
        .expect("indexable");
    let extent = cells_for_geometry(&polygon, &extent_policy)
        .expect("4326 is always normalisable")
        .expect("indexable");

    assert!(
        extent.cells.len() > centroid.cells.len(),
        "extent cover must add cells"
    );

    // The cap is mandatory: a country-sized polygon at precision 11 would
    // otherwise want billions of cells. MAX_COVER_CELLS applies per precision.
    let max_total =
        raisin_models::nodes::properties::MAX_COVER_CELLS * extent_policy.precisions.len();
    assert!(
        extent.cells.len() <= max_total,
        "extent cover produced {} cells, above the {} cap",
        extent.cells.len(),
        max_total
    );
}

#[test]
fn test_bbox_of_extended_geometry() {
    let line = GeoJson::LineString {
        coordinates: vec![[1.0, 2.0].into(), [5.0, -3.0].into(), [3.0, 4.0].into()],
        srid: None,
    };
    assert_eq!(geometry_bbox(&line), Some([1.0, -3.0, 5.0, 4.0]));
}

/// A `MultiPolygon` used to be silently unindexed: the old hand-rolled centroid
/// table in this module returned `None` for every `Multi*` and for
/// `GeometryCollection`. It now delegates to `GeoJson::centroid`, which covers all
/// seven types.
#[test]
fn test_multi_geometry_is_indexable() {
    let multi = GeoJson::MultiPoint {
        coordinates: vec![[8.0, 47.0].into(), [9.0, 48.0].into()],
        srid: None,
    };
    assert!(geometry_centroid(&multi).is_some());
    assert!(!geohashes_for_geometry(&multi).is_empty());
}

#[test]
fn test_empty_geometry_is_not_indexed() {
    // An empty geometry has no position, so no query point can be near it.
    assert!(geohashes_for_geometry(&GeoJson::empty()).is_empty());
}

// ===== TEMPORARY REVIEW PROBES (to be reverted) =====

fn cover_holes(lon: f64, lat: f64, radius_m: f64, precisions: &[usize]) -> (usize, usize) {
    let plan = plan_radius_scan(lon, lat, radius_m, precisions);
    let (cells, precision) = match &plan {
        SpatialScanPlan::Covering { cells, precision } => (
            cells
                .iter()
                .cloned()
                .collect::<std::collections::HashSet<String>>(),
            *precision,
        ),
        SpatialScanPlan::NotCovering => return (usize::MAX, 0),
    };
    let earth = 6_371_008.8_f64;
    let mut misses = 0usize;
    let mut total = 0usize;
    for ring in [1.0_f64, 0.99, 0.9, 0.75, 0.5] {
        let r = radius_m * ring;
        for step in 0..360 {
            let bearing = (step as f64).to_radians();
            let dlat = (r / earth) * bearing.cos();
            let dlon = (r / earth) * bearing.sin() / lat.to_radians().cos();
            let plon = lon + dlon.to_degrees();
            let plat = lat + dlat.to_degrees();
            if !(-180.0..=180.0).contains(&plon) || !(-90.0..=90.0).contains(&plat) {
                continue;
            }
            let Some(cell) = encode_point(plon, plat, precision) else {
                continue;
            };
            total += 1;
            if !cells.contains(&cell) {
                misses += 1;
            }
        }
    }
    (misses, total)
}

#[test]
fn probe_high_latitude_and_antimeridian_cover() {
    let mut report = String::new();
    for &(lon, lat, label) in &[
        (8.5417, 47.3769, "zurich"),
        (8.5417, 70.0, "lat70"),
        (8.5417, 80.0, "lat80"),
        (8.5417, 85.0, "lat85"),
        (179.99, 0.0, "antimeridian-eq"),
        (-179.99, 40.0, "antimeridian-40"),
        (0.0, 0.0, "equator"),
        (0.0, -60.0, "south60"),
    ] {
        for radius in [1.0_f64, 10.0, 100.0, 1_000.0, 10_000.0, 100_000.0] {
            let (misses, total) = cover_holes(lon, lat, radius, INDEX_PRECISIONS);
            if misses == usize::MAX {
                report.push_str(&format!("{label} r={radius}: NOT COVERING\n"));
            } else if misses > 0 {
                report.push_str(&format!("{label} r={radius}: {misses}/{total} MISSES\n"));
            }
        }
    }
    assert!(report.is_empty(), "\n{report}");
}

// The planner/executor coverage divergence this file used to probe is now
// structurally impossible, so both the probe and the local mirror it needed have
// been removed.
//
// The probe compared [`plan_radius_scan`] against a hand-copied mirror of the
// planner's `radius_is_covered`, and it was RIGHT to fail: the mirror was
// latitude-agnostic, while a geohash cell narrows in longitude towards the poles.
// It reported `planner_covered = true` / `executor_covered = false` from about
// 5,000 km, and from 500 km at 89° N — and on exactly those queries the planner
// stripped `ST_DWithin` from the residual filter while the scan could only return
// a partial cell set, so rows went missing silently.
//
// `radius_is_covered` (raisin-sql-execution, catalog/spatial_availability.rs) now
// DELEGATES to `plan_radius_scan` instead of mirroring it, so there is one
// implementation and nothing left to diverge. The equivalence guard lives beside
// it as `the_planner_never_claims_coverage_the_executor_cannot_deliver`, which is
// the only crate that can see both sides — raisin-rocksdb cannot depend on
// raisin-sql-execution. Do not reintroduce a local copy here to "test" the
// planner; a copy is what caused the bug.

/// `plan_radius_scan` must be self-consistent about what it claims: a plan that
/// reports itself covering has to carry a cell list, and one that does not must
/// carry none. This is the executor-side half of the invariant whose planner-side
/// half is asserted in raisin-sql-execution (see the comment above).
#[test]
fn a_covering_plan_always_carries_cells() {
    for &(lon, lat) in &[
        (8.5417, 47.3769),
        (8.5417, 80.0),
        (8.5417, 89.0),
        (0.0, 0.0),
    ] {
        for radius in [0.5_f64, 50.0, 5_000.0, 500_000.0, 5_000_000.0, 15_000_000.0] {
            let plan = plan_radius_scan(lon, lat, radius, INDEX_PRECISIONS);
            if plan.is_covering() {
                assert!(
                    !plan.cells().is_empty(),
                    "covering plan at ({lon},{lat}) r={radius} carried no cells"
                );
            } else {
                assert!(
                    plan.cells().is_empty(),
                    "non-covering plan at ({lon},{lat}) r={radius} carried cells anyway"
                );
            }
        }
    }
}
