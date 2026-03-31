//! Relational operator documentation and helpers.
//!
//! This module documents the relational algebra variants of [`PhysicalPlan`].
//!
//! ## Filter
//!
//! Evaluates filter predicates against each input row. Multiple predicates
//! are AND-ed together (CNF form). This is a streaming operator.
//!
//! ## Project
//!
//! Computes projection expressions and outputs selected columns. This can
//! include column references, computed expressions (DEPTH(path), JSON
//! operators, etc.), and function calls.
//!
//! ## Sort
//!
//! Sorts rows by one or more expressions. This is a **blocking** operator
//! -- it must consume all input before producing any output. Uses in-memory
//! sorting.
//!
//! ## TopN
//!
//! Optimized sort with limit using a heap. More efficient than Sort + Limit
//! because it only materializes the top N rows instead of sorting the entire
//! result set. Uses a min-heap (for ascending) or max-heap (for descending).
//!
//! Generated when the planner detects: `ORDER BY ... LIMIT N` (offset = 0).
//!
//! ## Limit
//!
//! Returns at most `limit` rows, skipping the first `offset` rows. This is
//! a streaming operator.
//!
//! ## HashAggregate
//!
//! Groups rows by grouping expressions and computes aggregate functions.
//! Uses HashMap for efficient grouping.
//!
//! ### Algorithm
//! 1. For each input row, evaluate `group_by` expressions to get group key
//! 2. Update accumulators for this group
//! 3. Emit one row per group with aggregated values
//!
//! Memory usage: O(number of groups).
//!
//! ## WithCTE
//!
//! Materializes all CTEs first, storing their results in memory (or spilling
//! to disk if they exceed memory limits). The main query can then reference
//! these materialized CTEs.
//!
//! ### Execution Strategy
//! 1. Execute each CTE plan in order
//! 2. Materialize results with automatic disk spillage
//! 3. Store in execution context's CTE storage
//! 4. Execute main query with CTE results available
//!
//! ### Memory Management
//! - Small CTEs (< 10MB): Kept fully in memory
//! - Large CTEs (> 10MB): Automatically spill to temporary files
//! - Temp files cleaned up when execution context is dropped
//!
//! ## Window
//!
//! Computes window functions over partitions with optional ordering and
//! framing. Buffers all input rows, partitions them, sorts within partitions,
//! and computes window functions for each row.
//!
//! ## Distinct
//!
//! Removes duplicate rows using hash-based deduplication.
//!
//! For basic DISTINCT (`on_columns` is empty): All columns determine uniqueness.
//! For DISTINCT ON (`on_columns` is specified): Only specified columns determine
//! uniqueness, keeping the first row for each distinct key combination.
//!
//! NULL handling: Per SQL standard, NULL = NULL for distinctness (unlike equality).
//!
//! ## Empty
//!
//! Empty plan for DDL statements that bypass physical execution. DDL statements
//! are executed directly in the engine, not through physical plans.

use super::PhysicalPlan;

impl PhysicalPlan {
    /// Returns true if this is a relational algebra operator
    /// (Filter, Project, Sort, TopN, Limit, HashAggregate, WithCTE,
    /// Window, Distinct, or Empty).
    pub fn is_relational(&self) -> bool {
        matches!(
            self,
            PhysicalPlan::Filter { .. }
                | PhysicalPlan::Project { .. }
                | PhysicalPlan::Sort { .. }
                | PhysicalPlan::TopN { .. }
                | PhysicalPlan::Limit { .. }
                | PhysicalPlan::HashAggregate { .. }
                | PhysicalPlan::WithCTE { .. }
                | PhysicalPlan::Window { .. }
                | PhysicalPlan::Distinct { .. }
                | PhysicalPlan::Empty
        )
    }
}
