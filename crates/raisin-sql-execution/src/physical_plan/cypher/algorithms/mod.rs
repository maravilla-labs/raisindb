//! Graph algorithm implementations reachable from Cypher.
//!
//! Centrality, community detection, counting and distance algorithms live
//! here. The **path** algorithms (shortest path, A*, Yen's K-shortest) moved
//! to [`crate::physical_plan::graph_algo`] so SQL/PGQ can call the same copy;
//! they are re-exported below at their historical paths, so every existing
//! `algorithms::shortest_path(...)` call site still resolves.
//!
//! Note the re-exported path functions now return
//! [`crate::physical_plan::graph_algo::GraphPath`]. Cypher call sites convert
//! with `PathInfo::from(...)` — see the `From<GraphPath>` impl in
//! `cypher::types`.

pub mod betweenness;
pub mod bfs_distance;
pub mod cdlp;
pub mod centrality;
pub mod connected_components;
pub mod label_propagation;
pub mod lcc;
pub mod louvain;
pub mod pagerank;
pub mod sssp;
pub mod triangles;
pub mod types;

pub use betweenness::betweenness_centrality;
pub use bfs_distance::{bfs_distances, node_bfs_distance};
pub use cdlp::{cdlp, node_cdlp_community};
pub use centrality::closeness_centrality;
pub use connected_components::{component_count, node_component_id};
pub use label_propagation::{community_count, node_community_id};
pub use lcc::{lcc, node_lcc};
pub use louvain::node_louvain_community_id;
pub use pagerank::{pagerank, PageRankConfig};
pub use sssp::{
    node_sssp_distance, node_sssp_distance_weighted, sssp_distances, sssp_distances_weighted,
};
pub use triangles::node_triangle_count;
pub use types::{
    BfsVisited, GraphAdjacency, GraphEdge, GraphNodeId, IndexedPath, WeightedIndexedPath,
};

// Path algorithms — moved to `graph_algo`, re-exported here unchanged.
pub use crate::physical_plan::graph_algo::{
    all_shortest_paths, astar_shortest_path, cheapest_path, k_shortest_paths, shortest_path,
    CostError, CostSpec, GraphPath,
};
