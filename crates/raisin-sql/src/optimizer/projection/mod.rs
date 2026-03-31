//! Projection Pruning Optimization
//!
//! Computes the minimal set of columns required for query execution and pushes
//! projection requirements down to scan operators.
//!
//! # Key Features
//!
//! - Identifies columns referenced in filters, projections, sorts, and aggregates
//! - Includes ORDER BY columns even if not in SELECT list
//! - Pushes column requirements to Scan operators for efficient reads
//! - Handles computed expressions and function calls
//!
//! # Module Structure
//!
//! This module is split into submodules for maintainability:
//! - column_refs - Expression tree traversal to extract column references
//! - required_columns - Computing the minimal column set for a plan subtree
//! - pruning - Applying projection pushdown to Scan operators

mod column_refs;
mod pruning;
mod required_columns;

pub use column_refs::extract_column_refs;
pub use pruning::apply_projection_pruning;
pub use required_columns::compute_required_columns;

#[cfg(test)]
mod tests;
