//! Query Optimizer Module
//!
//! Applies rule-based and cost-based optimizations to logical query plans.
//!
//! # Optimization Passes
//!
//! The optimizer applies optimizations in the following order:
//!
//! 1. **Constant Folding** - Evaluate deterministic functions with constant arguments
//!    - `DEPTH('/content/')` → `1`
//!    - `1 + 2` → `3`
//!
//! 2. **Hierarchy Rewriting** - Transform hierarchy functions to canonical predicates
//!    - `PATH_STARTS_WITH(path, '/x/')` → PrefixRange (uses RocksDB prefix scan)
//!    - `PARENT(path) = '/x'` → PrefixRange + DepthEq
//!
//! 3. **Common Subexpression Elimination (CSE)** - Extract repeated expressions
//!    - `author.properties ->> 'username'` (repeated) → Extract to intermediate projection
//!    - Reduces redundant computation in SELECT lists
//!
//! 4. **Projection Pruning** - Compute minimal column set and push to Scan
//!    - Includes columns from SELECT, WHERE, ORDER BY
//!    - Reduces I/O by only reading needed columns
//!
//! # Usage
//!
//! ```
//! use raisin_sql::optimizer::Optimizer;
//! use raisin_sql::logical_plan::LogicalPlan;
//!
//! let optimizer = Optimizer::new();
//! let optimized_plan = optimizer.optimize(original_plan);
//! ```
//!
//! # Future Enhancements
//!
//! - Predicate pushdown into Scan operators
//! - Join reordering (when joins are supported)
//! - Cost-based optimization with statistics
//! - Index selection hints

pub mod cnf;
pub mod constant_fold;
pub mod cse;
pub mod hierarchy_rewrite;
mod passes;
pub mod projection;

#[cfg(test)]
mod tests;

use crate::logical_plan::{FilterPredicate, LogicalPlan};
use hierarchy_rewrite::{rewrite_hierarchy_predicates, CanonicalPredicate};
use projection::apply_projection_pruning;

/// Query optimizer configuration
#[derive(Debug, Clone)]
pub struct OptimizerConfig {
    /// Enable constant folding optimization
    pub enable_constant_folding: bool,

    /// Enable hierarchy function rewriting
    pub enable_hierarchy_rewriting: bool,

    /// Enable common subexpression elimination (CSE)
    pub enable_cse: bool,

    /// CSE threshold - minimum occurrences to extract an expression
    pub cse_threshold: usize,

    /// Enable projection pruning
    pub enable_projection_pruning: bool,

    /// Maximum optimization passes (to prevent infinite loops)
    pub max_passes: usize,
}

impl Default for OptimizerConfig {
    fn default() -> Self {
        Self {
            enable_constant_folding: true,
            enable_hierarchy_rewriting: true,
            enable_cse: true,
            cse_threshold: 2,
            enable_projection_pruning: true,
            max_passes: 10,
        }
    }
}

/// Query optimizer
pub struct Optimizer {
    config: OptimizerConfig,
}

impl Optimizer {
    /// Create optimizer with default configuration
    pub fn new() -> Self {
        Self {
            config: OptimizerConfig::default(),
        }
    }

    /// Create optimizer with custom configuration
    pub fn with_config(config: OptimizerConfig) -> Self {
        Self { config }
    }

    /// Apply all optimization passes to a logical plan
    ///
    /// Returns the optimized plan. The optimizer may apply multiple passes
    /// until the plan stabilizes or max_passes is reached.
    pub fn optimize(&self, plan: LogicalPlan) -> LogicalPlan {
        let mut current = plan;
        let mut pass_count = 0;

        // Apply optimization passes
        while pass_count < self.config.max_passes {
            let previous = format!("{:?}", current); // Poor man's change detection

            // Pass 1: Constant folding (applied to expressions in the plan)
            if self.config.enable_constant_folding {
                current = self.apply_constant_folding(current);
            }

            // Pass 2: Hierarchy rewriting (applied to filter predicates)
            if self.config.enable_hierarchy_rewriting {
                current = self.apply_hierarchy_rewriting(current);
            }

            // Pass 3: Common Subexpression Elimination (CSE)
            // Applied after constant folding to maximize opportunities
            if self.config.enable_cse {
                let cse_config = cse::CseConfig {
                    threshold: self.config.cse_threshold,
                };
                current = cse::apply_cse_recursive(current, &cse_config);
            }

            // Pass 4: Projection pruning (applied last, after other optimizations)
            if self.config.enable_projection_pruning {
                current = apply_projection_pruning(current);
            }

            // Check if plan changed
            let current_repr = format!("{:?}", current);
            if current_repr == previous {
                // Plan stabilized
                break;
            }

            pass_count += 1;
        }

        current
    }
}

impl Default for Optimizer {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract canonical predicates from a filter for execution planning
///
/// This is a utility function that can be used by the physical planner
/// to extract optimized predicates for RocksDB execution.
pub fn extract_canonical_predicates(predicate: &FilterPredicate) -> Vec<CanonicalPredicate> {
    let mut result = Vec::new();

    for conjunct in &predicate.conjuncts {
        let canonical = rewrite_hierarchy_predicates(conjunct.clone());
        result.extend(canonical);
    }

    result
}
