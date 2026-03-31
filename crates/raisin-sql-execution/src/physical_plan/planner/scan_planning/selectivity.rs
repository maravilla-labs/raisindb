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
            // Equality predicates are highly selective
            CanonicalPredicate::ColumnEq { .. } => 0.05,
            CanonicalPredicate::JsonPropertyEq { .. } => 0.05,
            // CHILD_OF is highly selective (only direct children)
            CanonicalPredicate::ChildOf { .. } => 0.10,
            // DESCENDANT_OF is moderately selective (all descendants under a path)
            CanonicalPredicate::DescendantOf { .. } => 0.15,
            // Spatial distance queries are moderately selective
            // Selectivity depends on radius: smaller radius = more selective
            CanonicalPredicate::SpatialDWithin { radius_meters, .. } => {
                // Estimate selectivity based on search radius
                // < 100m: very selective (0.01)
                // < 1km: selective (0.05)
                // < 10km: moderately selective (0.15)
                // > 10km: less selective (0.30)
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
