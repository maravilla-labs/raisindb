//! Function evaluation module
//!
//! Provides an enum-based, type-safe architecture for Cypher function evaluation.
//! Functions are dispatched through the CypherFunction enum which avoids trait
//! object safety issues while maintaining clean separation between function types.

pub mod aggregate;
pub mod centrality;
pub mod community;
pub mod graph;
pub mod path;
mod registry;
pub mod scalar;
mod traits;

pub use registry::CypherFunction;
pub use traits::FunctionContext;

use raisin_cypher_parser::Expr;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::Storage;

use crate::physical_plan::cypher::types::VariableBinding;

/// Evaluate a function call by name
///
/// This is the primary entry point for function evaluation from the executor.
/// It looks up the function by name, converts it to a CypherFunction enum variant,
/// and delegates to the enum's evaluate method.
///
/// # Arguments
///
/// * `name` - Function name (case-insensitive)
/// * `args` - Expression arguments to the function
/// * `binding` - Current variable binding
/// * `context` - Function evaluation context
///
/// # Returns
///
/// Result containing the computed PropertyValue or an Error
///
/// # Errors
///
/// Returns Error::Validation if:
/// - Unknown function name
/// - Wrong number of arguments
/// - Invalid argument types
///
/// Returns Error::Backend if:
/// - Storage operation fails
/// - Network error during distributed query
///
/// # Example
///
/// ```ignore
/// let result = evaluate_function(
///     "lookup",
///     &[id_expr, workspace_expr],
///     &binding,
///     &context,
/// ).await?;
/// ```
pub async fn evaluate_function<S: Storage>(
    name: &str,
    args: &[Expr],
    binding: &VariableBinding,
    context: &FunctionContext<'_, S>,
) -> Result<PropertyValue, Error> {
    // Lookup function by name and convert to enum
    let func = CypherFunction::from_name(name)
        .ok_or_else(|| Error::Validation(format!("Unknown function: {}", name)))?;

    // Delegate to function's evaluate method
    func.evaluate(args, binding, context).await
}

/// Check if a function is an aggregate
///
/// Aggregate functions (COUNT, SUM, AVG, MIN, MAX, COLLECT) require special
/// handling in the projection phase as they operate on groups of values.
///
/// # Arguments
///
/// * `name` - Function name (case-insensitive)
///
/// # Returns
///
/// true if the function is an aggregate, false otherwise (including unknown functions)
///
/// # Example
///
/// ```ignore
/// if is_aggregate_function("count") {
///     // Handle as aggregate in projection
/// } else {
///     // Evaluate normally
/// }
/// ```
pub fn is_aggregate_function(name: &str) -> bool {
    CypherFunction::from_name(name)
        .map(|f| f.is_aggregate())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_function_from_name() {
        // Test all function lookups
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
    fn test_case_insensitive_lookup() {
        // Test case-insensitive lookup
        assert_eq!(
            CypherFunction::from_name("LOOKUP"),
            Some(CypherFunction::Lookup)
        );
        assert_eq!(
            CypherFunction::from_name("Lookup"),
            Some(CypherFunction::Lookup)
        );
        assert_eq!(
            CypherFunction::from_name("lookup"),
            Some(CypherFunction::Lookup)
        );
    }

    #[test]
    fn test_aggregate_detection() {
        // Aggregate functions
        assert!(is_aggregate_function("count"));
        assert!(is_aggregate_function("COUNT"));
        assert!(is_aggregate_function("sum"));
        assert!(is_aggregate_function("avg"));
        assert!(is_aggregate_function("min"));
        assert!(is_aggregate_function("max"));
        assert!(is_aggregate_function("collect"));

        // Non-aggregate functions
        assert!(!is_aggregate_function("lookup"));
        assert!(!is_aggregate_function("type"));
        assert!(!is_aggregate_function("resolve_node_path"));
        assert!(!is_aggregate_function("degree"));
        assert!(!is_aggregate_function("pagerank"));
        assert!(!is_aggregate_function("shortestpath"));
    }

    #[test]
    fn test_unknown_function() {
        // Unknown function should return None
        assert_eq!(CypherFunction::from_name("nonexistent"), None);
        assert!(!is_aggregate_function("nonexistent"));
    }
}
