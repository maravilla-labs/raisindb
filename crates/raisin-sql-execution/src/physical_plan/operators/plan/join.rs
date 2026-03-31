//! Join operator documentation and helpers.
//!
//! This module documents the join variants of [`PhysicalPlan`].
//!
//! ## NestedLoopJoin
//!
//! For each row from the left input, iterates through all rows from the right
//! input and evaluates the join condition. This is the simplest join algorithm
//! and works for all join types and conditions, but has O(n*m) complexity.
//!
//! **Best for:**
//! - Small datasets
//! - Non-equality join conditions
//! - CROSS JOIN (no condition)
//!
//! ### Fields
//! - `left` / `right` - Input operators
//! - `join_type` - INNER, LEFT, RIGHT, FULL, or CROSS
//! - `condition` - Optional join condition (None for CROSS JOIN)
//!
//! ## HashJoin
//!
//! Builds a hash table from the right input using the join keys, then probes
//! it with rows from the left input. Much more efficient than nested loop join
//! for equality conditions, with O(n+m) complexity.
//!
//! **Best for:**
//! - Large datasets with equality join conditions
//! - When right side fits in memory
//!
//! **Limitations:**
//! - Only works for equality joins (`a.id = b.id`)
//! - Right side must fit in memory
//!
//! ### Fields
//! - `left` / `right` - Input operators
//! - `join_type` - INNER, LEFT, RIGHT, FULL (CROSS should use NestedLoopJoin)
//! - `left_keys` / `right_keys` - Expressions to use as join keys
//!
//! ## HashSemiJoin
//!
//! Returns rows from the left side where the key exists in the right side's
//! hash table (or doesn't exist, for anti-join). Used for IN/NOT IN subqueries.
//!
//! ### Algorithm
//! 1. **Build phase:** Hash all distinct values from the right input
//! 2. **Probe phase:** For each left row, check if key exists in hash set
//!    - Semi-join (IN): output row if key exists
//!    - Anti-join (NOT IN): output row if key doesn't exist
//!
//! Memory: O(distinct values in right side).
//! Complexity: O(n + m).
//!
//! ### Fields
//! - `left` - Rows to filter
//! - `right` - Values to check membership against
//! - `left_key` / `right_key` - Lookup key expressions
//! - `anti` - If true, NOT IN semantics; if false, IN semantics
//!
//! ## IndexLookupJoin
//!
//! For each row from the outer input, performs an O(1) index lookup on the
//! inner side using the join key. Optimal when joining on an indexed column
//! like `id`.
//!
//! **Best for:**
//! - Small outer input (e.g., CTE results from graph queries)
//! - Join key is an indexed column (id, path)
//! - Inner table is large (avoid full scan)
//!
//! Complexity: O(n) where n = outer rows (each lookup is O(1)).
//! Memory: O(1) - no hash table needed.
//!
//! ### Example
//! ```sql
//! WITH related AS (SELECT target_id FROM CYPHER(...))
//! SELECT n.* FROM workspace n JOIN related r ON n.id = r.target_id
//! ```
//! This becomes: `IndexLookupJoin(CTEScan -> NodeIdScan per row)`
//!
//! ### Fields
//! - `outer` - Outer input (smaller side, e.g., CTE results)
//! - `join_type` - INNER, LEFT supported; RIGHT/FULL not supported
//! - `outer_key_column` - Column name from outer side containing the lookup key
//! - `inner_lookup` - Index lookup parameters for the inner side

use super::PhysicalPlan;

impl PhysicalPlan {
    /// Returns true if this is a join operator.
    pub fn is_join(&self) -> bool {
        matches!(
            self,
            PhysicalPlan::NestedLoopJoin { .. }
                | PhysicalPlan::HashJoin { .. }
                | PhysicalPlan::HashSemiJoin { .. }
                | PhysicalPlan::IndexLookupJoin { .. }
        )
    }
}
