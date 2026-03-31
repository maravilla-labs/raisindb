//! # Pattern Matching Module
//!
//! Provides functions for matching Cypher graph patterns against storage.
//!
//! This module implements the core pattern matching logic for Cypher's MATCH clause,
//! handling nodes, relationships, paths, and variable-length paths.
//!
//! ## Modules
//!
//! - [`node`] - Match single node patterns (e.g., `(n:Label)`)
//! - [`relationship`] - Match relationship patterns between nodes (e.g., `-[r:TYPE]->`)
//! - [`path`] - Match complete path patterns (e.g., `(a)-[r]->(b)`)
//! - [`variable_length`] - Match variable-length paths with DFS (e.g., `-[*1..5]->`)
//!
//! ## Example
//!
//! ```ignore
//! use matching::match_path_pattern;
//!
//! // Match a path pattern
//! let pattern = parse_pattern("(n:Person)-[:KNOWS]->(friend)");
//! let bindings = match_path_pattern(&pattern, initial_binding, &storage, &context).await?;
//! ```
//!
//! ## Performance
//!
//! - Hot path functions are marked with `#[inline]`
//! - Pre-allocation of result vectors based on estimates
//! - Efficient filtering using iterator methods

mod node;
mod path;
mod relationship;
mod variable_length;

// Re-export public functions
pub use path::match_path_pattern;
