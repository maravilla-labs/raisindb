//! Hierarchy Function Rewriting
//!
//! Transforms hierarchy-specific functions into canonical predicates that can be
//! efficiently executed using RocksDB index scans.

mod comparison_op;
mod helpers;
mod predicate;
mod rewrite;
mod spatial;

#[cfg(test)]
mod tests;

pub use comparison_op::ComparisonOp;
pub use helpers::{compute_depth, compute_parent_path};
pub use predicate::CanonicalPredicate;
pub use rewrite::rewrite_hierarchy_predicates;
/// The single spatial predicate extractor, shared by this optimizer pass and the
/// execution planner's `filter_analysis`. Nothing else may pattern-match spatial
/// SQL syntax — two copies of that matcher is exactly how the index came to be
/// used for one spelling of a query and not another.
pub use spatial::{
    const_geometry, envelope_center, envelope_circumradius_meters, extract_distance_order,
    extract_geometry_source, extract_spatial_predicate, geojson_envelope, numeric_literal,
    SpatialDistanceOrder,
};
