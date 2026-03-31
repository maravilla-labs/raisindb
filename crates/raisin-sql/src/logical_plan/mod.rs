//! Logical query planning
//!
//! Converts analyzed SQL statements into logical operator trees.
//!
//! # Overview
//!
//! The logical planner transforms semantically analyzed queries into a tree of relational
//! algebra operators. This intermediate representation:
//!
//! - Maintains type information from the analyzer
//! - Provides a clean abstraction for optimization
//! - Can be traversed and transformed using visitor patterns
//! - Produces human-readable explanations for debugging
//!
//! # Architecture
//!
//! ```text
//! AnalyzedStatement → PlanBuilder → LogicalPlan → Optimizer (Phase 4)
//! ```
//!
//! # Example
//!
//! ```
//! use raisin_sql::{Analyzer, PlanBuilder};
//! use raisin_sql::analyzer::StaticCatalog;
//!
//! let catalog = StaticCatalog::default_nodes_schema();
//! let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
//! let planner = PlanBuilder::new(&catalog);
//!
//! // Analyze and plan a query
//! let analyzed = analyzer.analyze("SELECT id, name FROM nodes WHERE id = 'test'").unwrap();
//! let plan = planner.build(&analyzed).unwrap();
//!
//! // Print the plan
//! println!("{}", plan.explain());
//! // Output:
//! // Project: [Column { table: "nodes", column: "id" } AS id, Column { table: "nodes", column: "name" } AS name]
//! //   Filter: BinaryOp { left: Column { table: "nodes", column: "id" }, op: Eq, right: Literal(Text("test")) }
//! //     Scan: nodes
//! ```
//!
//! # Operators
//!
//! The logical plan consists of these relational operators:
//!
//! - **Scan**: Read from a table
//! - **Filter**: Apply a predicate (WHERE clause)
//! - **Project**: Select columns and expressions (SELECT list)
//! - **Sort**: Order rows (ORDER BY clause)
//! - **Limit**: Restrict number of rows (LIMIT/OFFSET)
//! - **Aggregate**: Group and aggregate (GROUP BY - future)

pub mod builder;
pub mod display;
pub mod error;
pub mod operators;
pub mod visitor;

// Re-export commonly used types
pub use builder::PlanBuilder;
pub use error::{PlanError, Result};
pub use operators::{
    AggregateExpr, AggregateFunction, DistinctSpec, FilterPredicate, LogicalPlan, ProjectionExpr,
    SchemaColumn, SortExpr, TableSchema, WindowExpr,
};
pub use visitor::{PlanRewriter, PlanVisitor};

#[cfg(test)]
mod tests;
