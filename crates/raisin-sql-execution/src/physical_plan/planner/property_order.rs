//! Property-ordered scan optimization
//!
//! Detects query patterns that can use PropertyOrderScan for efficient
//! ORDER BY + LIMIT queries (e.g., ORDER BY created_at DESC LIMIT 10).

use super::{
    CanonicalPredicate, Error, FilterSelectivityHint, LogicalPlan, PhysicalPlan, PhysicalPlanner,
    PlanContext, ProjectionExpr, PropertyOrderComponents, ScanNodeInfo, ScanReason, SortExpr,
    TableSchema, TypedExpr, SCAN_LIMIT_BUFFER,
};
use raisin_sql::analyzer::{BinaryOperator, DataType, Expr, Literal};
use std::borrow::Cow;
use std::sync::Arc;

impl PhysicalPlanner {
    pub(super) fn try_plan_property_order(
        &self,
        input: &LogicalPlan,
        limit: usize,
        offset: usize,
    ) -> Result<Option<PhysicalPlan>, Error> {
        tracing::debug!("🔍 Checking PropertyOrderScan optimization for LIMIT query");

        let LogicalPlan::Sort {
            input: sort_input,
            sort_exprs,
        } = input
        else {
            tracing::debug!("❌ PropertyOrderScan: input is not a Sort node");
            return Ok(None);
        };

        if sort_exprs.len() != 1 {
            tracing::debug!(
                "❌ PropertyOrderScan: multiple sort expressions ({})",
                sort_exprs.len()
            );
            return Ok(None);
        }

        let sort_expr = sort_exprs.first().expect("non-empty after length check");
        let property_name = match Self::match_property_order_column(sort_expr) {
            Some(name) => {
                tracing::debug!(
                    "✅ PropertyOrderScan: matched property '{}' from column '{:?}'",
                    name,
                    sort_expr.expr
                );
                name.into_owned()
            }
            None => {
                tracing::debug!(
                    "❌ PropertyOrderScan: unrecognized column in sort expression: {:?}",
                    sort_expr.expr
                );
                return Ok(None);
            }
        };

        let components = match self.extract_property_order_components(sort_input.as_ref()) {
            Some(c) => {
                tracing::debug!("✅ PropertyOrderScan: extracted components from plan structure");
                c
            }
            None => {
                tracing::debug!("❌ PropertyOrderScan: failed to extract components (expected Project -> [Filter ->]* Scan)");
                return Ok(None);
            }
        };

        let workspace_name = components
            .scan_info
            .workspace
            .clone()
            .unwrap_or_else(|| self.default_workspace.to_string());
        let branch = components
            .scan_info
            .branch_override
            .clone()
            .unwrap_or_else(|| self.default_branch.to_string());

        // Optimization: If ordering by path and there's a PATH_STARTS_WITH/CHILD_OF/DESCENDANT_OF predicate,
        // skip PropertyOrderScan - PrefixScan is much more efficient
        // (it jumps directly to the prefix and returns sorted results naturally)
        if property_name == "__path" {
            if let Some(ref filter) = components.filter_expr {
                if Self::contains_path_starts_with(filter)
                    || Self::contains_child_of(filter)
                    || Self::contains_descendant_of(filter)
                {
                    tracing::info!(
                        "⚡ Skipping PropertyOrderScan for path ordering with PATH_STARTS_WITH/CHILD_OF/DESCENDANT_OF - PrefixScan is better"
                    );
                    return Ok(None);
                }
            }
        }

        // Optimization: If ordering by a non-path property (like created_at) with PATH_STARTS_WITH/CHILD_OF/DESCENDANT_OF,
        // skip PropertyOrderScan - it would scan the entire database!
        // Better to use PrefixScan to filter first, then sort matching rows in memory.
        //
        // Problem: PropertyOrderScan scans ALL nodes in property order (e.g., by timestamp),
        // fetching each node to check if path matches. With 2M nodes, this examines all 2M.
        // Solution: PrefixScan jumps to matching path prefix, returns ~10K nodes, sorts in memory.
        // Impact: 2000ms → 100-200ms
        if property_name != "__path" {
            if let Some(ref filter) = components.filter_expr {
                if Self::contains_path_starts_with(filter)
                    || Self::contains_child_of(filter)
                    || Self::contains_descendant_of(filter)
                {
                    tracing::info!(
                        "⚡ Skipping PropertyOrderScan for '{}' ordering with PATH_STARTS_WITH/CHILD_OF/DESCENDANT_OF - using PrefixScan + sort (avoids full table scan)",
                        property_name
                    );
                    return Ok(None);
                }
            }
        }

        // PHASE 2 OPTIMIZATION: If filter has highly-selective predicates (node_type or multiple property filters),
        // skip PropertyOrderScan and use filter-first strategy (PropertyIndexScan + TopN).
        //
        // Rationale: When node_type = 'X' matches few nodes (e.g., 50 out of 2M), it's faster to:
        // 1. Filter first: PropertyIndexScan(node_type='X') → 50 nodes
        // 2. Sort in memory: 50 nodes → trivial
        // 3. Apply LIMIT
        //
        // vs. PropertyOrderScan which scans created_at index looking for matches (could scan 2M entries).
        //
        // Heuristic: node_type predicates or multiple property predicates indicate high selectivity.
        if property_name != "__path" {
            if let Some(ref filter) = components.filter_expr {
                let selectivity_hint = Self::estimate_filter_selectivity_hint(filter);
                if selectivity_hint == FilterSelectivityHint::HighlySelective {
                    tracing::info!(
                        "⚡ Skipping PropertyOrderScan for '{}' ordering with highly-selective filter - using filter-first strategy (PropertyIndexScan + TopN)",
                        property_name
                    );
                    return Ok(None);
                }
            }
        }

        let limit_hint = limit.saturating_add(offset);

        tracing::info!(
            "⚡ Detected property-order optimization on '{}' (limit={}, offset={})",
            property_name,
            limit,
            offset
        );

        let mut plan = PhysicalPlan::PropertyOrderScan {
            tenant_id: self.default_tenant_id.to_string(),
            repo_id: self.default_repo_id.to_string(),
            branch,
            workspace: workspace_name,
            table: components.scan_info.table,
            alias: components.scan_info.alias,
            schema: components.scan_info.schema,
            projection: components.scan_info.projection,
            filter: components.filter_expr,
            property_name,
            ascending: sort_exprs[0].ascending,
            limit: limit_hint,
        };

        plan = PhysicalPlan::Project {
            input: Box::new(plan),
            exprs: components.project_exprs,
        };

        plan = PhysicalPlan::Limit {
            input: Box::new(plan),
            limit,
            offset,
        };

        Ok(Some(plan))
    }

    /// Match a sort expression to a property name for PropertyOrderScan.
    ///
    /// Recognizes system columns (`created_at`, `updated_at`, `path`) and
    /// custom property expressions (`properties->>'field'`, including `::String` casts).
    /// Returns `Cow::Borrowed` for system columns to avoid allocation on the hot path.
    pub(super) fn match_property_order_column(sort_expr: &SortExpr) -> Option<Cow<'static, str>> {
        Self::extract_order_property(&sort_expr.expr, false)
    }

    /// Extract column name from a TypedExpr (for ORDER BY context propagation).
    ///
    /// Similar to `match_property_order_column` but also passes through unknown
    /// plain columns (e.g., aliases) for broader context propagation.
    pub(super) fn extract_column_name(expr: &TypedExpr) -> Option<String> {
        Self::extract_order_property(expr, true).map(Cow::into_owned)
    }

    /// Shared helper: extract an orderable property name from a typed expression.
    ///
    /// Iteratively unwraps `Cast` wrappers (no recursion) and recognizes:
    /// - `Expr::Column` for system columns (`created_at` → `__created_at`, etc.)
    /// - `Expr::JsonExtractText` for custom properties (`properties->>'title'`)
    ///
    /// When `allow_unknown_columns` is true, unrecognized plain columns are passed
    /// through (used by `extract_column_name` for ORDER BY context propagation).
    fn extract_order_property(
        expr: &TypedExpr,
        allow_unknown_columns: bool,
    ) -> Option<Cow<'static, str>> {
        // Iteratively unwrap Cast expressions to avoid unbounded recursion
        let mut current = expr;
        while let Expr::Cast { expr: inner, .. } = &current.expr {
            current = inner;
        }

        match &current.expr {
            Expr::Column { column, .. } => {
                // Strip table qualifier if present (e.g., "social.created_at" -> "created_at")
                let col_name = column.split('.').next_back().unwrap_or(column);
                match col_name.to_ascii_lowercase().as_str() {
                    "created_at" => Some(Cow::Borrowed("__created_at")),
                    "updated_at" => Some(Cow::Borrowed("__updated_at")),
                    "path" => Some(Cow::Borrowed("__path")),
                    other if allow_unknown_columns => Some(Cow::Owned(other.to_string())),
                    _ => None,
                }
            }
            // Handle properties->>'field' (JsonExtractText)
            Expr::JsonExtractText { object, key } => {
                if let Expr::Column { column, .. } = &object.expr {
                    if column
                        .split('.')
                        .next_back()
                        .unwrap_or(column)
                        .eq_ignore_ascii_case("properties")
                    {
                        if let Expr::Literal(Literal::Text(prop_name)) = &key.expr {
                            return Some(Cow::Owned(prop_name.clone()));
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Check if an expression contains a PATH_STARTS_WITH function call
    pub(super) fn contains_path_starts_with(expr: &TypedExpr) -> bool {
        match &expr.expr {
            Expr::Function { name, args, .. } => {
                if name.to_uppercase() == "PATH_STARTS_WITH" {
                    return true;
                }
                // Recursively check arguments
                args.iter().any(Self::contains_path_starts_with)
            }
            Expr::BinaryOp { left, right, .. } => {
                Self::contains_path_starts_with(left) || Self::contains_path_starts_with(right)
            }
            Expr::UnaryOp { expr, .. } => Self::contains_path_starts_with(expr),
            _ => false,
        }
    }

    /// Check if an expression contains a CHILD_OF function call
    pub(super) fn contains_child_of(expr: &TypedExpr) -> bool {
        match &expr.expr {
            Expr::Function { name, args, .. } => {
                if name.to_uppercase() == "CHILD_OF" {
                    return true;
                }
                // Recursively check arguments
                args.iter().any(Self::contains_child_of)
            }
            Expr::BinaryOp { left, right, .. } => {
                Self::contains_child_of(left) || Self::contains_child_of(right)
            }
            Expr::UnaryOp { expr, .. } => Self::contains_child_of(expr),
            _ => false,
        }
    }

    /// Check if an expression contains a DESCENDANT_OF function call
    pub(super) fn contains_descendant_of(expr: &TypedExpr) -> bool {
        match &expr.expr {
            Expr::Function { name, args, .. } => {
                if name.to_uppercase() == "DESCENDANT_OF" {
                    return true;
                }
                // Recursively check arguments
                args.iter().any(Self::contains_descendant_of)
            }
            Expr::BinaryOp { left, right, .. } => {
                Self::contains_descendant_of(left) || Self::contains_descendant_of(right)
            }
            Expr::UnaryOp { expr, .. } => Self::contains_descendant_of(expr),
            _ => false,
        }
    }

    /// Estimate filter selectivity for scan selection.
    ///
    /// Returns HighlySelective if the filter contains:
    /// - node_type = 'SomeType' (node types typically have few instances)
    /// - Multiple property equality predicates (compound filters are very selective)
    ///
    /// This is a heuristic-based approach that doesn't require runtime statistics.
    pub(super) fn estimate_filter_selectivity_hint(expr: &TypedExpr) -> FilterSelectivityHint {
        let (has_node_type, property_count) = Self::count_filter_predicates(expr);

        // node_type = 'X' is typically highly selective
        if has_node_type {
            return FilterSelectivityHint::HighlySelective;
        }

        // Multiple property equality predicates indicate high selectivity
        // (the intersection of two filters is more selective than either alone)
        if property_count >= 2 {
            return FilterSelectivityHint::HighlySelective;
        }

        FilterSelectivityHint::Unknown
    }

    /// Count filter predicates to estimate selectivity.
    /// Returns (has_node_type_filter, count_of_property_equality_filters)
    pub(super) fn count_filter_predicates(expr: &TypedExpr) -> (bool, usize) {
        match &expr.expr {
            // AND: combine counts from both sides
            Expr::BinaryOp {
                left,
                op: BinaryOperator::And,
                right,
            } => {
                let (left_nt, left_count) = Self::count_filter_predicates(left);
                let (right_nt, right_count) = Self::count_filter_predicates(right);
                (left_nt || right_nt, left_count + right_count)
            }

            // Equality: check for node_type or property filter
            Expr::BinaryOp {
                left,
                op: BinaryOperator::Eq,
                right,
            } => {
                // Check for node_type = 'value'
                if let Expr::Column { column, .. } = &left.expr {
                    if column.to_lowercase() == "node_type"
                        && matches!(&right.expr, Expr::Literal(Literal::Text(_)))
                    {
                        return (true, 0);
                    }
                }

                // Check for properties ->> 'key' = 'value'
                if let Expr::JsonExtractText { object, .. } = &left.expr {
                    if let Expr::Column { column, .. } = &object.expr {
                        if column.to_lowercase() == "properties"
                            && matches!(&right.expr, Expr::Literal(Literal::Text(_)))
                        {
                            return (false, 1);
                        }
                    }
                }

                (false, 0)
            }

            // OR: don't count (OR makes filters less selective)
            Expr::BinaryOp {
                op: BinaryOperator::Or,
                ..
            } => (false, 0),

            // Other expressions: no contribution
            _ => (false, 0),
        }
    }

    pub(super) fn extract_property_order_components(
        &self,
        plan: &LogicalPlan,
    ) -> Option<PropertyOrderComponents> {
        let LogicalPlan::Project { input, exprs } = plan else {
            return None;
        };

        let mut filters: Vec<TypedExpr> = Vec::new();
        let mut current = input.as_ref();

        while let LogicalPlan::Filter { input, predicate } = current {
            filters.extend(predicate.conjuncts.clone());
            current = input.as_ref();
        }

        let LogicalPlan::Scan {
            table,
            alias,
            schema,
            workspace,
            branch_override,
            filter,
            projection,
            ..
        } = current
        else {
            return None;
        };

        if let Some(scan_filter) = filter {
            filters.push(scan_filter.clone());
        }

        let combined_filter = if filters.is_empty() {
            None
        } else {
            Some(self.combine_predicates(&filters))
        };

        Some(PropertyOrderComponents {
            project_exprs: exprs.clone(),
            filter_expr: combined_filter,
            scan_info: ScanNodeInfo {
                table: table.clone(),
                alias: alias.clone(),
                schema: schema.clone(),
                workspace: workspace.clone(),
                branch_override: branch_override.clone(),
                projection: projection.clone(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_sql::analyzer::{DataType, Expr, Literal, TypedExpr};

    fn make_sort_expr(expr: TypedExpr) -> SortExpr {
        SortExpr {
            expr,
            ascending: true,
            nulls_first: false,
        }
    }

    fn make_column(name: &str) -> TypedExpr {
        TypedExpr::new(
            Expr::Column {
                table: String::new(),
                column: name.to_string(),
            },
            DataType::Text,
        )
    }

    fn make_json_extract_text(object_col: &str, key: &str) -> TypedExpr {
        TypedExpr::new(
            Expr::JsonExtractText {
                object: Box::new(TypedExpr::new(
                    Expr::Column {
                        table: String::new(),
                        column: object_col.to_string(),
                    },
                    DataType::JsonB,
                )),
                key: Box::new(TypedExpr::literal(Literal::Text(key.to_string()))),
            },
            DataType::Text,
        )
    }

    fn wrap_cast(expr: TypedExpr) -> TypedExpr {
        TypedExpr::new(
            Expr::Cast {
                expr: Box::new(expr),
                target_type: DataType::Text,
            },
            DataType::Text,
        )
    }

    // --- match_property_order_column tests ---

    #[test]
    fn test_system_columns() {
        let cases = [
            ("created_at", "__created_at"),
            ("updated_at", "__updated_at"),
            ("path", "__path"),
        ];
        for (col, expected) in cases {
            let sort = make_sort_expr(make_column(col));
            let result = PhysicalPlanner::match_property_order_column(&sort);
            assert_eq!(result.as_deref(), Some(expected), "column: {col}");
        }
    }

    #[test]
    fn test_system_columns_with_table_qualifier() {
        let expr = TypedExpr::new(
            Expr::Column {
                table: "social".to_string(),
                column: "social.created_at".to_string(),
            },
            DataType::Text,
        );
        let sort = make_sort_expr(expr);
        assert_eq!(
            PhysicalPlanner::match_property_order_column(&sort).as_deref(),
            Some("__created_at")
        );
    }

    #[test]
    fn test_unknown_column_returns_none() {
        let sort = make_sort_expr(make_column("unknown_col"));
        assert!(PhysicalPlanner::match_property_order_column(&sort).is_none());
    }

    #[test]
    fn test_json_extract_text_custom_property() {
        let sort = make_sort_expr(make_json_extract_text("properties", "title"));
        assert_eq!(
            PhysicalPlanner::match_property_order_column(&sort).as_deref(),
            Some("title")
        );
    }

    #[test]
    fn test_json_extract_text_with_table_qualifier() {
        let expr = TypedExpr::new(
            Expr::JsonExtractText {
                object: Box::new(TypedExpr::new(
                    Expr::Column {
                        table: "ws".to_string(),
                        column: "ws.properties".to_string(),
                    },
                    DataType::JsonB,
                )),
                key: Box::new(TypedExpr::literal(Literal::Text("title".to_string()))),
            },
            DataType::Text,
        );
        let sort = make_sort_expr(expr);
        assert_eq!(
            PhysicalPlanner::match_property_order_column(&sort).as_deref(),
            Some("title")
        );
    }

    #[test]
    fn test_json_extract_non_properties_column_returns_none() {
        let sort = make_sort_expr(make_json_extract_text("other_column", "title"));
        assert!(PhysicalPlanner::match_property_order_column(&sort).is_none());
    }

    #[test]
    fn test_cast_wrapping_json_extract() {
        // properties->>'title'::String
        let inner = make_json_extract_text("properties", "title");
        let sort = make_sort_expr(wrap_cast(inner));
        assert_eq!(
            PhysicalPlanner::match_property_order_column(&sort).as_deref(),
            Some("title")
        );
    }

    #[test]
    fn test_deeply_nested_casts_unwrap_iteratively() {
        // Cast(Cast(Cast(properties->>'title'))) — should not stack overflow
        let mut expr = make_json_extract_text("properties", "title");
        for _ in 0..100 {
            expr = wrap_cast(expr);
        }
        let sort = make_sort_expr(expr);
        assert_eq!(
            PhysicalPlanner::match_property_order_column(&sort).as_deref(),
            Some("title")
        );
    }

    #[test]
    fn test_cast_wrapping_system_column() {
        let sort = make_sort_expr(wrap_cast(make_column("created_at")));
        assert_eq!(
            PhysicalPlanner::match_property_order_column(&sort).as_deref(),
            Some("__created_at")
        );
    }

    // --- extract_column_name tests ---

    #[test]
    fn test_extract_column_name_system_columns() {
        assert_eq!(
            PhysicalPlanner::extract_column_name(&make_column("created_at")),
            Some("__created_at".to_string())
        );
    }

    #[test]
    fn test_extract_column_name_unknown_passes_through() {
        // extract_column_name allows unknown columns (unlike match_property_order_column)
        assert_eq!(
            PhysicalPlanner::extract_column_name(&make_column("my_alias")),
            Some("my_alias".to_string())
        );
    }

    #[test]
    fn test_extract_column_name_json_extract() {
        let expr = make_json_extract_text("properties", "email");
        assert_eq!(
            PhysicalPlanner::extract_column_name(&expr),
            Some("email".to_string())
        );
    }

    #[test]
    fn test_extract_column_name_cast_unwrap() {
        let expr = wrap_cast(make_json_extract_text("properties", "email"));
        assert_eq!(
            PhysicalPlanner::extract_column_name(&expr),
            Some("email".to_string())
        );
    }
}
