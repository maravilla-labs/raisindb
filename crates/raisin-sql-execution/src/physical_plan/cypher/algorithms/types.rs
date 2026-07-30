//! Re-export shim for the shared graph algorithm types.
//!
//! These types moved to [`crate::physical_plan::graph_algo::types`] so the
//! Cypher and SQL/PGQ engines share one set. They are re-exported here at
//! their historical paths — `algorithms::types::GraphAdjacency` and friends
//! still resolve, so no Cypher call site needed to change.

pub use crate::physical_plan::graph_algo::types::{
    BfsVisited, GraphAdjacency, GraphEdge, GraphNodeId, IndexedPath, WeightedIndexedPath,
};
