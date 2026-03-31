//! Basic scan operator documentation and helpers.
//!
//! This module documents the basic scan variants of [`PhysicalPlan`]:
//!
//! ## TableScan
//!
//! Full table scan with optional filter pushdown. Scans all nodes of a
//! specific type (or all nodes if no filter). This is the fallback scan
//! when no better access method is available.
//!
//! ### Fields
//! - `tenant_id` - Tenant identifier
//! - `repo_id` - Repository identifier
//! - `branch` - Branch name
//! - `workspace` - Workspace identifier
//! - `table` - Table name (typically "nodes")
//! - `alias` - Optional table alias (used for column qualification)
//! - `schema` - Schema for the table
//! - `filter` - Optional pushed-down filter (for storage-level filtering)
//! - `projection` - Optional column projection (only read these columns)
//! - `limit` - Optional limit hint for early termination
//! - `reason` - Reason why TableScan was chosen instead of index scan
//!
//! ## CountScan
//!
//! Optimized count scan for `COUNT(*)` queries. Counts nodes without
//! deserializing node data. This is a specialized operator for `COUNT(*)`
//! with no `GROUP BY`. It is 10-100x faster than TableScan + HashAggregate
//! for counting, using only O(1) memory.
//!
//! ### Example Query
//! ```sql
//! SELECT COUNT(*) FROM workspace
//! ```
//!
//! ### Fields
//! - `tenant_id` - Tenant identifier
//! - `repo_id` - Repository identifier
//! - `branch` - Branch name
//! - `workspace` - Workspace identifier
//! - `max_revision` - Maximum revision to count (for point-in-time queries)
//!
//! ## TableFunction
//!
//! Table-valued function invocation. Executes a registered table function
//! and returns its result set.
//!
//! ### Fields
//! - `name` - Function name
//! - `alias` - Optional table alias
//! - `args` - Function arguments
//! - `schema` - Output schema reported by the analyzer
//! - `workspace` - Optional workspace override
//! - `branch_override` - Optional branch override
//! - `max_revision` - Maximum revision to read (for time-travel)
//!
//! ## PrefixScan
//!
//! Prefix scan on path hierarchy. Uses the `path_index` column family to
//! efficiently scan nodes with a specific path prefix. This is optimal for
//! queries like: `WHERE PATH_STARTS_WITH(path, '/content/blog/')`
//!
//! ### Fields
//! - `tenant_id` - Tenant identifier
//! - `repo_id` - Repository identifier
//! - `branch` - Branch name
//! - `workspace` - Workspace identifier
//! - `table` - Base table name (typically "nodes")
//! - `alias` - Optional table alias (used for column qualification)
//! - `path_prefix` - Path prefix to scan (e.g., "/content/blog/")
//! - `projection` - Optional column projection
//! - `direct_children_only` - If true, only scan direct children (optimized
//!   for PARENT queries). If false, scan all descendants at any depth.
//! - `limit` - Optional limit hint for early termination

use super::PhysicalPlan;

impl PhysicalPlan {
    /// Returns true if this is a basic scan operator (TableScan, CountScan,
    /// TableFunction, or PrefixScan).
    pub fn is_basic_scan(&self) -> bool {
        matches!(
            self,
            PhysicalPlan::TableScan { .. }
                | PhysicalPlan::CountScan { .. }
                | PhysicalPlan::TableFunction { .. }
                | PhysicalPlan::PrefixScan { .. }
        )
    }
}
