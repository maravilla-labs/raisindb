//! Enum-based function dispatch for Cypher
//!
//! Replaces trait-based registry with a simple enum dispatch that works
//! with Storage's generic parameters and avoids trait object safety issues.

use raisin_cypher_parser::Expr;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::Storage;

use super::traits::FunctionContext;
use crate::physical_plan::cypher::types::VariableBinding;

/// All available Cypher functions
///
/// Each variant represents a built-in function that can be called in Cypher queries.
/// This enum-based dispatch avoids trait object safety issues while maintaining
/// clean separation between function types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CypherFunction {
    // Scalar functions
    /// lookup(id, workspace) - Fetch a node by ID and workspace
    Lookup,
    /// type(r) - Get the type of a relationship
    Type,
    /// resolve_node_path(workspace, path) - Fast O(1) path to ID lookup
    ResolveNodePath,

    // Graph structure functions
    /// degree(node) - Total number of relationships connected to a node
    Degree,
    /// indegree(node) - Number of incoming relationships
    InDegree,
    /// outdegree(node) - Number of outgoing relationships
    OutDegree,

    // Path finding functions
    /// shortestpath(start, end) - Find shortest path between two nodes
    ShortestPath,
    /// allshortestpaths(start, end) - Find all shortest paths between two nodes
    AllShortestPaths,
    /// astar(start, end, config?) - Find shortest path using A*
    AStar,
    /// kshortestpaths(start, end, k, config?) - Find K shortest paths between two nodes
    KShortestPaths,
    /// distance(start, end) - Calculate distance between two nodes
    Distance,

    // Centrality functions
    /// pagerank(node) - Calculate PageRank centrality
    PageRank,
    /// closeness(node) - Calculate closeness centrality
    Closeness,
    /// betweenness(node) - Calculate betweenness centrality
    Betweenness,

    // Community detection functions
    /// componentid(node) - Get connected component ID
    ComponentId,
    /// componentcount() - Get number of connected components
    ComponentCount,
    /// communityid(node) - Get community ID from community detection
    CommunityId,
    /// communitycount() - Get number of detected communities
    CommunityCount,
    /// louvain(node) - Get community ID using Louvain algorithm
    Louvain,
    /// trianglecount(node) - Get number of triangles a node participates in
    TriangleCount,

    // Aggregate functions
    /// count(*) or count(expr) - Count values
    Count,
    /// sum(expr) - Sum numeric values
    Sum,
    /// avg(expr) - Average of numeric values
    Avg,
    /// min(expr) - Minimum value
    Min,
    /// max(expr) - Maximum value
    Max,
    /// collect(expr) - Collect values into an array
    Collect,
}

impl CypherFunction {
    /// Parse function name to enum variant
    ///
    /// # Arguments
    ///
    /// * `name` - Function name (case-insensitive)
    ///
    /// # Returns
    ///
    /// Some(CypherFunction) if the name matches a known function, None otherwise
    ///
    /// # Example
    ///
    /// ```
    /// # use raisin_sql::physical_plan::cypher::evaluation::functions::CypherFunction;
    /// assert_eq!(CypherFunction::from_name("lookup"), Some(CypherFunction::Lookup));
    /// assert_eq!(CypherFunction::from_name("LOOKUP"), Some(CypherFunction::Lookup));
    /// assert_eq!(CypherFunction::from_name("unknown"), None);
    /// ```
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "lookup" => Some(Self::Lookup),
            "type" => Some(Self::Type),
            "resolve_node_path" => Some(Self::ResolveNodePath),
            "degree" => Some(Self::Degree),
            "indegree" => Some(Self::InDegree),
            "outdegree" => Some(Self::OutDegree),
            "shortestpath" => Some(Self::ShortestPath),
            "allshortestpaths" => Some(Self::AllShortestPaths),
            "astar" => Some(Self::AStar),
            "kshortestpaths" => Some(Self::KShortestPaths),
            "distance" => Some(Self::Distance),
            "pagerank" => Some(Self::PageRank),
            "closeness" => Some(Self::Closeness),
            "betweenness" => Some(Self::Betweenness),
            "componentid" => Some(Self::ComponentId),
            "componentcount" => Some(Self::ComponentCount),
            "communityid" => Some(Self::CommunityId),
            "communitycount" => Some(Self::CommunityCount),
            "louvain" => Some(Self::Louvain),
            "trianglecount" => Some(Self::TriangleCount),
            "count" => Some(Self::Count),
            "sum" => Some(Self::Sum),
            "avg" => Some(Self::Avg),
            "min" => Some(Self::Min),
            "max" => Some(Self::Max),
            "collect" => Some(Self::Collect),
            _ => None,
        }
    }

    /// Check if function is an aggregate
    ///
    /// Aggregate functions (COUNT, SUM, AVG, MIN, MAX, COLLECT) require special
    /// handling in projection as they operate on groups of values.
    ///
    /// # Returns
    ///
    /// true if the function is an aggregate, false otherwise
    pub fn is_aggregate(&self) -> bool {
        matches!(
            self,
            Self::Count | Self::Sum | Self::Avg | Self::Min | Self::Max | Self::Collect
        )
    }

    /// Evaluate the function with given arguments
    ///
    /// Dispatches to the appropriate function implementation based on the variant.
    ///
    /// # Arguments
    ///
    /// * `args` - Expression arguments passed to the function
    /// * `binding` - Current variable binding (contains matched nodes/relationships)
    /// * `context` - Execution context with storage access and query parameters
    ///
    /// # Returns
    ///
    /// Result containing the computed PropertyValue or an Error
    ///
    /// # Errors
    ///
    /// Returns Error::Validation if:
    /// - Wrong number of arguments
    /// - Invalid argument types
    /// - Invalid node/relationship references
    ///
    /// Returns Error::Backend if:
    /// - Storage operation fails
    /// - Network error during distributed query
    pub async fn evaluate<S: Storage>(
        &self,
        args: &[Expr],
        binding: &VariableBinding,
        context: &FunctionContext<'_, S>,
    ) -> Result<PropertyValue, Error> {
        match self {
            Self::Lookup => super::scalar::evaluate_lookup(args, binding, context).await,
            Self::Type => super::scalar::evaluate_type(args, binding, context).await,
            Self::ResolveNodePath => {
                super::scalar::evaluate_resolve_node_path(args, binding, context).await
            }
            Self::Degree => super::graph::evaluate_degree(args, binding, context).await,
            Self::InDegree => super::graph::evaluate_indegree(args, binding, context).await,
            Self::OutDegree => super::graph::evaluate_outdegree(args, binding, context).await,
            Self::ShortestPath => super::path::evaluate_shortest_path(args, binding, context).await,
            Self::AllShortestPaths => {
                super::path::evaluate_all_shortest_paths(args, binding, context).await
            }
            Self::AStar => super::path::evaluate_astar(args, binding, context).await,
            Self::KShortestPaths => {
                super::path::evaluate_k_shortest_paths(args, binding, context).await
            }
            Self::Distance => super::path::evaluate_distance(args, binding, context).await,
            Self::PageRank => super::centrality::evaluate_pagerank(args, binding, context).await,
            Self::Closeness => super::centrality::evaluate_closeness(args, binding, context).await,
            Self::Betweenness => {
                super::centrality::evaluate_betweenness(args, binding, context).await
            }
            Self::ComponentId => {
                super::community::evaluate_component_id(args, binding, context).await
            }
            Self::ComponentCount => {
                super::community::evaluate_component_count(args, binding, context).await
            }
            Self::CommunityId => {
                super::community::evaluate_community_id(args, binding, context).await
            }
            Self::CommunityCount => {
                super::community::evaluate_community_count(args, binding, context).await
            }
            Self::Louvain => super::community::evaluate_louvain(args, binding, context).await,
            Self::TriangleCount => {
                super::community::evaluate_triangle_count(args, binding, context).await
            }
            Self::Count | Self::Sum | Self::Avg | Self::Min | Self::Max | Self::Collect => {
                // Aggregates return marker values during evaluation
                // Actual aggregation happens in the executor's projection phase
                super::aggregate::evaluate_aggregate(self, args, binding, context).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_name_case_insensitive() {
        assert_eq!(
            CypherFunction::from_name("lookup"),
            Some(CypherFunction::Lookup)
        );
        assert_eq!(
            CypherFunction::from_name("LOOKUP"),
            Some(CypherFunction::Lookup)
        );
        assert_eq!(
            CypherFunction::from_name("Lookup"),
            Some(CypherFunction::Lookup)
        );
    }

    #[test]
    fn test_from_name_all_functions() {
        // Scalar
        assert_eq!(
            CypherFunction::from_name("lookup"),
            Some(CypherFunction::Lookup)
        );
        assert_eq!(
            CypherFunction::from_name("type"),
            Some(CypherFunction::Type)
        );
        assert_eq!(
            CypherFunction::from_name("resolve_node_path"),
            Some(CypherFunction::ResolveNodePath)
        );

        // Graph structure
        assert_eq!(
            CypherFunction::from_name("degree"),
            Some(CypherFunction::Degree)
        );
        assert_eq!(
            CypherFunction::from_name("indegree"),
            Some(CypherFunction::InDegree)
        );
        assert_eq!(
            CypherFunction::from_name("outdegree"),
            Some(CypherFunction::OutDegree)
        );

        // Path finding
        assert_eq!(
            CypherFunction::from_name("shortestpath"),
            Some(CypherFunction::ShortestPath)
        );
        assert_eq!(
            CypherFunction::from_name("allshortestpaths"),
            Some(CypherFunction::AllShortestPaths)
        );
        assert_eq!(
            CypherFunction::from_name("distance"),
            Some(CypherFunction::Distance)
        );

        // Centrality
        assert_eq!(
            CypherFunction::from_name("pagerank"),
            Some(CypherFunction::PageRank)
        );
        assert_eq!(
            CypherFunction::from_name("closeness"),
            Some(CypherFunction::Closeness)
        );
        assert_eq!(
            CypherFunction::from_name("betweenness"),
            Some(CypherFunction::Betweenness)
        );

        // Community
        assert_eq!(
            CypherFunction::from_name("componentid"),
            Some(CypherFunction::ComponentId)
        );
        assert_eq!(
            CypherFunction::from_name("componentcount"),
            Some(CypherFunction::ComponentCount)
        );
        assert_eq!(
            CypherFunction::from_name("communityid"),
            Some(CypherFunction::CommunityId)
        );
        assert_eq!(
            CypherFunction::from_name("communitycount"),
            Some(CypherFunction::CommunityCount)
        );

        // Aggregates
        assert_eq!(
            CypherFunction::from_name("count"),
            Some(CypherFunction::Count)
        );
        assert_eq!(CypherFunction::from_name("sum"), Some(CypherFunction::Sum));
        assert_eq!(CypherFunction::from_name("avg"), Some(CypherFunction::Avg));
        assert_eq!(CypherFunction::from_name("min"), Some(CypherFunction::Min));
        assert_eq!(CypherFunction::from_name("max"), Some(CypherFunction::Max));
        assert_eq!(
            CypherFunction::from_name("collect"),
            Some(CypherFunction::Collect)
        );
    }

    #[test]
    fn test_from_name_unknown() {
        assert_eq!(CypherFunction::from_name("unknown"), None);
        assert_eq!(CypherFunction::from_name("nonexistent"), None);
    }

    #[test]
    fn test_is_aggregate() {
        // Aggregates
        assert!(CypherFunction::Count.is_aggregate());
        assert!(CypherFunction::Sum.is_aggregate());
        assert!(CypherFunction::Avg.is_aggregate());
        assert!(CypherFunction::Min.is_aggregate());
        assert!(CypherFunction::Max.is_aggregate());
        assert!(CypherFunction::Collect.is_aggregate());

        // Non-aggregates
        assert!(!CypherFunction::Lookup.is_aggregate());
        assert!(!CypherFunction::Type.is_aggregate());
        assert!(!CypherFunction::ResolveNodePath.is_aggregate());
        assert!(!CypherFunction::Degree.is_aggregate());
        assert!(!CypherFunction::PageRank.is_aggregate());
        assert!(!CypherFunction::ShortestPath.is_aggregate());
    }
}
