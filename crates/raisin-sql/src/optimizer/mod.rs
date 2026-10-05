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
//! ```no_run
//! use raisin_sql::optimizer::Optimizer;
//! use raisin_sql::logical_plan::LogicalPlan;
//!
//! # fn example(original_plan: LogicalPlan) {
//! let optimizer = Optimizer::new();
//! let optimized_plan = optimizer.optimize(original_plan);
//! # }
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

    /// Apply all optimization passes to a logical plan.
    ///
    /// ONE round of the passes (folding, hierarchy rewriting, CSE, pruning)
    /// reaches the fixpoint: each pass is idempotent, and no pass creates work
    /// for one that ran before it in the round — pruning adds only
    /// pass-through columns, CSE only column references and projections, the
    /// hierarchy rewrite only canonical shapes that fold nothing and rewrite to
    /// themselves. The loop used to run a second, confirming round on every
    /// statement (plus a plan clone and a structural comparison), which was
    /// about half of the optimizer's cost on a never-seen SQL text (plan
    /// Phase 13d).
    ///
    /// Debug builds still run the confirming round and fail loudly when a
    /// round changed anything (see [`verify_fixpoint`]), so every SQL test in
    /// the workspace re-proves the claim; `RAISIN_SQL_OPTIMIZER_VERIFY=1`
    /// turns the same check on in a release build (logging instead of
    /// panicking), and with it the old loop up to `max_passes`.
    pub fn optimize(&self, plan: LogicalPlan) -> LogicalPlan {
        let mut current = self.round(plan);
        if !verify_fixpoint() {
            return current;
        }
        let mut rounds = 1;
        while rounds < self.config.max_passes {
            let next = self.round(current.clone());
            if same_plan(&next, &current) {
                break;
            }
            NON_FIXPOINT_ROUNDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if cfg!(debug_assertions) && strict_fixpoint() {
                panic!(
                    "optimizer: one round did not reach the fixpoint (plan Phase 13d); \
                     round {} changed\nbefore: {:?}\nafter: {:?}",
                    rounds + 1,
                    current,
                    next
                );
            }
            tracing::error!(
                "optimizer: round {} changed the plan; one round is not a fixpoint for it",
                rounds + 1
            );
            current = next;
            rounds += 1;
        }
        current
    }

    /// The passes that read literal VALUES — constant folding and the
    /// hierarchy rewrite — over a plan whose parameters were just bound
    /// (`crate::template`, plan Phase 13d). A template is optimized with
    /// opaque placeholders; these two passes are what its literals would have
    /// changed, and both are idempotent, so re-running them on the bound plan
    /// gives what optimizing the substituted text gives. CSE and pruning read
    /// structure and column references only, which binding does not change.
    pub fn rebind(&self, plan: LogicalPlan) -> LogicalPlan {
        let mut current = plan;
        if self.config.enable_constant_folding {
            current = self.apply_constant_folding(current);
        }
        if self.config.enable_hierarchy_rewriting {
            current = self.apply_hierarchy_rewriting(current);
        }
        current
    }

    /// One round of every enabled pass.
    fn round(&self, plan: LogicalPlan) -> LogicalPlan {
        let mut current = plan;
        // Pass 1: Constant folding (applied to expressions in the plan)
        if self.config.enable_constant_folding {
            current = self.apply_constant_folding(current);
        }
        // Pass 2: Hierarchy rewriting (applied to filter predicates)
        if self.config.enable_hierarchy_rewriting {
            current = self.apply_hierarchy_rewriting(current);
        }
        // Pass 3: Common Subexpression Elimination (CSE), after folding to
        // maximize opportunities
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
        current
    }
}

/// Rounds that changed a plan after the first one — always 0 unless a pass
/// lost its idempotence. Counted only while [`verify_fixpoint`] is on.
static NON_FIXPOINT_ROUNDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Rounds after the first that changed a plan, since the process started
/// (only counted while the fixpoint is verified) — for tests.
#[doc(hidden)]
pub fn non_fixpoint_rounds() -> u64 {
    NON_FIXPOINT_ROUNDS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Whether to run the confirming round: always in a debug build, otherwise
/// only with `RAISIN_SQL_OPTIMIZER_VERIFY=1`.
fn verify_fixpoint() -> bool {
    static VERIFY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *VERIFY.get_or_init(|| {
        cfg!(debug_assertions)
            || std::env::var("RAISIN_SQL_OPTIMIZER_VERIFY")
                .map(|v| matches!(v.trim(), "1" | "true" | "on" | "yes"))
                .unwrap_or(false)
    })
}

/// A debug build panics on a non-fixpoint round unless
/// `RAISIN_SQL_OPTIMIZER_VERIFY=log` asks it only to log.
fn strict_fixpoint() -> bool {
    std::env::var("RAISIN_SQL_OPTIMIZER_VERIFY")
        .map(|v| v.trim() != "log")
        .unwrap_or(true)
}

/// Structural equality, with a `Debug` fallback so a NaN literal (never
/// equal to itself) does not count as a change.
fn same_plan(a: &LogicalPlan, b: &LogicalPlan) -> bool {
    a == b || format!("{a:?}") == format!("{b:?}")
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
