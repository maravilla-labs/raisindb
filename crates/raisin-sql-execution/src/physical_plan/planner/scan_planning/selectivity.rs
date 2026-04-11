//! Selectivity estimation for scan predicates
//!
//! Estimates the selectivity of filter predicates to choose optimal scan methods.
//! Lower values indicate more selective predicates (fewer matching rows).

use super::super::{CanonicalPredicate, PhysicalPlanner};

impl PhysicalPlanner {
    /// Estimate the selectivity of a predicate (lower is more selective)
    ///
    /// Selectivity estimates:
    /// - 0.05: Equality predicates (node_type =, property =)  - very selective
    /// - 0.20: Depth equality - moderately selective
    /// - 0.30: Prefix range (path LIKE) - less selective
    /// - 1.00: Other/unknown predicates - not selective
    pub(in super::super) fn estimate_selectivity(&self, predicate: &CanonicalPredicate) -> f64 {
        match predicate {
            // Equality predicates are highly selective.
            // When schema stats are available, use 1/count for node_type and archetype
            // columns; otherwise fall back to the default 0.05 heuristic.
            CanonicalPredicate::ColumnEq { column, .. } => {
                let col = column.to_lowercase();
                if col == "node_type" {
                    self.schema_stats
                        .as_ref()
                        .filter(|s| s.node_type_count > 0)
                        .map(|s| 1.0 / s.node_type_count as f64)
                        .unwrap_or(0.05)
                } else if col == "archetype" {
                    self.schema_stats
                        .as_ref()
                        .filter(|s| s.archetype_count > 0)
                        .map(|s| 1.0 / s.archetype_count as f64)
                        .unwrap_or(0.05)
                } else {
                    0.05
                }
            }
            CanonicalPredicate::JsonPropertyEq { .. } => 0.05,
            // CHILD_OF is highly selective (only direct children)
            CanonicalPredicate::ChildOf { .. } => 0.10,
            // DESCENDANT_OF is moderately selective (all descendants under a path)
            CanonicalPredicate::DescendantOf { .. } => 0.15,
            // Spatial distance queries: selectivity estimated from search radius.
            //
            // These heuristics assume roughly uniform data distribution across
            // the indexed geographic area. In practice, real data clusters
            // (e.g., urban areas), so these estimates are conservative:
            //   < 100m  -> 0.01  (neighborhood-scale: very few nodes)
            //   < 1km   -> 0.05  (district-scale: some nodes)
            //   < 10km  -> 0.15  (city-scale: moderate fraction)
            //   >= 10km -> 0.30  (regional-scale: large fraction)
            //
            // Without per-property data density statistics, we cannot do better.
            // For large radii (>10km) where spatial selectivity reads 0.30, a
            // DescendantOf scan (0.15) may "win" even though the spatial index
            // would scan fewer rows. This is acceptable: the SpatialDWithin
            // predicate is preserved as a row-level filter in that case, so
            // correctness is maintained.
            CanonicalPredicate::SpatialDWithin { radius_meters, .. } => {
                if *radius_meters < 100.0 {
                    0.01
                } else if *radius_meters < 1000.0 {
                    0.05
                } else if *radius_meters < 10000.0 {
                    0.15
                } else {
                    0.30
                }
            }
            // REFERENCES is highly selective (uses reverse reference index)
            // Typically returns only nodes that explicitly reference a specific target
            CanonicalPredicate::References { .. } => 0.05,
            // Depth equality is moderately selective
            CanonicalPredicate::DepthEq { .. } => 0.20,
            // Prefix ranges are less selective
            CanonicalPredicate::PrefixRange { .. } => 0.30,
            // Property prefix ranges (node_type LIKE 'prefix%') - similar to path prefix
            CanonicalPredicate::PropertyPrefixRange { .. } => 0.30,
            // Range comparisons (>, <, >=, <=) - moderately selective
            // Similar to prefix ranges but can be more or less selective depending on the bound
            CanonicalPredicate::RangeCompare { .. } => 0.35,
            // Other predicates are not selective
            CanonicalPredicate::Other(_) => 1.00,
        }
    }
}
