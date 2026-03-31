//! # Cypher Query Execution Module
//!
//! This module provides a complete Cypher query execution engine for graph queries.
//! It implements Cypher as a table-valued function that can be called from SQL.
//!
//! ## Architecture
//!
//! The module is organized into several focused submodules:
//!
//! - [`algorithms`] - Graph algorithms (PageRank, shortest path, centrality, community detection)
//! - [`context`] - Execution context management ([`ExecutionContext`])
//! - [`evaluation`] - Expression and function evaluation
//! - [`matching`] - Pattern matching for nodes, relationships, and paths
//! - [`projection`] - Result projection and aggregation ([`ProjectionEngine`](projection::ProjectionEngine))
//! - [`types`] - Core data structures ([`CypherRow`], [`VariableBinding`], etc.)
//! - [`utils`] - Shared utility functions
//!
//! ## Supported Features
//!
//! - ✅ **MATCH patterns** - Node, relationship, and path pattern matching
//! - ✅ **Variable-length paths** - e.g., `[:KNOWS*1..5]`
//! - ✅ **WHERE clauses** - Complex filtering with expressions
//! - ✅ **RETURN projection** - Simple and aggregate projections
//! - ✅ **Aggregations** - GROUP BY with COUNT, SUM, AVG, MIN, MAX, COLLECT
//! - ✅ **Graph algorithms** - PageRank, centrality measures, community detection
//! - ✅ **Built-in functions** - 20+ functions for strings, numbers, nodes, relationships
//! - ⏸️ **CREATE/UPDATE/DELETE** - Stub implementation (not yet functional)
//!
//! ## Example Usage
//!
//! ```no_run
//! use raisin_sql::physical_plan::cypher::execute_cypher;
//! use std::collections::HashMap;
//! use std::sync::Arc;
//!
//! # async fn example(storage: Arc<impl raisin_storage::Storage + 'static>) {
//! // Execute a Cypher query
//! let query = "MATCH (n:Person)-[:KNOWS]->(friend) RETURN n.name, friend.name";
//! let results = execute_cypher(
//!     storage,
//!     "workspace_id".to_string(),
//!     "tenant_id".to_string(),
//!     "repo_id".to_string(),
//!     "main".to_string(),
//!     None,
//!     query,
//!     HashMap::new(),
//! );
//!
//! // Process results
//! use futures::StreamExt;
//! let mut stream = results;
//! while let Some(result) = stream.next().await {
//!     match result {
//!         Ok(row) => println!("Row: {:?}", row),
//!         Err(e) => eprintln!("Error: {}", e),
//!     }
//! }
//! # }
//! ```
//!
//! ## Design Principles
//!
//! - **Modular architecture** - Each submodule has a single, well-defined responsibility
//! - **Async-first** - Built on Tokio for efficient I/O
//! - **Type safety** - Leverages Rust's type system for correctness
//! - **Performance** - Optimized hot paths with inline hints and lazy tracing
//! - **Graph-only semantics** - Queries only nodes that participate in relationships

mod algorithms;
mod context;
mod evaluation;
mod executor;
mod matching;
mod projection;
mod types;
mod utils;

pub use context::ExecutionContext;
pub use types::{CypherContext, CypherRow, NodeInfo, PathInfo, RelationInfo, VariableBinding};

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use async_stream::try_stream;
use futures::Stream;
use raisin_cypher_parser::parse_query;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::Storage;

use crate::physical_plan::executor::ExecutionError;
use raisin_sql::analyzer::types::DataType;

type Result<T> = std::result::Result<T, ExecutionError>;

/// Execute a Cypher query and return results as a stream
pub fn execute_cypher<S: Storage + 'static>(
    storage: Arc<S>,
    workspace_id: String,
    tenant_id: String,
    repo_id: String,
    branch: String,
    revision: Option<raisin_hlc::HLC>,
    cypher_query: &str,
    parameters: HashMap<String, PropertyValue>,
) -> Pin<Box<dyn Stream<Item = Result<CypherRow>> + Send>> {
    let query_string = cypher_query.to_string();

    Box::pin(try_stream! {
        tracing::info!("🔵 Parsing Cypher query...");

        // Parse the Cypher query
        let query = parse_query(&query_string).map_err(|e| {
            tracing::error!("❌ Cypher parse error: {}", e);
            ExecutionError::Validation(format!("Invalid Cypher query: {}", e))
        })?;

        tracing::debug!("   ✓ Query parsed successfully: {} clauses", query.clauses.len());

        // Create execution context
        let context = CypherContext::new(workspace_id.clone(), tenant_id.clone(), repo_id.clone(), branch.clone(), revision)
            .with_parameters(parameters);
        tracing::debug!("   Context: tenant={}, repo={}, branch={}, workspace={}, revision={:?}",
            context.tenant_id, context.repo_id, context.branch, context.workspace_id, context.revision);

        // Create executor
        let executor = executor::CypherExecutor::new(storage.clone(), context);

        // Execute query
        tracing::info!("🔵 Executing Cypher query...");
        let results = executor.execute(query).await?;
        tracing::info!("   ✓ Execution complete: {} results", results.len());

        // Yield results
        for (idx, row) in results.into_iter().enumerate() {
            tracing::debug!("   Yielding result #{}", idx + 1);
            yield row;
        }

        tracing::info!("✅ Cypher execution finished");
    })
}

/// Extract column information from Cypher RETURN clause
pub fn get_cypher_columns(
    return_items: &[raisin_cypher_parser::ReturnItem],
) -> Vec<(String, DataType)> {
    return_items
        .iter()
        .map(|item| {
            let col_name = item.alias.clone().unwrap_or_else(|| "result".to_string());
            // For now, assume all Cypher results are JSON (agtype in PostgreSQL AGE)
            (col_name, DataType::JsonB)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_cypher() {
        let query = "MATCH (n:Person) RETURN n";
        let parsed = parse_query(query);
        assert!(parsed.is_ok(), "Should parse simple MATCH...RETURN query");
    }

    #[test]
    fn test_parse_complex_cypher() {
        let query = r#"
            MATCH (a:Person), (b:Person)
            WHERE a.name = 'Node A' AND b.name = 'Node B'
            CREATE (a)-[e:RELTYPE {name: a.name + '<->' + b.name}]->(b)
            RETURN e
        "#;
        let parsed = parse_query(query);
        assert!(parsed.is_ok(), "Should parse complex query");
    }
}
