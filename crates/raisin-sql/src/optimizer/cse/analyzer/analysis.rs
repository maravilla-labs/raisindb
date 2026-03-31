//! Core analysis methods for identifying CSE candidates in logical plans
//!
//! This module contains the main entry points for analyzing projections, filters,
//! and aggregates to discover repeated subexpressions.

use super::cost;
use super::traversal;
use super::CseAnalyzer;
use super::CseCandidate;
use crate::analyzer::TypedExpr;
use crate::logical_plan::{LogicalPlan, ProjectionExpr};
use crate::optimizer::cse::arena::ExprId;
use crate::optimizer::cse::CseContext;
use std::collections::HashMap;

impl CseAnalyzer {
    /// Analyze a logical plan and return CSE candidates
    ///
    /// This analyzes projection expressions, filter predicates, join conditions,
    /// and aggregate expressions to find subexpressions that appear multiple times
    /// and would benefit from extraction.
    ///
    /// # Arguments
    ///
    /// * `ctx` - Mutable reference to the CSE context containing the arena and config
    /// * `plan` - The logical plan to analyze
    ///
    /// # Returns
    ///
    /// A vector of `CseCandidate` objects representing expressions to extract,
    /// ordered by frequency (most common first) for optimal extraction.
    pub fn analyze(ctx: &mut CseContext, plan: &LogicalPlan) -> Vec<CseCandidate> {
        match plan {
            LogicalPlan::Project { exprs, .. } => Self::analyze_projection(ctx, exprs),
            LogicalPlan::Filter { predicate, .. } => {
                Self::analyze_filter(ctx, &predicate.conjuncts)
            }
            LogicalPlan::Join { condition, .. } => {
                if let Some(cond) = condition {
                    // Join conditions are typically single expressions
                    Self::analyze_filter(ctx, std::slice::from_ref(cond))
                } else {
                    Vec::new()
                }
            }
            LogicalPlan::Aggregate {
                group_by,
                aggregates,
                ..
            } => Self::analyze_aggregate(ctx, group_by, aggregates),
            _ => Vec::new(),
        }
    }

    /// Analyze projection expressions to find common subexpressions
    fn analyze_projection(ctx: &mut CseContext, exprs: &[ProjectionExpr]) -> Vec<CseCandidate> {
        tracing::trace!(
            "🔍 CSE Analyzer: Analyzing {} projection expressions",
            exprs.len()
        );

        // Build frequency map: hash -> Vec<(expr_id, count)>
        let mut frequency_map: HashMap<u64, Vec<(ExprId, usize)>> = HashMap::new();

        // Collect all subexpressions from all projection expressions
        for (idx, proj_expr) in exprs.iter().enumerate() {
            tracing::trace!(
                "  [{}] Collecting subexpressions from '{}'",
                idx,
                proj_expr.alias
            );
            traversal::collect_subexpressions(&proj_expr.expr, &mut ctx.arena, &mut frequency_map);
        }

        tracing::trace!(
            "  Found {} unique expression hashes in frequency map",
            frequency_map.len()
        );

        // Filter candidates that meet threshold and convert to CseCandidate
        // Flatten the Vec to get all expressions
        let threshold = ctx.config.threshold;
        let mut candidates: Vec<_> = Vec::new();
        for (hash, entries) in &frequency_map {
            for (expr_id, count) in entries {
                let expr = ctx.arena.get(*expr_id);
                let extractable = cost::is_extractable(expr);
                let meets_threshold = *count >= threshold;
                tracing::trace!(
                    "    Hash {}: count={} threshold={} extractable={} meets_threshold={}",
                    hash,
                    count,
                    threshold,
                    extractable,
                    meets_threshold
                );
                if meets_threshold && extractable {
                    candidates.push((*expr_id, *count, *hash));
                    tracing::trace!("      ✓ Added as candidate");
                }
            }
        }

        tracing::trace!("  Pre-sort: {} candidates", candidates.len());

        // Sort by frequency (descending) for optimal extraction order
        candidates.sort_by(|a, b| b.1.cmp(&a.1));

        // Generate unique aliases and create CseCandidate objects
        let result: Vec<_> = candidates
            .into_iter()
            .enumerate()
            .map(|(idx, (expr_id, count, _hash))| {
                // Clone the expression for backward compatibility
                let expr = ctx.arena.get(expr_id).clone();
                let alias = format!("__cse_{}", idx);
                tracing::trace!("  Generated candidate: alias='{}' count={}", alias, count);
                CseCandidate {
                    expr_id,
                    expr,
                    count,
                    alias,
                }
            })
            .collect();

        tracing::trace!("🎯 CSE Analyzer: Returning {} candidates", result.len());
        result
    }

    /// Analyze filter predicates to find common subexpressions
    ///
    /// This analyzes WHERE clause and JOIN ON condition predicates for repeated
    /// subexpressions that could be materialized in an intermediate projection.
    fn analyze_filter(ctx: &mut CseContext, predicates: &[TypedExpr]) -> Vec<CseCandidate> {
        let mut frequency_map: HashMap<u64, Vec<(ExprId, usize)>> = HashMap::new();

        // Collect all subexpressions from all predicates
        for predicate in predicates {
            traversal::collect_subexpressions(predicate, &mut ctx.arena, &mut frequency_map);
        }

        // Filter candidates that meet threshold (flatten Vec)
        let threshold = ctx.config.threshold;
        let mut candidates: Vec<_> = Vec::new();
        for (hash, entries) in frequency_map {
            for (expr_id, count) in entries {
                let expr = ctx.arena.get(expr_id);
                if count >= threshold && cost::is_extractable(expr) {
                    candidates.push((expr_id, count, hash));
                }
            }
        }

        // Sort by frequency (descending)
        candidates.sort_by(|a, b| b.1.cmp(&a.1));

        // Generate candidates
        candidates
            .into_iter()
            .enumerate()
            .map(|(idx, (expr_id, count, _hash))| {
                let expr = ctx.arena.get(expr_id).clone();
                CseCandidate {
                    expr_id,
                    expr,
                    count,
                    alias: format!("__cse_{}", idx),
                }
            })
            .collect()
    }

    /// Analyze aggregate expressions to find common subexpressions
    ///
    /// This analyzes GROUP BY expressions and aggregate function arguments
    /// for repeated subexpressions.
    fn analyze_aggregate(
        ctx: &mut CseContext,
        group_by: &[TypedExpr],
        aggregates: &[crate::logical_plan::AggregateExpr],
    ) -> Vec<CseCandidate> {
        let mut frequency_map: HashMap<u64, Vec<(ExprId, usize)>> = HashMap::new();

        // Collect from GROUP BY expressions
        for expr in group_by {
            traversal::collect_subexpressions(expr, &mut ctx.arena, &mut frequency_map);
        }

        // Collect from aggregate function arguments and filters
        for agg in aggregates {
            for arg in &agg.args {
                traversal::collect_subexpressions(arg, &mut ctx.arena, &mut frequency_map);
            }
            if let Some(filter) = &agg.filter {
                traversal::collect_subexpressions(filter, &mut ctx.arena, &mut frequency_map);
            }
        }

        // Filter candidates that meet threshold (flatten Vec)
        let threshold = ctx.config.threshold;
        let mut candidates: Vec<_> = Vec::new();
        for (hash, entries) in frequency_map {
            for (expr_id, count) in entries {
                let expr = ctx.arena.get(expr_id);
                if count >= threshold && cost::is_extractable(expr) {
                    candidates.push((expr_id, count, hash));
                }
            }
        }

        // Sort by frequency (descending)
        candidates.sort_by(|a, b| b.1.cmp(&a.1));

        // Generate candidates
        candidates
            .into_iter()
            .enumerate()
            .map(|(idx, (expr_id, count, _hash))| {
                let expr = ctx.arena.get(expr_id).clone();
                CseCandidate {
                    expr_id,
                    expr,
                    count,
                    alias: format!("__cse_{}", idx),
                }
            })
            .collect()
    }
}
