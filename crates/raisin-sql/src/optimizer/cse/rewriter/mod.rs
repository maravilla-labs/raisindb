//! Plan rewriting for CSE - transforms plans to eliminate common subexpressions
//!
//! This module rewrites logical plans to extract common subexpressions into
//! intermediate projections, avoiding redundant computation.
//!
//! # Submodules
//!
//! - **expr_replacement** - Recursive expression tree rewriting with CSE column references
//! - **helpers** - Utility functions (table qualifier resolution)

mod expr_replacement;
mod helpers;

#[cfg(test)]
mod tests;

use super::analyzer::CseCandidate;
use super::hasher::ExprHasher;
use crate::analyzer::TypedExpr;
use crate::logical_plan::{LogicalPlan, ProjectionExpr};
use helpers::get_table_qualifier;
use std::collections::HashMap;

/// Rewrites logical plans to use common subexpression elimination
pub struct CsePlanRewriter;

impl CsePlanRewriter {
    /// Rewrite a plan to use common subexpression elimination
    ///
    /// Transforms:
    /// ```text
    /// Project([expr1_using_common, expr2_using_common, expr3_using_common])
    /// ```
    ///
    /// Into:
    /// ```text
    /// Project([Column(cse_0), Column(cse_1), Column(cse_2)])
    ///   -> Project([common_expr AS cse_0])
    ///        -> Original Input
    /// ```
    ///
    /// # Arguments
    ///
    /// * `plan` - The logical plan to rewrite (must be a Project node)
    /// * `candidates` - CSE candidates identified by the analyzer
    ///
    /// # Returns
    ///
    /// The rewritten plan with intermediate projections for common subexpressions.
    pub fn rewrite(plan: LogicalPlan, candidates: Vec<CseCandidate>) -> LogicalPlan {
        if candidates.is_empty() {
            return plan;
        }

        tracing::trace!("CSE Rewriter: Processing {} candidates", candidates.len());

        // Only rewrite Project nodes
        if let LogicalPlan::Project { input, exprs } = plan {
            tracing::trace!("Original projection has {} expressions", exprs.len());
            for (idx, expr) in exprs.iter().enumerate() {
                tracing::trace!(
                    "  [{}] alias='{}' expr_type={:?}",
                    idx,
                    expr.alias,
                    std::mem::discriminant(&expr.expr.expr)
                );
            }

            // Build replacement map: expr_hash -> cse_alias
            let replacement_map = Self::build_replacement_map(&candidates);

            // Step 1: Create intermediate projection with extracted CSE expressions
            // and pass-through for non-CSE top-level expressions
            let (intermediate_exprs, passthrough_aliases) =
                Self::build_intermediate_projection(&candidates, &exprs, &replacement_map);

            tracing::trace!(
                "Intermediate projection has {} expressions:",
                intermediate_exprs.len()
            );
            for (idx, expr) in intermediate_exprs.iter().enumerate() {
                tracing::trace!("  [{}] '{}'", idx, expr.alias);
            }

            let intermediate_project = LogicalPlan::Project {
                input,
                exprs: intermediate_exprs,
            };

            // Step 2: Rewrite original projection expressions to reference extracted columns
            let table_qualifier = get_table_qualifier(&intermediate_project);
            let rewritten_exprs: Vec<ProjectionExpr> = exprs
                .into_iter()
                .enumerate()
                .map(|(idx, proj)| {
                    tracing::trace!("  Rewriting final projection [{}] '{}'", idx, proj.alias);

                    let rewritten_expr = if passthrough_aliases.contains(&proj.alias) {
                        tracing::trace!("    Pass-through: replacing with column reference");
                        TypedExpr::column(
                            table_qualifier.clone(),
                            proj.alias.clone(),
                            proj.expr.data_type.clone(),
                        )
                    } else {
                        expr_replacement::replace_common_subexpressions(
                            proj.expr,
                            &replacement_map,
                            &table_qualifier,
                        )
                    };

                    tracing::trace!(
                        "    Result: expr_type={:?}",
                        std::mem::discriminant(&rewritten_expr.expr)
                    );
                    ProjectionExpr {
                        expr: rewritten_expr,
                        alias: proj.alias.clone(),
                    }
                })
                .collect();

            tracing::trace!(
                "Final projection has {} expressions:",
                rewritten_exprs.len()
            );
            for (idx, expr) in rewritten_exprs.iter().enumerate() {
                tracing::trace!(
                    "  [{}] alias='{}' expr_type={:?}",
                    idx,
                    expr.alias,
                    std::mem::discriminant(&expr.expr.expr)
                );
            }

            // Step 3: Create final projection that references the intermediate projection
            LogicalPlan::Project {
                input: Box::new(intermediate_project),
                exprs: rewritten_exprs,
            }
        } else {
            // Not a Project node, return unchanged
            plan
        }
    }

    /// Inject a Project node with CSE candidates between a plan node and its input
    ///
    /// This is used when applying CSE to non-Project nodes (Filter, Join, Aggregate).
    /// We cannot modify their expressions in-place; instead, we must inject a new
    /// Project node that materializes the common subexpressions.
    ///
    /// # Arguments
    ///
    /// * `input` - The input plan (will become the input to the new Project)
    /// * `candidates` - CSE candidates to materialize in the injected Project
    ///
    /// # Returns
    ///
    /// A new Project node with the candidates as projection expressions
    pub fn inject_projection(input: LogicalPlan, candidates: &[CseCandidate]) -> LogicalPlan {
        if candidates.is_empty() {
            return input;
        }

        let intermediate_exprs: Vec<ProjectionExpr> = candidates
            .iter()
            .map(|candidate| ProjectionExpr {
                expr: candidate.expr.clone(),
                alias: candidate.alias.clone(),
            })
            .collect();

        LogicalPlan::Project {
            input: Box::new(input),
            exprs: intermediate_exprs,
        }
    }

    /// Replace common subexpressions in an expression with column references
    ///
    /// This is a public wrapper around the recursive replacement function,
    /// useful when rewriting expressions for Filter/Join/Aggregate nodes.
    pub fn replace_with_cse_columns(
        expr: TypedExpr,
        candidates: &[CseCandidate],
        table_qualifier: &str,
    ) -> TypedExpr {
        let mut replacement_map = HashMap::new();
        for candidate in candidates {
            let hash = ExprHasher::hash_expr(&candidate.expr);
            replacement_map.insert(hash, candidate.alias.clone());
        }

        expr_replacement::replace_common_subexpressions(expr, &replacement_map, table_qualifier)
    }

    /// Build the replacement map from expression hash to CSE alias
    fn build_replacement_map(candidates: &[CseCandidate]) -> HashMap<u64, String> {
        let mut replacement_map = HashMap::new();
        for candidate in candidates {
            let hash = ExprHasher::hash_expr(&candidate.expr);
            tracing::trace!(
                "  CSE Candidate: alias='{}' hash={} count={}",
                candidate.alias,
                hash,
                candidate.count
            );
            replacement_map.insert(hash, candidate.alias.clone());
        }
        replacement_map
    }

    /// Build the intermediate projection expressions and track pass-through aliases
    ///
    /// Returns a tuple of (intermediate_exprs, passthrough_aliases).
    fn build_intermediate_projection(
        candidates: &[CseCandidate],
        exprs: &[ProjectionExpr],
        replacement_map: &HashMap<u64, String>,
    ) -> (Vec<ProjectionExpr>, std::collections::HashSet<String>) {
        let mut intermediate_exprs = Vec::new();
        let mut passthrough_aliases = std::collections::HashSet::new();

        // Add all CSE candidates first
        for candidate in candidates {
            tracing::trace!(
                "  Adding CSE candidate to intermediate: '{}'",
                candidate.alias
            );
            intermediate_exprs.push(ProjectionExpr {
                expr: candidate.expr.clone(),
                alias: candidate.alias.clone(),
            });
        }

        // Add pass-through columns for non-CSE top-level expressions
        for proj_expr in exprs {
            let expr_hash = ExprHasher::hash_expr(&proj_expr.expr);
            let in_map = replacement_map.contains_key(&expr_hash);
            tracing::trace!(
                "  Checking pass-through for '{}': hash={} in_replacement_map={}",
                proj_expr.alias,
                expr_hash,
                in_map
            );
            if !in_map {
                tracing::trace!("    Adding as pass-through: '{}'", proj_expr.alias);
                intermediate_exprs.push(proj_expr.clone());
                passthrough_aliases.insert(proj_expr.alias.clone());
            } else {
                tracing::trace!("    Skipping (CSE candidate): '{}'", proj_expr.alias);
            }
        }

        (intermediate_exprs, passthrough_aliases)
    }
}
