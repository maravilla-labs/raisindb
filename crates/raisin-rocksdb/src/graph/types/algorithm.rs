//! Graph algorithm enum with Display and FromStr implementations

use serde::{Deserialize, Serialize};

/// Supported graph algorithms
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum GraphAlgorithm {
    PageRank,
    Louvain,
    ConnectedComponents,
    BetweennessCentrality,
    ClosenessCentrality,
    TriangleCount,
    RelatesCache,
    Bfs,
    Sssp,
    Cdlp,
    Lcc,
}

impl std::fmt::Display for GraphAlgorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PageRank => write!(f, "pagerank"),
            Self::Louvain => write!(f, "louvain"),
            Self::ConnectedComponents => write!(f, "connected_components"),
            Self::BetweennessCentrality => write!(f, "betweenness_centrality"),
            Self::ClosenessCentrality => write!(f, "closeness_centrality"),
            Self::TriangleCount => write!(f, "triangle_count"),
            Self::RelatesCache => write!(f, "relates_cache"),
            Self::Bfs => write!(f, "bfs"),
            Self::Sssp => write!(f, "sssp"),
            Self::Cdlp => write!(f, "cdlp"),
            Self::Lcc => write!(f, "lcc"),
        }
    }
}

impl std::str::FromStr for GraphAlgorithm {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "pagerank" | "page_rank" => Ok(Self::PageRank),
            "louvain" => Ok(Self::Louvain),
            "connected_components" | "connectedcomponents" => Ok(Self::ConnectedComponents),
            "betweenness_centrality" | "betweennesscentrality" | "betweenness" => {
                Ok(Self::BetweennessCentrality)
            }
            "closeness_centrality" | "closenesscentrality" | "closeness" => {
                Ok(Self::ClosenessCentrality)
            }
            "triangle_count" | "trianglecount" | "triangles" => Ok(Self::TriangleCount),
            "relates_cache" | "relatescache" | "relates" => Ok(Self::RelatesCache),
            "bfs" | "breadth_first_search" => Ok(Self::Bfs),
            "sssp" | "single_source_shortest_path" => Ok(Self::Sssp),
            "cdlp" | "community_detection_label_propagation" => Ok(Self::Cdlp),
            "lcc" | "local_clustering_coefficient" => Ok(Self::Lcc),
            _ => Err(format!("Unknown graph algorithm: {}", s)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_graph_algorithm_parse() {
        assert_eq!(
            "pagerank".parse::<GraphAlgorithm>().unwrap(),
            GraphAlgorithm::PageRank
        );
        assert_eq!(
            "louvain".parse::<GraphAlgorithm>().unwrap(),
            GraphAlgorithm::Louvain
        );
        assert_eq!(
            "connected_components".parse::<GraphAlgorithm>().unwrap(),
            GraphAlgorithm::ConnectedComponents
        );
    }
}
