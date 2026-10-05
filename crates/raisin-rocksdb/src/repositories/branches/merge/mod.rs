//! Branch merge operations
//!
//! Implements Git-like merge functionality including fast-forward and three-way merges,
//! conflict detection, and conflict resolution.

mod apply;
mod compound_after_copy;
mod deletion;
mod localized_after_copy;
mod merged_view;
mod node_translations;
mod resolution;
mod superseded;
mod three_way;
mod translations;
mod unique_props;
