//! Explain formatting for query operators (Scan, Filter, Project, Sort, etc.)

use super::super::operators::LogicalPlan;

/// Format query-related plan nodes as tree strings
pub(super) fn explain_query_op(plan: &LogicalPlan, prefix: &str, indent: usize) -> String {
    match plan {
        LogicalPlan::Scan { table, filter, .. } => {
            let filter_str = if let Some(f) = filter {
                format!(" [filter: {:?}]", f.expr)
            } else {
                String::new()
            };
            format!("{}Scan: {}{}", prefix, table, filter_str)
        }
        LogicalPlan::TableFunction {
            name, alias, args, ..
        } => {
            let alias_str = alias
                .as_ref()
                .map(|a| format!(" AS {}", a))
                .unwrap_or_default();
            let arg_count = args.len();
            let plural = if arg_count == 1 { "" } else { "s" };
            format!(
                "{}TableFunction: {}{} ({} arg{})",
                prefix, name, alias_str, arg_count, plural
            )
        }
        LogicalPlan::Filter { input, predicate } => {
            let pred_str = if predicate.conjuncts.len() == 1 {
                format!("{:?}", predicate.conjuncts[0].expr)
            } else {
                let conjuncts: Vec<String> = predicate
                    .conjuncts
                    .iter()
                    .map(|c| format!("{:?}", c.expr))
                    .collect();
                format!("({})", conjuncts.join(" AND "))
            };
            format!(
                "{}Filter: {}\n{}",
                prefix,
                pred_str,
                input.explain_with_indent(indent + 1)
            )
        }
        LogicalPlan::Project { input, exprs } => {
            let cols: Vec<String> = exprs
                .iter()
                .map(|e| format!("{:?} AS {}", e.expr.expr, e.alias))
                .collect();
            format!(
                "{}Project: [{}]\n{}",
                prefix,
                cols.join(", "),
                input.explain_with_indent(indent + 1)
            )
        }
        LogicalPlan::Sort { input, sort_exprs } => {
            let sorts: Vec<String> = sort_exprs
                .iter()
                .map(|s| {
                    format!(
                        "{:?} {}",
                        s.expr.expr,
                        if s.ascending { "ASC" } else { "DESC" }
                    )
                })
                .collect();
            format!(
                "{}Sort: [{}]\n{}",
                prefix,
                sorts.join(", "),
                input.explain_with_indent(indent + 1)
            )
        }
        LogicalPlan::Limit {
            input,
            limit,
            offset,
        } => {
            format!(
                "{}Limit: {} OFFSET {}\n{}",
                prefix,
                limit,
                offset,
                input.explain_with_indent(indent + 1)
            )
        }
        LogicalPlan::Distinct {
            input,
            distinct_spec,
        } => {
            let spec_str = match distinct_spec {
                super::super::operators::DistinctSpec::All => "ALL".to_string(),
                super::super::operators::DistinctSpec::On(exprs) => {
                    let expr_strs: Vec<String> =
                        exprs.iter().map(|e| format!("{:?}", e.expr)).collect();
                    format!("ON ({})", expr_strs.join(", "))
                }
            };
            format!(
                "{}Distinct: {}\n{}",
                prefix,
                spec_str,
                input.explain_with_indent(indent + 1)
            )
        }
        LogicalPlan::Aggregate {
            input,
            group_by,
            aggregates,
        } => {
            format!(
                "{}Aggregate: group_by={:?}, aggs={:?}\n{}",
                prefix,
                group_by,
                aggregates,
                input.explain_with_indent(indent + 1)
            )
        }
        LogicalPlan::Join {
            left,
            right,
            join_type,
            condition,
        } => {
            let cond_str = if let Some(cond) = condition {
                format!(" ON {:?}", cond.expr)
            } else {
                String::new()
            };
            format!(
                "{}Join: {:?}{}\n{}\n{}",
                prefix,
                join_type,
                cond_str,
                left.explain_with_indent(indent + 1),
                right.explain_with_indent(indent + 1)
            )
        }
        LogicalPlan::SemiJoin {
            left,
            right,
            left_key,
            right_key,
            anti,
        } => {
            let join_type = if *anti { "Anti-SemiJoin" } else { "SemiJoin" };
            format!(
                "{}{}: {:?} = {:?}\n{}\n{}",
                prefix,
                join_type,
                left_key.expr,
                right_key.expr,
                left.explain_with_indent(indent + 1),
                right.explain_with_indent(indent + 1)
            )
        }
        LogicalPlan::WithCTE { ctes, main_query } => {
            let mut result = format!("{}WithCTE: {} CTE(s)\n", prefix, ctes.len());

            for (name, cte_plan) in ctes {
                result.push_str(&format!("{}  CTE '{}': \n", prefix, name));
                result.push_str(&cte_plan.explain_with_indent(indent + 2));
                result.push('\n');
            }

            result.push_str(&format!("{}  Main Query:\n", prefix));
            result.push_str(&main_query.explain_with_indent(indent + 2));
            result
        }
        LogicalPlan::CTEScan {
            cte_name, alias, ..
        } => {
            let alias_str = alias
                .as_ref()
                .map(|a| format!(" AS {}", a))
                .unwrap_or_default();
            format!("{}CTEScan: {}{}", prefix, cte_name, alias_str)
        }
        LogicalPlan::Subquery { input, alias, .. } => {
            format!(
                "{}Subquery AS {}\n{}",
                prefix,
                alias,
                input.explain_with_indent(indent + 1)
            )
        }
        LogicalPlan::Window {
            input,
            window_exprs,
        } => {
            let window_strs: Vec<String> = window_exprs
                .iter()
                .map(|w| format!("{:?} AS {}", w.function, w.alias))
                .collect();
            format!(
                "{}Window: [{}]\n{}",
                prefix,
                window_strs.join(", "),
                input.explain_with_indent(indent + 1)
            )
        }
        LogicalPlan::LateralMap {
            input,
            function_expr,
            column_name,
        } => {
            format!(
                "{}LateralMap: {:?} AS {}\n{}",
                prefix,
                function_expr.expr,
                column_name,
                input.explain_with_indent(indent + 1)
            )
        }
        // Non-query operators are handled elsewhere
        _ => unreachable!("explain_query_op called for non-query operator"),
    }
}
