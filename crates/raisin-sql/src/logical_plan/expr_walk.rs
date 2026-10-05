//! Every expression a [`LogicalPlan`] owns, for passes that rewrite them in
//! place — parameter binding (`crate::template`, plan Phase 13d) is the first.
//!
//! ONE exhaustive enumeration: a new `LogicalPlan` variant fails to compile
//! here until it says which expressions it holds, so a binder cannot silently
//! leave a placeholder in an operator it has never heard of.

use super::operators::{DistinctSpec, LogicalPlan};
use crate::analyzer::{TypedExpr, WindowFunction};

impl LogicalPlan {
    /// Visit every top-level expression of this plan and of every plan below
    /// it (the callback recurses into an expression's children itself).
    /// `FilterPredicate::canonical` is NOT visited: it is a cache of the
    /// conjuncts' canonical form that the optimizer's hierarchy rewrite
    /// replaces wholesale; a caller rewriting conjuncts re-runs that pass
    /// (`Optimizer::rebind`).
    pub fn for_each_expr_mut(&mut self, f: &mut dyn FnMut(&mut TypedExpr)) {
        match self {
            LogicalPlan::Scan { filter, .. } => filter.iter_mut().for_each(|e| f(e)),
            LogicalPlan::TableFunction { args, filter, .. } => {
                args.iter_mut().for_each(|a| f(&mut a.value));
                filter.iter_mut().for_each(|e| f(e));
            }
            LogicalPlan::Filter { input, predicate } => {
                predicate.conjuncts.iter_mut().for_each(|e| f(e));
                input.for_each_expr_mut(f);
            }
            LogicalPlan::Project { input, exprs } => {
                exprs.iter_mut().for_each(|p| f(&mut p.expr));
                input.for_each_expr_mut(f);
            }
            LogicalPlan::Sort { input, sort_exprs } => {
                sort_exprs.iter_mut().for_each(|s| f(&mut s.expr));
                input.for_each_expr_mut(f);
            }
            LogicalPlan::Limit { input, .. } => input.for_each_expr_mut(f),
            LogicalPlan::Distinct {
                input,
                distinct_spec,
            } => {
                if let DistinctSpec::On(exprs) = distinct_spec {
                    exprs.iter_mut().for_each(|e| f(e));
                }
                input.for_each_expr_mut(f);
            }
            LogicalPlan::Aggregate {
                input,
                group_by,
                aggregates,
            } => {
                group_by.iter_mut().for_each(|e| f(e));
                for agg in aggregates {
                    agg.args.iter_mut().for_each(|e| f(e));
                    agg.filter.iter_mut().for_each(|e| f(e));
                    agg.order_by.iter_mut().for_each(|(e, _)| f(e));
                }
                input.for_each_expr_mut(f);
            }
            LogicalPlan::Join {
                left,
                right,
                condition,
                ..
            } => {
                condition.iter_mut().for_each(|e| f(e));
                left.for_each_expr_mut(f);
                right.for_each_expr_mut(f);
            }
            LogicalPlan::SemiJoin {
                left,
                right,
                left_key,
                right_key,
                ..
            } => {
                f(left_key);
                f(right_key);
                left.for_each_expr_mut(f);
                right.for_each_expr_mut(f);
            }
            LogicalPlan::SetOperation { left, right, .. } => {
                left.for_each_expr_mut(f);
                right.for_each_expr_mut(f);
            }
            LogicalPlan::WithCTE { ctes, main_query } => {
                for (_, cte) in ctes {
                    cte.for_each_expr_mut(f);
                }
                main_query.for_each_expr_mut(f);
            }
            LogicalPlan::Subquery { input, .. } => input.for_each_expr_mut(f),
            LogicalPlan::Window {
                input,
                window_exprs,
            } => {
                for w in window_exprs {
                    match &mut w.function {
                        WindowFunction::Sum(e)
                        | WindowFunction::Avg(e)
                        | WindowFunction::Min(e)
                        | WindowFunction::Max(e) => f(e),
                        WindowFunction::RowNumber
                        | WindowFunction::Rank
                        | WindowFunction::DenseRank
                        | WindowFunction::Count => {}
                    }
                    w.partition_by.iter_mut().for_each(|e| f(e));
                    w.order_by.iter_mut().for_each(|(e, _)| f(e));
                }
                input.for_each_expr_mut(f);
            }
            LogicalPlan::LateralMap {
                input,
                function_expr,
                ..
            } => {
                f(function_expr);
                input.for_each_expr_mut(f);
            }
            LogicalPlan::Insert {
                values, returning, ..
            } => {
                values.iter_mut().flatten().for_each(|e| f(e));
                returning.iter_mut().flatten().for_each(|p| f(&mut p.expr));
            }
            LogicalPlan::Update {
                assignments,
                filter,
                returning,
                ..
            } => {
                assignments.iter_mut().for_each(|(_, e)| f(e));
                filter.iter_mut().for_each(|e| f(e));
                returning.iter_mut().flatten().for_each(|p| f(&mut p.expr));
            }
            LogicalPlan::Delete {
                filter, returning, ..
            } => {
                filter.iter_mut().for_each(|e| f(e));
                returning.iter_mut().flatten().for_each(|p| f(&mut p.expr));
            }
            // Statements whose operands are not expressions.
            LogicalPlan::CTEScan { .. }
            | LogicalPlan::Order { .. }
            | LogicalPlan::Move { .. }
            | LogicalPlan::Copy { .. }
            | LogicalPlan::Translate { .. }
            | LogicalPlan::Relate { .. }
            | LogicalPlan::Unrelate { .. }
            | LogicalPlan::Empty => {}
        }
    }

    /// Visit the `locales` list of every scan in this plan — the locale a
    /// `locale = $n` predicate was extracted into.
    pub fn for_each_locales_mut(&mut self, f: &mut dyn FnMut(&mut Vec<String>)) {
        match self {
            LogicalPlan::Scan { locales, .. } | LogicalPlan::TableFunction { locales, .. } => {
                f(locales)
            }
            LogicalPlan::Filter { input, .. }
            | LogicalPlan::Project { input, .. }
            | LogicalPlan::Sort { input, .. }
            | LogicalPlan::Limit { input, .. }
            | LogicalPlan::Distinct { input, .. }
            | LogicalPlan::Aggregate { input, .. }
            | LogicalPlan::Subquery { input, .. }
            | LogicalPlan::Window { input, .. }
            | LogicalPlan::LateralMap { input, .. } => input.for_each_locales_mut(f),
            LogicalPlan::Join { left, right, .. }
            | LogicalPlan::SemiJoin { left, right, .. }
            | LogicalPlan::SetOperation { left, right, .. } => {
                left.for_each_locales_mut(f);
                right.for_each_locales_mut(f);
            }
            LogicalPlan::WithCTE { ctes, main_query } => {
                for (_, cte) in ctes {
                    cte.for_each_locales_mut(f);
                }
                main_query.for_each_locales_mut(f);
            }
            LogicalPlan::CTEScan { .. }
            | LogicalPlan::Insert { .. }
            | LogicalPlan::Update { .. }
            | LogicalPlan::Delete { .. }
            | LogicalPlan::Order { .. }
            | LogicalPlan::Move { .. }
            | LogicalPlan::Copy { .. }
            | LogicalPlan::Translate { .. }
            | LogicalPlan::Relate { .. }
            | LogicalPlan::Unrelate { .. }
            | LogicalPlan::Empty => {}
        }
    }
}
