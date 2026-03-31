//! SQL/PGQ (Property Graph Query) Execution Module
//!
//! Implements ISO SQL:2023 Part 16 GRAPH_TABLE execution for RaisinDB.
//!
//! # Architecture
//!
//! The module provides execution for `GRAPH_TABLE` queries embedded in SQL:
//!
//! ```sql
//! SELECT * FROM GRAPH_TABLE(
//!     MATCH (a:User)-[:FOLLOWS]->(b:User)
//!     WHERE a.name = 'alice'
//!     COLUMNS (a.id, b.name AS friend)
//! ) AS friends
//! ```
//!
//! # Submodules
//!
//! - [`context`] - Execution context (tenant, repo, workspace, revision)
//! - [`types`] - Core types (PgqRow, VariableBinding)
//! - [`matching`] - Graph pattern matching
//! - [`filter`] - WHERE clause evaluation
//! - [`projection`] - COLUMNS clause projection
//! - [`aggregation`] - Aggregate functions (COUNT, COLLECT)
//!
//! # Output Format
//!
//! Unlike Cypher which returns nested graph objects, GRAPH_TABLE returns
//! flat SQL rows that integrate seamlessly with the rest of SQL:
//!
//! ```text
//! | user_id | friend_name | weight |
//! |---------|-------------|--------|
//! | alice   | bob         | 0.9    |
//! | alice   | charlie     | 0.8    |
//! ```

mod aggregation;
mod context;
mod executor;
mod filter;
mod matching;
mod projection;
mod types;

pub use context::PgqContext;
pub use executor::PgqExecutor;
pub use types::{PgqRow, SqlValue, VariableBinding};

use std::sync::Arc;

use raisin_sql::ast::GraphTableQuery;
use raisin_storage::Storage;

use crate::physical_plan::executor::ExecutionError;

type Result<T> = std::result::Result<T, ExecutionError>;

/// Execute a GRAPH_TABLE query and return results as a stream
///
/// # Arguments
///
/// * `storage` - Storage backend
/// * `workspace_id` - Current workspace
/// * `tenant_id` - Tenant identifier
/// * `repo_id` - Repository identifier
/// * `branch` - Branch name
/// * `revision` - Optional revision (HLC timestamp) for point-in-time queries
/// * `query` - Parsed GraphTableQuery AST
///
/// # Example
///
/// ```ignore
/// use raisin_sql::ast::pgq_parser::parse_graph_table;
///
/// let query = parse_graph_table(r#"
///     MATCH (a:User)-[r:FOLLOWS]->(b:User)
///     COLUMNS (a.id, b.id AS friend)
/// "#).unwrap();
///
/// let results = execute_graph_table(
///     storage,
///     "workspace".into(),
///     "tenant".into(),
///     "repo".into(),
///     "main".into(),
///     None,
///     query,
/// ).await;
/// ```
pub async fn execute_graph_table<S: Storage + 'static>(
    storage: Arc<S>,
    workspace_id: String,
    tenant_id: String,
    repo_id: String,
    branch: String,
    revision: Option<raisin_hlc::HLC>,
    query: GraphTableQuery,
) -> Result<Vec<PgqRow>> {
    let context = PgqContext::new(workspace_id, tenant_id, repo_id, branch, revision);

    let executor = PgqExecutor::new(storage, context);
    executor.execute(query).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_compiles() {
        // Basic compilation test
        let _ = PgqContext::new(
            "ws".into(),
            "tenant".into(),
            "repo".into(),
            "main".into(),
            None,
        );
    }
}
