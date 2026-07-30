//! Engine-neutral graph path algorithms, shared by Cypher and SQL/PGQ.
//!
//! These used to live under `physical_plan/cypher/algorithms/` and were
//! reachable only through Cypher-shaped types (`cypher::types::PathInfo`,
//! whose nodes lost their node type, and `RelationInfo`, which carries Cypher
//! *binding* concepts a path algorithm has no business knowing). PGQ needed
//! the same maths with node types and edge weights, so the types moved to the
//! PGQ shape and the algorithms moved here.
//!
//! `cypher::algorithms` re-exports everything below at its historical paths,
//! so every existing `algorithms::shortest_path(...)` call site still resolves.
//! `impl From<GraphPath> for cypher::types::PathInfo` bridges the return type.
//!
//! # What lives here
//!
//! | item | selector it backs |
//! |---|---|
//! | [`shortest_path`] | `ANY SHORTEST` |
//! | [`all_shortest_paths`] | `ALL SHORTEST` |
//! | [`cheapest_path`] | `ANY CHEAPEST` (RaisinDB extension) |
//! | [`astar_shortest_path`] | Cypher `astar()` |
//! | [`k_shortest_paths`] | `SHORTEST k` (deferred, see `docs/OPEN-ITEMS.md`) |
//!
//! The remaining algorithms (pagerank, betweenness, cdlp, louvain, lcc, sssp,
//! triangles, connected components, centrality, bfs distance, label
//! propagation) deliberately stay in `cypher::algorithms` — PGQ already
//! reaches them there and moving files nobody asked to move is churn.

pub mod astar;
pub mod cost;
pub mod path;
pub mod shortest_path;
pub mod types;
pub mod yen;

#[cfg(test)]
mod path_tests;
#[cfg(test)]
mod tests;

pub use astar::{astar_shortest_path, cheapest_path};
pub use cost::{CostError, CostSpec};
pub use path::{GraphPath, PathEdge, PathNode};
pub use shortest_path::{all_shortest_paths, shortest_path};
pub use types::{
    BfsVisited, GraphAdjacency, GraphEdge, GraphNodeId, IndexedPath, WeightedIndexedPath,
};
pub use yen::k_shortest_paths;
