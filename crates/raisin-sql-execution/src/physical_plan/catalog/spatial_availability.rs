//! The planner's half of the spatial availability decision.
//!
//! The *state* half — is there an index, at which precisions, is the record
//! trustworthy — belongs to storage and lives in
//! [`raisin_storage::SpatialAvailability`], which is re-exported here so the
//! planner and the storage layer cannot drift into two notions of "ready".
//!
//! What this module adds is the *coverage* half: given the precisions the index
//! was actually built at, can a particular radius at a particular latitude be
//! answered **completely**? That question is NOT pure arithmetic over a cell-size
//! table — a geohash cell narrows in longitude towards the poles — so
//! [`radius_is_covered`] delegates to the executor's own
//! `raisin_rocksdb::spatial::plan_radius_scan` rather than approximating it.
//! Planner and executor must give the same answer or the planner will strip a
//! predicate the scan cannot honour; delegation is what makes that impossible.

use raisin_models::nodes::properties::MAX_SCAN_CELLS;

pub use raisin_storage::spatial::{SpatialAvailability, SpatialStateSource};

/// Approximate cell radius in metres per geohash precision, for DIAGNOSTICS ONLY.
///
/// # Not used for coverage decisions any more
///
/// This table once backed [`radius_is_covered`], on the stated belief that "the
/// planner cannot import `raisin_rocksdb::spatial::ops::precision_radius_meters`
/// because `raisin-rocksdb` depends on this crate, not the other way round".
/// **That belief was wrong** — `raisin-sql-execution` declares `raisin-rocksdb` as
/// a normal dependency, so the authority was importable all along, and the mirror
/// existed for no reason. It then drifted, because these values are a single
/// number per precision while the executor's cover is latitude-dependent, and the
/// drift stripped `ST_DWithin` from queries the scan could only answer partially.
///
/// [`radius_is_covered`] now delegates to the executor. Keep this table only for
/// human-facing size estimates, and do not reintroduce it into any correctness
/// path — if you need to know whether a radius is coverable, ask the executor.
pub const GEOHASH_CELL_RADIUS_METERS: &[(usize, f64)] = &[
    (1, 5_000_000.0),
    (2, 1_250_000.0),
    (3, 156_000.0),
    (4, 39_000.0),
    (5, 4_900.0),
    (6, 1_200.0),
    (7, 153.0),
    (8, 38.0),
    (9, 4.8),
    (10, 1.2),
    (11, 0.15),
    (12, 0.04),
];

/// A short, actionable reason suitable for EXPLAIN output and the fallback
/// warning.
///
/// Lives here rather than on the storage enum because the *remedy* is a planner /
/// operator concern: storage knows the state, the planner knows what a user can
/// do about it.
pub fn explain_reason(availability: &SpatialAvailability) -> String {
    match availability {
        SpatialAvailability::Ready { precisions, .. } => {
            format!("ready at precisions {:?}", precisions)
        }
        SpatialAvailability::NotBuilt => {
            "spatial index NOT BUILT — run REBUILD SPATIAL INDEX".to_string()
        }
        SpatialAvailability::Unusable(reason) => format!("spatial index unusable: {}", reason),
    }
}

/// The configured discriminator ("bucket") property, when the index carries one.
///
/// A free function rather than a method because [`SpatialAvailability`] is owned
/// by the storage crate; keeping planner-side conveniences here avoids two
/// crates both trying to extend the same enum.
pub fn bucket_property(availability: &SpatialAvailability) -> Option<&str> {
    match availability {
        SpatialAvailability::Ready {
            bucket_property, ..
        } => bucket_property.as_deref(),
        _ => None,
    }
}

/// Cell radius for one precision, or `0.0` for an out-of-range precision (which
/// then contributes no coverage).
fn cell_radius_meters(precision: usize) -> f64 {
    GEOHASH_CELL_RADIUS_METERS
        .iter()
        .find(|(p, _)| *p == precision)
        .map(|(_, r)| *r)
        .unwrap_or(0.0)
}

/// Whether a radius query can be answered **completely** from an index built at
/// `precisions`, centred on `(center_lon, center_lat)`.
///
/// This is the planner's second precondition for dropping a spatial predicate
/// from the residual filter, so it must agree with the executor EXACTLY. It
/// therefore delegates to the executor's own planner,
/// [`raisin_rocksdb::spatial::plan_radius_scan`], rather than reimplementing the
/// decision.
///
/// # Why this is a delegation and not a mirror
///
/// It used to be a mirror — a latitude-agnostic approximation over
/// [`cell_radius_meters`] — and the two drifted. A geohash cell narrows in
/// longitude as latitude rises, so the same metric radius needs more cells near
/// the poles; the mirror could not see that. The result was
/// `planner_covered = true` / `executor_covered = false` for radii from about
/// 5,000 km, and from as little as 500 km at 89° N. On exactly those queries the
/// planner stripped `ST_DWithin` from the residual filter while the scan could
/// only return a *partial* cell set — so rows went missing with nothing left to
/// catch them.
///
/// That is the same silently-wrong-results shape as the old hardcoded
/// `has_spatial_index() == true`, and the reason this function now has a single
/// implementation shared with the code that actually performs the scan. Keep it
/// a delegation: any cheaper local approximation reintroduces the drift.
pub fn radius_is_covered(
    center_lon: f64,
    center_lat: f64,
    radius_meters: f64,
    precisions: &[usize],
) -> bool {
    raisin_rocksdb::spatial::plan_radius_scan(center_lon, center_lat, radius_meters, precisions)
        .is_covering()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Zurich — a representative mid-latitude query point.
    const LON: f64 = 8.5417;
    const LAT: f64 = 47.3769;

    /// The default precision set must cover the whole advertised range,
    /// sub-metre to global. The old `&[4, 5, 6, 7, 8]` set silently returned
    /// nothing outside roughly 4.8 m - 39 km; these are radii that used to fall
    /// off that cliff.
    #[test]
    fn default_precisions_cover_sub_metre_through_global() {
        let precisions = raisin_models::nodes::properties::INDEX_PRECISIONS_DEFAULT;
        for radius in [
            0.05, 0.5, 1.0, 5.0, 50.0, 500.0, 5_000.0, 50_000.0, 500_000.0,
        ] {
            assert!(
                radius_is_covered(LON, LAT, radius, precisions),
                "radius {}m must be coverable by the default precision set",
                radius
            );
        }
    }

    /// A sub-metre query against a coarse-only index is STILL covered — it just
    /// over-fetches. This is the property that lets a heterogeneous cluster
    /// answer identically at differing latency.
    #[test]
    fn a_fine_radius_is_covered_by_a_coarse_index() {
        assert!(radius_is_covered(LON, LAT, 0.1, &[8]));
        assert!(radius_is_covered(LON, LAT, 37.0, &[8]));
    }

    /// Beyond the coarsest cell, coverage depends on the ring budget.
    #[test]
    fn a_huge_radius_beyond_the_ring_budget_is_not_covered() {
        // 38 m cells, 100 km radius => ~2632 rings => far past MAX_SCAN_CELLS.
        assert!(!radius_is_covered(LON, LAT, 100_000.0, &[8]));
        // A few rings is fine.
        assert!(radius_is_covered(LON, LAT, 150.0, &[8]));
    }

    #[test]
    fn nonsense_radii_and_empty_precision_sets_are_never_covered() {
        assert!(!radius_is_covered(LON, LAT, f64::NAN, &[8]));
        assert!(!radius_is_covered(LON, LAT, f64::INFINITY, &[8]));
        assert!(!radius_is_covered(LON, LAT, -1.0, &[8]));
        assert!(!radius_is_covered(LON, LAT, 10.0, &[]));
        assert!(!radius_is_covered(LON, LAT, 10.0, &[99]));
        // A non-finite CENTRE is just as uncoverable as a non-finite radius.
        assert!(!radius_is_covered(f64::NAN, LAT, 10.0, &[8]));
        assert!(!radius_is_covered(LON, f64::INFINITY, 10.0, &[8]));
    }

    /// The regression this delegation exists to prevent.
    ///
    /// A geohash cell narrows in longitude towards the poles, so a radius that
    /// is comfortably coverable at 47° N need not be at 89° N. The old
    /// latitude-agnostic mirror answered `true` for both and the planner then
    /// stripped `ST_DWithin`, leaving a partial scan with nothing to correct it.
    /// The planner must now never claim coverage the executor cannot deliver.
    #[test]
    fn the_planner_never_claims_coverage_the_executor_cannot_deliver() {
        let precisions = raisin_models::nodes::properties::INDEX_PRECISIONS_DEFAULT;
        for &(lon, lat, label) in &[
            (8.5417, 47.3769, "zurich"),
            (8.5417, 80.0, "lat80"),
            (8.5417, 89.0, "lat89"),
            (0.0, 0.0, "equator"),
        ] {
            for radius in [
                0.5,
                5.0,
                50.0,
                500.0,
                5_000.0,
                50_000.0,
                200_000.0,
                500_000.0,
                1_000_000.0,
                5_000_000.0,
                10_000_000.0,
                15_000_000.0,
            ] {
                let planner = radius_is_covered(lon, lat, radius, precisions);
                let executor =
                    raisin_rocksdb::spatial::plan_radius_scan(lon, lat, radius, precisions)
                        .is_covering();
                assert_eq!(
                    planner, executor,
                    "{label} r={radius}: planner said {planner}, executor said {executor}"
                );
            }
        }
    }

    /// Pinned so a change to either table shows up as a test failure rather than
    /// as a stripped predicate the scan cannot honour.
    #[test]
    fn cell_radius_table_is_pinned() {
        assert_eq!(cell_radius_meters(11), 0.15);
        assert_eq!(cell_radius_meters(9), 4.8);
        assert_eq!(cell_radius_meters(8), 38.0);
        assert_eq!(cell_radius_meters(6), 1_200.0);
        assert_eq!(cell_radius_meters(4), 39_000.0);
        assert_eq!(cell_radius_meters(2), 1_250_000.0);
        assert_eq!(cell_radius_meters(0), 0.0);
        assert_eq!(cell_radius_meters(13), 0.0);
    }
}
