//! CSE analysis - identifies common subexpression candidates
//!
//! This module analyzes projection expressions to identify expressions that appear
//! multiple times and would benefit from extraction and reuse.
//!
//! # Submodules
//!
//! - **analysis**: Core analysis methods for projection, filter, and aggregate plans
//! - **traversal**: Expression tree traversal and subexpression collection
//! - **cost**: Cost estimation, volatility detection, and extractability checks

mod analysis;
mod cost;
mod traversal;

#[cfg(test)]
mod tests;

use super::arena::ExprId;
use crate::analyzer::TypedExpr;

/// A candidate for common subexpression elimination
#[derive(Debug, Clone)]
pub struct CseCandidate {
    /// The expression ID in the arena
    pub expr_id: ExprId,
    /// The expression to extract (for backward compatibility)
    pub expr: TypedExpr,
    /// Number of times this expression appears
    pub count: usize,
    /// Generated alias for the extracted expression (e.g., "__cse_0")
    pub alias: String,
}

/// Analyzes logical plans to identify CSE opportunities
pub struct CseAnalyzer;
