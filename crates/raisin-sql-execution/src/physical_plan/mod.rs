//! Physical Plan and Execution Engine
//!
//! This module implements the physical planning and execution layer for RaisinSQL.
//! It converts logical plans into executable physical plans and provides streaming
//! execution against RocksDB storage and Tantivy full-text indexes.
//!
//! # Architecture
//!
//! The physical execution layer consists of several components:
//!
//! 1. **Physical Operators** (`operators.rs`) - Physical plan nodes representing concrete execution strategies
//! 2. **Physical Planner** (`planner.rs`) - Converts logical plans to physical plans with scan selection
//! 3. **Executor** (`executor.rs`) - Execution context and streaming execution engine
//! 4. **Scan Executors** (`scan_executors.rs`) - Physical scans (Table, Prefix, Property, FullText)
//! 5. **Expression Evaluator** (`eval.rs`) - Runtime expression evaluation
//! 6. **Operators** - Filter, Project, Sort, Limit implementations
//! 7. **Type System** (`types.rs`) - Type conversion between SQL types and PropertyValue
//!
//! # Execution Model
//!
//! The execution engine uses async streaming with the Volcano-style iterator model:
//! - Each operator produces a stream of rows
//! - Operators pull from their inputs on demand
//! - Backpressure is naturally handled by async streams
//!
//! # Example
//!
//! ```rust,ignore
//! use raisin_sql::{QueryPlan, PhysicalPlanner};
//! use raisin_sql::physical_plan::{ExecutionContext, execute_plan};
//!
//! // Create query plan (parse + analyze + optimize)
//! let query_plan = QueryPlan::from_sql("SELECT id, name FROM nodes WHERE depth = 2 LIMIT 10")?;
//!
//! // Convert to physical plan
//! let physical_planner = PhysicalPlanner::new();
//! let physical_plan = physical_planner.plan(&query_plan.optimized)?;
//!
//! // Execute
//! let ctx = ExecutionContext::new(storage, tenant_id, repo_id, branch, workspace);
//! let mut stream = execute_plan(&physical_plan, &ctx).await?;
//!
//! while let Some(row) = stream.next().await {
//!     println!("{:?}", row?);
//! }
//! ```

pub mod batch;
pub mod batch_execution;
pub mod catalog;
pub mod cte_storage;
pub mod cypher;
pub mod ddl_executor;
pub mod distinct;
pub mod dml_executor;
pub mod eval;
pub mod executor;
pub mod filter;
pub mod fulltext;
pub mod hash_aggregate;
pub mod hash_join;
pub mod index_lookup_join;
pub mod lateral_map;
pub mod limit;
pub mod nested_loop_join;
pub mod operators;
pub mod pg_catalog_executor;
pub mod pgq;
pub mod planner;
pub mod project;
pub mod scan_executors;
pub mod semi_join;
pub mod sort;
pub mod table_function;
pub mod types;
pub mod window;

#[cfg(test)]
mod tests;

// Re-export commonly used types

// Batch data structures and configuration
pub use batch::{Batch, BatchConfig, BatchIterator, ColumnArray};

// Batch execution types (from batch_execution module)
pub use batch_execution::{
    convert_batch_stream_to_row_stream, convert_row_stream_to_batch_stream, BatchExecutionConfig,
    BatchStream, RowAccumulator,
};

// Other re-exports
pub use catalog::{IndexCatalog, RocksDBIndexCatalog};
pub use cte_storage::{CTEConfig, CTEIterator, MaterializedCTE};
pub use executor::{
    execute_plan, execute_plan_batch, ExecutionContext, ExecutionError, Row, RowStream,
};
pub use operators::{IndexLookupParams, IndexLookupType, PhysicalPlan, VectorDistanceMetric};
pub use planner::PhysicalPlanner;
pub use types::{from_property_value, to_property_value};

// DML executor types for filter classification
pub use dml_executor::{classify_filter, FilterComplexity, NodeIdentifier};
