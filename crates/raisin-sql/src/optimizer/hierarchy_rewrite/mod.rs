//! Hierarchy Function Rewriting
//!
//! Transforms hierarchy-specific functions into canonical predicates that can be
//! efficiently executed using RocksDB index scans.

mod comparison_op;
mod helpers;
mod predicate;
mod rewrite;

#[cfg(test)]
mod tests;

pub use comparison_op::ComparisonOp;
pub use helpers::{compute_depth, compute_parent_path};
pub use predicate::CanonicalPredicate;
pub use rewrite::rewrite_hierarchy_predicates;
