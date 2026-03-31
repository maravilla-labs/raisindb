//! Vector k-NN optimization within LIMIT planning
//!
//! Detects `ORDER BY (vector_col <op> query) LIMIT k` patterns and converts
//! them into a `VectorScan` physical plan for efficient approximate nearest
//! neighbor search.

use super::super::{
    Error, LogicalPlan, PhysicalPlan, PhysicalPlanner, PlanContext, VectorDistanceMetric,
};

impl PhysicalPlanner {
    /// Try to optimise a `Limit { Sort { ... } }` pattern into a `VectorScan`
    /// when the sort expression is a vector distance function.
    ///
    /// Returns `Some(plan)` if the optimisation applied, `None` otherwise
    /// (callers should fall back to TopN or regular limit planning).
    pub(in crate::physical_plan::planner) fn try_plan_vector_knn(
        &self,
        sort_input: &LogicalPlan,
        sort_exprs: &[raisin_sql::logical_plan::SortExpr],
        limit: usize,
    ) -> Result<Option<PhysicalPlan>, Error> {
        if sort_exprs.len() != 1 {
            return Ok(None);
        }

        let sort_expr = &sort_exprs[0];

        // Try to detect vector pattern directly from sort expression
        let mut vector_pattern = self.detect_vector_knn_pattern(&sort_expr.expr);
        let mut distance_alias = None;

        // If not found, check if sort_input is a Project that computes the vector distance
        // This handles: SELECT *, embedding <=> EMBEDDING(...) AS sim FROM t ORDER BY sim
        if vector_pattern.is_none() {
            if let LogicalPlan::Project {
                input: _project_input,
                exprs,
            } = sort_input
            {
                vector_pattern = self.extract_vector_sort_from_project(exprs, &sort_expr.expr);

                // Extract the distance column alias from the Project expressions
                // The sort_expr references a column by alias (e.g., "sim")
                if let raisin_sql::analyzer::Expr::Column { column, .. } = &sort_expr.expr.expr {
                    distance_alias = Some(column.clone());
                }
            }
        }

        let (vector_column, query_vector, distance_metric, _is_asc) = match vector_pattern {
            Some(p) => p,
            None => return Ok(None),
        };

        if distance_alias.is_none() {
            distance_alias = self.find_vector_distance_alias(
                sort_input,
                &vector_column,
                &query_vector,
                distance_metric,
            );
        }

        // Determine the actual scan input (may need to traverse through Project)
        let actual_sort_input = match sort_input {
            LogicalPlan::Project { input, .. } => input.as_ref(),
            other => other,
        };

        // Extract scan information from the input
        // Look for a Scan or Filter(Scan) pattern
        let (scan_input, filter_opt) = match actual_sort_input {
            LogicalPlan::Scan { .. } => (actual_sort_input, None),
            LogicalPlan::Filter {
                input: filter_input,
                predicate,
            } => {
                // Check if ALL predicates are simple
                // If any predicate is complex, fall back to full scan + embedding population
                let all_simple = predicate
                    .conjuncts
                    .iter()
                    .all(|p| self.is_simple_predicate(p));

                if !all_simple {
                    tracing::debug!(
                        "Complex predicate detected - falling back to full scan + embedding population"
                    );
                    // Complex predicate - fall through to TopN
                    let topn_context = PlanContext::with_limit(limit);
                    return Ok(Some(PhysicalPlan::TopN {
                        input: Box::new(self.plan_with_context(sort_input, &topn_context)?),
                        sort_exprs: sort_exprs.to_vec(),
                        limit,
                    }));
                }

                // All predicates are simple - safe to use VectorScan
                let combined = self.combine_predicates(&predicate.conjuncts);
                (filter_input.as_ref(), Some(combined))
            }
            _ => {
                // Not a recognizable pattern, fall through to TopN
                tracing::debug!(
                    "Unrecognized pattern in vector scan optimization - falling back to TopN"
                );
                let topn_context = PlanContext::with_limit(limit);
                return Ok(Some(PhysicalPlan::TopN {
                    input: Box::new(self.plan_with_context(sort_input, &topn_context)?),
                    sort_exprs: sort_exprs.to_vec(),
                    limit,
                }));
            }
        };

        // Extract scan details
        if let LogicalPlan::Scan {
            table,
            alias,
            workspace,
            branch_override,
            projection,
            ..
        } = scan_input
        {
            let workspace_name = workspace
                .clone()
                .unwrap_or_else(|| self.default_workspace.to_string());
            let effective_branch = branch_override
                .clone()
                .unwrap_or_else(|| self.default_branch.to_string());

            // TODO: Extract max_distance threshold from filter
            let max_distance = None;

            if let Some(ref alias_name) = distance_alias {
                tracing::info!(
                    "Detected vector k-NN pattern: {} {} LIMIT {} (distance alias: {})",
                    vector_column,
                    distance_metric,
                    limit,
                    alias_name
                );
            } else {
                tracing::info!(
                    "Detected vector k-NN pattern: {} {} LIMIT {}",
                    vector_column,
                    distance_metric,
                    limit
                );
            }

            // VectorScan now outputs the distance column with the correct alias
            // No need to wrap in Project - VectorScan handles the alias directly
            return Ok(Some(PhysicalPlan::VectorScan {
                tenant_id: self.default_tenant_id.to_string(),
                repo_id: self.default_repo_id.to_string(),
                branch: effective_branch,
                workspace: workspace_name,
                table: table.clone(),
                alias: alias.clone(),
                query_vector: query_vector.clone(),
                distance_metric,
                vector_column,
                k: limit,
                max_distance,
                projection: projection.clone(),
                distance_alias,
            }));
        }

        Ok(None)
    }
}
