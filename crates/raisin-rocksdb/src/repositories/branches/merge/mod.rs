//! Branch merge operations
//!
//! Implements Git-like merge functionality including fast-forward and three-way merges,
//! conflict detection, and conflict resolution.

mod apply;
mod deletion;
mod resolution;
mod superseded;
mod three_way;
mod unique_props;
