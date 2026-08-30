//! Vector k-NN optimization within LIMIT planning
//!
//! Detects `ORDER BY (vector_col <op> query) LIMIT k` patterns and converts
//! them into a `VectorScan` physical plan for efficient approximate nearest
//! neighbor search.

use super::super::{
    Error, Expr, Literal, LogicalPlan, PhysicalPlan, PhysicalPlanner, PlanContext, TypedExpr,
    VectorDistanceMetric,
};
use raisin_sql::analyzer::BinaryOperator;

/// How much wider than `k` the candidate pool is drawn when a residual filter
/// sits above the scan.
///
/// The index ranks by distance alone, so a scoped query (`node_type = 'X'`,
/// a subtree, an installation's own folder) can have every one of its k global
/// neighbours rejected by the filter. Fetching `k * RESIDUAL_OVERFETCH` gives
/// the filter something to work with. The same device already exists one layer
/// down for the workspace filter in `raisin-hnsw`, and multiplies with it.
///
/// This is a RE-EXPORT, not a second declaration. Row-level security is a
/// residual filter too, so the search table functions face exactly this problem
/// and must not answer it with a different number that drifts.
const RESIDUAL_OVERFETCH: usize = crate::physical_plan::search::SEARCH_OVERFETCH;

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

        // Collect the scan input and EVERY conjunct constraining it.
        //
        // A filter reaches this point in two different shapes and the planner
        // has to read BOTH. `Filter { Scan }` survives when predicate pushdown
        // declined; when it succeeded — the normal case — there is no Filter
        // node left at all and the predicate lives in `Scan.filter`. That
        // second shape was never read here, which is why
        // `WHERE node_type = 'X' ORDER BY embedding <=> EMBEDDING(q) LIMIT k`
        // built a VectorScan carrying no constraint whatsoever and answered
        // with the global k nearest neighbours, of any type, presented as
        // correct.
        let (scan_input, mut conjuncts) = match actual_sort_input {
            LogicalPlan::Scan { .. } => (actual_sort_input, Vec::new()),
            LogicalPlan::Filter {
                input: filter_input,
                predicate,
            } => (filter_input.as_ref(), predicate.conjuncts.clone()),
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
            filter: pushed_filter,
            ..
        } = scan_input
        {
            if let Some(pushed) = pushed_filter {
                conjuncts.extend(self.flatten_ands(pushed));
            }

            let workspace_name = workspace
                .clone()
                .unwrap_or_else(|| self.default_workspace.to_string());
            let effective_branch = branch_override
                .clone()
                .unwrap_or_else(|| self.default_branch.to_string());

            // Split the conjuncts into what the scan CONSUMES and what has to
            // survive as a row-level filter. Nothing may fall between the two —
            // that gap is exactly what this split exists to close.
            let mut max_distance: Option<f32> = None;
            let mut residual: Vec<TypedExpr> = Vec::new();
            for conjunct in conjuncts {
                if max_distance.is_none() {
                    if let Some(threshold) =
                        self.conjunct_max_distance(&conjunct, distance_alias.as_deref())
                    {
                        max_distance = Some(threshold);
                        continue;
                    }
                }
                if Self::vector_scan_guarantees(&conjunct) {
                    continue;
                }
                residual.push(conjunct);
            }

            let residual_count = residual.len();
            let residual_filter = if residual.is_empty() {
                None
            } else {
                Some(self.combine_predicates(&residual))
            };

            // A residual filter runs AFTER the index has truncated to its k
            // nearest, so `LIMIT 5` scoped to a subtree could find its five
            // global neighbours, reject all five and return nothing. Widen the
            // candidate pool, the way the workspace filter already does inside
            // the engine. The answer stays approximate — an ANN index always is
            // — but it becomes an approximate answer to the question asked.
            let overfetch = if residual_filter.is_some() {
                RESIDUAL_OVERFETCH
            } else {
                1
            };

            tracing::info!(
                "Detected vector k-NN pattern: {} {} LIMIT {} (distance alias: {:?}, residual predicates: {}, overfetch: {})",
                vector_column,
                distance_metric,
                limit,
                distance_alias,
                residual_count,
                overfetch
            );

            // VectorScan outputs the distance column with the correct alias, so
            // there is no Project to wrap it in.
            let scan = PhysicalPlan::VectorScan {
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
                overfetch,
                max_distance,
                projection: projection.clone(),
                distance_alias,
            };

            if residual_filter.is_none() {
                return Ok(Some(scan));
            }

            // The same helper the FullTextScan / NodeIdScan / PathIndexScan
            // branches use — one implementation of "wrap a scan in what it does
            // not itself guarantee". The Limit re-imposes the user's k, which
            // the widened candidate pool would otherwise overshoot.
            return Ok(Some(PhysicalPlan::Limit {
                input: Box::new(Self::wrap_with_residual(scan, residual_filter)),
                limit,
                offset: 0,
            }));
        }

        Ok(None)
    }

    /// The `max_distance` threshold this ONE conjunct expresses, if it is
    /// nothing but a threshold.
    ///
    /// Recognises `embedding <=> EMBEDDING('q') < 0.5`, `distance_alias <= 0.5`
    /// and the reversed forms. It deliberately does NOT recurse through `AND`:
    /// the caller has already flattened the filter into conjuncts, and a
    /// version that answered `Some` for `x < 0.5 AND y = 1` would let the
    /// caller consume the whole conjunct and throw `y = 1` away — the exact
    /// failure this change exists to remove.
    fn conjunct_max_distance(&self, expr: &TypedExpr, distance_alias: Option<&str>) -> Option<f32> {
        match &expr.expr {
            // distance_expr < threshold  or  distance_expr <= threshold
            Expr::BinaryOp {
                left,
                op: BinaryOperator::Lt | BinaryOperator::LtEq,
                right,
            } if self.is_distance_related(left, distance_alias) => Self::expr_to_f32(right),

            // threshold > distance_expr  or  threshold >= distance_expr
            Expr::BinaryOp {
                left,
                op: BinaryOperator::Gt | BinaryOperator::GtEq,
                right,
            } if self.is_distance_related(right, distance_alias) => Self::expr_to_f32(left),

            _ => None,
        }
    }

    /// Check if an expression is related to vector distance (either a distance
    /// operator/function or a column matching the distance alias).
    fn is_distance_related(&self, expr: &TypedExpr, distance_alias: Option<&str>) -> bool {
        match &expr.expr {
            // Direct vector distance operator
            Expr::BinaryOp { op, .. } => matches!(
                op,
                BinaryOperator::VectorL2Distance
                    | BinaryOperator::VectorCosineDistance
                    | BinaryOperator::VectorInnerProduct
            ),
            // Vector distance function
            Expr::Function { name, .. } => matches!(
                name.to_uppercase().as_str(),
                "VECTOR_L2_DISTANCE" | "VECTOR_COSINE_DISTANCE" | "VECTOR_INNER_PRODUCT"
            ),
            // Column matching the distance alias (e.g., WHERE sim < 0.5)
            Expr::Column { column, .. } => distance_alias.is_some_and(|alias| column == alias),
            _ => false,
        }
    }

    /// Extract a numeric literal as f32
    fn expr_to_f32(expr: &TypedExpr) -> Option<f32> {
        match &expr.expr {
            Expr::Literal(Literal::Double(d)) => Some(*d as f32),
            Expr::Literal(Literal::Int(i)) => Some(*i as f32),
            Expr::Literal(Literal::BigInt(i)) => Some(*i as f32),
            _ => None,
        }
    }
}
