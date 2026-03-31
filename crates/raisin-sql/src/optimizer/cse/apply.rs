//! Core CSE application logic

use crate::logical_plan::{AggregateExpr, FilterPredicate, LogicalPlan};

use super::analyzer::CseAnalyzer;
use super::config::{CseConfig, CseContext};
use super::rewriter::CsePlanRewriter;

/// Apply common subexpression elimination to a logical plan
///
/// This is the main entry point for CSE optimization. It analyzes the plan
/// for repeated expressions and rewrites it to eliminate redundant computation.
///
/// # Arguments
///
/// * `plan` - The logical plan to optimize
/// * `config` - CSE configuration (threshold, etc.)
///
/// # Returns
///
/// An optimized plan with common subexpressions extracted into intermediate
/// projections, or the original plan if no optimization opportunities were found.
pub fn apply_cse(plan: LogicalPlan, config: &CseConfig) -> LogicalPlan {
    // Step 1: Create CSE context with arena
    let mut ctx = CseContext::new(config.clone());

    // Step 2: Analyze plan for CSE opportunities
    let candidates = CseAnalyzer::analyze(&mut ctx, &plan);

    // Step 3: If no candidates found, return original plan
    if candidates.is_empty() {
        return plan;
    }

    // Step 4: Rewrite plan based on node type
    match plan {
        LogicalPlan::Project { .. } => CsePlanRewriter::rewrite(plan, candidates),

        LogicalPlan::Filter { input, predicate } => {
            // Inject Project node with CSE candidates
            let table_qualifier = get_table_qualifier(&input);
            let new_input = CsePlanRewriter::inject_projection(*input, &candidates);

            // Replace common subexpressions in each conjunct
            let new_conjuncts: Vec<_> = predicate
                .conjuncts
                .into_iter()
                .map(|conj| {
                    CsePlanRewriter::replace_with_cse_columns(conj, &candidates, &table_qualifier)
                })
                .collect();

            LogicalPlan::Filter {
                input: Box::new(new_input),
                predicate: FilterPredicate {
                    conjuncts: new_conjuncts,
                    canonical: predicate.canonical,
                },
            }
        }

        LogicalPlan::Join {
            left,
            right,
            join_type,
            condition,
        } => {
            if let Some(cond) = condition {
                // Inject Project node on left input
                let table_qualifier = get_table_qualifier(&left);
                let new_left = CsePlanRewriter::inject_projection(*left, &candidates);

                // Replace common subexpressions in condition
                let new_condition =
                    CsePlanRewriter::replace_with_cse_columns(cond, &candidates, &table_qualifier);

                LogicalPlan::Join {
                    left: Box::new(new_left),
                    right,
                    join_type,
                    condition: Some(new_condition),
                }
            } else {
                LogicalPlan::Join {
                    left,
                    right,
                    join_type,
                    condition: None,
                }
            }
        }

        LogicalPlan::Aggregate {
            input,
            group_by,
            aggregates,
        } => {
            // Inject Project node with CSE candidates
            let table_qualifier = get_table_qualifier(&input);
            let new_input = CsePlanRewriter::inject_projection(*input, &candidates);

            // Replace common subexpressions in GROUP BY
            let new_group_by: Vec<_> = group_by
                .into_iter()
                .map(|expr| {
                    CsePlanRewriter::replace_with_cse_columns(expr, &candidates, &table_qualifier)
                })
                .collect();

            // Replace common subexpressions in aggregate expressions
            let new_aggregates: Vec<_> = aggregates
                .into_iter()
                .map(|agg| AggregateExpr {
                    func: agg.func,
                    args: agg
                        .args
                        .into_iter()
                        .map(|arg| {
                            CsePlanRewriter::replace_with_cse_columns(
                                arg,
                                &candidates,
                                &table_qualifier,
                            )
                        })
                        .collect(),
                    alias: agg.alias,
                    return_type: agg.return_type,
                    filter: agg.filter.map(|f| {
                        CsePlanRewriter::replace_with_cse_columns(f, &candidates, &table_qualifier)
                    }),
                })
                .collect();

            LogicalPlan::Aggregate {
                input: Box::new(new_input),
                group_by: new_group_by,
                aggregates: new_aggregates,
            }
        }

        // Other node types not yet supported
        _ => plan,
    }
}

/// Get the table qualifier for referencing intermediate projection columns
pub(super) fn get_table_qualifier(plan: &LogicalPlan) -> String {
    match plan {
        LogicalPlan::Project { input, .. } => get_table_qualifier(input),
        LogicalPlan::Scan { table, alias, .. } => alias.as_deref().unwrap_or(table).to_string(),
        LogicalPlan::TableFunction { alias, name, .. } => {
            alias.as_deref().unwrap_or(name).to_string()
        }
        LogicalPlan::Subquery { alias, .. }
        | LogicalPlan::CTEScan {
            alias: Some(alias), ..
        } => alias.clone(),
        LogicalPlan::CTEScan { cte_name, .. } => cte_name.clone(),
        LogicalPlan::Filter { input, .. }
        | LogicalPlan::Sort { input, .. }
        | LogicalPlan::Limit { input, .. }
        | LogicalPlan::Distinct { input, .. }
        | LogicalPlan::Aggregate { input, .. }
        | LogicalPlan::Window { input, .. }
        | LogicalPlan::LateralMap { input, .. } => get_table_qualifier(input),
        LogicalPlan::Join { left, .. } => get_table_qualifier(left),
        LogicalPlan::SemiJoin { left, .. } => get_table_qualifier(left),
        LogicalPlan::WithCTE { main_query, .. } => get_table_qualifier(main_query),
        // DML operations don't have meaningful table qualifiers (they're leaf nodes)
        LogicalPlan::Insert { target, .. }
        | LogicalPlan::Update { target, .. }
        | LogicalPlan::Delete { target, .. } => target.table_name(),
        // ORDER has no table qualifier
        LogicalPlan::Order { .. } => "order".to_string(),
        // MOVE has no table qualifier
        LogicalPlan::Move { .. } => "move".to_string(),
        // COPY has no table qualifier
        LogicalPlan::Copy { .. } => "copy".to_string(),
        // TRANSLATE has no table qualifier
        LogicalPlan::Translate { .. } => "translate".to_string(),
        // RELATE has no table qualifier
        LogicalPlan::Relate { .. } => "relate".to_string(),
        // UNRELATE has no table qualifier
        LogicalPlan::Unrelate { .. } => "unrelate".to_string(),
        // Empty plan has no table qualifier
        LogicalPlan::Empty => "empty".to_string(),
    }
}
