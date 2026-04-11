//! Predicate extraction, removal, and combination utilities
//!
//! Provides methods to extract specific predicate types from canonicalized
//! predicates and to combine/remove predicates.

use super::{CanonicalPredicate, ComparisonOp, Expr, Literal, PhysicalPlanner, TypedExpr};
use raisin_sql::analyzer::{BinaryOperator, DataType};

impl PhysicalPlanner {
    /// Extract full-text search predicate if present
    pub(super) fn extract_fulltext_predicate(
        &self,
        predicates: &[CanonicalPredicate],
    ) -> Option<(String, String, usize)> {
        // Look for FULLTEXT_MATCH(query, language) function call in predicates
        for pred in predicates {
            if let CanonicalPredicate::Other(expr) = pred {
                // Check if this is a FULLTEXT_MATCH function call
                if let Expr::Function { name, args, .. } = &expr.expr {
                    if name.to_uppercase() == "FULLTEXT_MATCH" && args.len() == 2 {
                        // Extract query (first argument)
                        let query = if let Expr::Literal(Literal::Text(q)) = &args[0].expr {
                            Some(q.clone())
                        } else {
                            None
                        };

                        // Extract language (second argument)
                        let language = if let Expr::Literal(Literal::Text(lang)) = &args[1].expr {
                            Some(lang.clone())
                        } else {
                            None
                        };

                        if let (Some(q), Some(lang)) = (query, language) {
                            return Some((lang, q, 1000)); // Default limit
                        }
                    }
                }
            }
        }
        None
    }

    /// Extract prefix predicate (PATH_STARTS_WITH)
    pub(super) fn extract_prefix_predicate(
        &self,
        predicates: &[CanonicalPredicate],
    ) -> Option<String> {
        for pred in predicates {
            if let CanonicalPredicate::PrefixRange { prefix, .. } = pred {
                return Some(prefix.clone());
            }
        }
        None
    }

    /// Extract path equality predicate for PathIndexScan
    ///
    /// Returns the exact path value for predicates like: path = '/exact/path'
    pub(super) fn extract_path_predicate(
        &self,
        predicates: &[CanonicalPredicate],
    ) -> Option<String> {
        for pred in predicates {
            match pred {
                CanonicalPredicate::ColumnEq { column, value, .. }
                    if column.to_lowercase() == "path" =>
                {
                    // Extract string value from literal
                    if let raisin_sql::analyzer::Expr::Literal(
                        raisin_sql::analyzer::Literal::Text(s),
                    ) = &value.expr
                    {
                        return Some(s.clone());
                    }
                }
                _ => continue,
            }
        }
        None
    }

    /// Extract id equality predicate for NodeIdScan
    ///
    /// Returns the exact node ID value for predicates like: id = 'uuid'
    pub(super) fn extract_id_predicate(&self, predicates: &[CanonicalPredicate]) -> Option<String> {
        for pred in predicates {
            match pred {
                CanonicalPredicate::ColumnEq { column, value, .. }
                    if column.to_lowercase() == "id" =>
                {
                    // Extract string value from literal
                    if let raisin_sql::analyzer::Expr::Literal(
                        raisin_sql::analyzer::Literal::Text(s),
                    ) = &value.expr
                    {
                        return Some(s.clone());
                    }
                }
                _ => continue,
            }
        }
        None
    }

    /// Extract property index predicate
    ///
    /// Returns (property_name, value) tuple for predicates that can use property index:
    /// - JSON properties: properties->>'key' = 'value' → ("key", "value")
    /// - node_type column: node_type = 'value' → ("__node_type", "value")
    pub(super) fn extract_property_predicate(
        &self,
        predicates: &[CanonicalPredicate],
    ) -> Option<(String, String)> {
        for pred in predicates {
            match pred {
                // JSON property extraction: properties->>'key' = 'value'
                CanonicalPredicate::JsonPropertyEq { key, value, .. } => {
                    // Extract raw value from serde_json::Value (not JSON-encoded)
                    // Note: value.to_string() would add quotes around strings!
                    let value_str = match value {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::Bool(b) => b.to_string(),
                        serde_json::Value::Null => "null".to_string(),
                        // For arrays/objects, fall back to JSON string (rare case)
                        _ => value.to_string(),
                    };
                    return Some((key.clone(), value_str));
                }
                // node_type column: node_type = 'value'
                // This is indexed as __node_type pseudo-property in RocksDB
                CanonicalPredicate::ColumnEq { column, value, .. }
                    if column.to_lowercase() == "node_type" =>
                {
                    // Extract string value from literal
                    if let raisin_sql::analyzer::Expr::Literal(
                        raisin_sql::analyzer::Literal::Text(s),
                    ) = &value.expr
                    {
                        return Some(("__node_type".to_string(), s.clone()));
                    }
                }
                // created_at column: created_at = now() or created_at = '2024-01-01'
                // This is indexed as __created_at pseudo-property in RocksDB
                CanonicalPredicate::ColumnEq { column, value, .. }
                    if column.to_lowercase() == "created_at"
                        || column.to_lowercase() == "updated_at" =>
                {
                    let col_lower = column.to_lowercase();
                    let prop_name = if col_lower == "created_at" {
                        "__created_at"
                    } else {
                        "__updated_at"
                    };

                    // Try to evaluate the value (handles now() and other constant expressions)
                    if let Some(lit) = self.evaluate_constant_expr(value) {
                        let prop_value = match lit {
                            Literal::Timestamp(ts) => {
                                let nanos = ts.timestamp_nanos_opt().unwrap_or(0);
                                format!("{:020}", nanos as i128)
                            }
                            _ => continue,
                        };
                        return Some((prop_name.to_string(), prop_value));
                    }
                }
                _ => continue,
            }
        }
        None
    }

    /// Remove prefix predicate from list
    pub(super) fn remove_prefix_predicate(
        &self,
        predicates: &[CanonicalPredicate],
    ) -> Vec<CanonicalPredicate> {
        predicates
            .iter()
            .filter(|p| !matches!(p, CanonicalPredicate::PrefixRange { .. }))
            .cloned()
            .collect()
    }

    /// Remove path equality predicate from list
    pub(super) fn remove_path_predicate(
        &self,
        predicates: &[CanonicalPredicate],
    ) -> Vec<CanonicalPredicate> {
        predicates
            .iter()
            .filter(|p| {
                !matches!(
                    p,
                    CanonicalPredicate::ColumnEq { column, .. } if column.to_lowercase() == "path"
                )
            })
            .cloned()
            .collect()
    }

    /// Remove id equality predicate from list
    pub(super) fn remove_id_predicate(
        &self,
        predicates: &[CanonicalPredicate],
    ) -> Vec<CanonicalPredicate> {
        predicates
            .iter()
            .filter(|p| {
                !matches!(
                    p,
                    CanonicalPredicate::ColumnEq { column, .. } if column.to_lowercase() == "id"
                )
            })
            .cloned()
            .collect()
    }

    /// Remove specific property predicate from list
    pub(super) fn remove_property_predicate(
        &self,
        predicates: &[CanonicalPredicate],
        prop_name: &str,
    ) -> Vec<CanonicalPredicate> {
        predicates
            .iter()
            .filter(|p| {
                match p {
                    // Remove JSON property predicates matching the property name
                    CanonicalPredicate::JsonPropertyEq { key, .. } if key == prop_name => false,
                    // Remove node_type column predicate if we're looking for __node_type
                    CanonicalPredicate::ColumnEq { column, .. }
                        if column.to_lowercase() == "node_type" && prop_name == "__node_type" =>
                    {
                        false
                    }
                    // Remove created_at/updated_at predicates
                    CanonicalPredicate::ColumnEq { column, .. }
                        if (column.to_lowercase() == "created_at"
                            && prop_name == "__created_at")
                            || (column.to_lowercase() == "updated_at"
                                && prop_name == "__updated_at") =>
                    {
                        false
                    }
                    _ => true,
                }
            })
            .cloned()
            .collect()
    }

    /// Combine canonical predicates back into a filter expression
    pub(super) fn combine_canonical_predicates(
        &self,
        predicates: &[CanonicalPredicate],
    ) -> Option<TypedExpr> {
        if predicates.is_empty() {
            return None;
        }

        // Filter out scan-level predicates that define the scan boundary and are
        // inherently satisfied by the scan itself (e.g., DescendantOf is guaranteed
        // by PrefixScan). Each build_*_scan method removes its own predicate from
        // remaining, but this filter acts as a safety net.
        //
        // NOTE: SpatialDWithin is intentionally NOT filtered here. Two paths exist:
        //
        // 1. Spatial scan selected (build_spatial_scan): SpatialDWithin is removed
        //    from `remaining` explicitly in build_scan.rs, so it never reaches this
        //    function. The spatial index handles distance filtering.
        //
        // 2. Non-spatial scan selected (e.g., DescendantOf wins by selectivity):
        //    SpatialDWithin stays in `remaining` and is converted back to an
        //    ST_DWithin(...) expression via `to_expr()`, then applied as a row-level
        //    filter. This ensures correctness — rows outside the radius are excluded
        //    even when the spatial index is not the primary scan method.
        let filter_predicates: Vec<_> = predicates
            .iter()
            .filter(|p| {
                !matches!(
                    p,
                    CanonicalPredicate::ChildOf { .. }
                        | CanonicalPredicate::PrefixRange { .. }
                        | CanonicalPredicate::DescendantOf { .. }
                        | CanonicalPredicate::References { .. }
                )
            })
            .collect();

        if filter_predicates.is_empty() {
            return None;
        }

        let exprs: Vec<TypedExpr> = filter_predicates.iter().map(|p| p.to_expr()).collect();

        if exprs.len() == 1 {
            return Some(exprs[0].clone());
        }

        // Combine with AND
        let mut result = exprs[0].clone();
        for expr in &exprs[1..] {
            result = TypedExpr::new(
                Expr::BinaryOp {
                    left: Box::new(result),
                    op: BinaryOperator::And,
                    right: Box::new(expr.clone()),
                },
                DataType::Boolean,
            );
        }

        Some(result)
    }

    /// Combine multiple predicate expressions with AND
    pub(super) fn combine_predicates(&self, predicates: &[TypedExpr]) -> TypedExpr {
        if predicates.is_empty() {
            // Return TRUE literal if no predicates
            return TypedExpr::new(Expr::Literal(Literal::Boolean(true)), DataType::Boolean);
        }

        if predicates.len() == 1 {
            return predicates[0].clone();
        }

        // Combine with AND
        let mut result = predicates[0].clone();
        for expr in &predicates[1..] {
            result = TypedExpr::new(
                Expr::BinaryOp {
                    left: Box::new(result),
                    op: BinaryOperator::And,
                    right: Box::new(expr.clone()),
                },
                DataType::Boolean,
            );
        }

        result
    }

    /// Check if a predicate is simple enough for VectorScan optimization
    ///
    /// Simple predicates are those that can be efficiently applied BEFORE
    /// vector search in the HNSW index, or are trivial checks. Complex
    /// predicates require full scan + embedding population.
    ///
    /// Simple predicates:
    /// - `embedding IS NOT NULL` - Checks if node has embedding
    /// - `path = 'constant'` - Exact path match
    /// - `id = 'constant'` - Exact ID match
    /// - Distance threshold: `distance_expr < 0.5` (handled by VectorScan)
    ///
    /// Complex predicates (NOT simple):
    /// - `path STARTS WITH '/docs'` - Requires prefix scan
    /// - `properties->>'status' = 'published'` - Requires property lookup
    /// - `DEPTH(path) > 2` - Requires computation
    /// - Anything with OR, NOT, or complex expressions
    pub(super) fn is_simple_predicate(&self, expr: &TypedExpr) -> bool {
        match &expr.expr {
            // IS NULL / IS NOT NULL on embedding column
            Expr::IsNull { expr } | Expr::IsNotNull { expr } => {
                matches!(
                    &expr.expr,
                    Expr::Column { column, .. } if column == "embedding"
                )
            }

            // Simple equality: column = literal
            Expr::BinaryOp {
                left,
                op: BinaryOperator::Eq,
                right,
            } => {
                // Left must be a simple column reference
                let is_simple_column = matches!(
                    &left.expr,
                    Expr::Column { column, .. } if matches!(column.as_str(), "id" | "path")
                );

                // Right must be a literal (not an expression)
                let is_literal = matches!(&right.expr, Expr::Literal(_));

                is_simple_column && is_literal
            }

            // Distance threshold comparisons: distance_expr < literal or column_alias < literal
            // These are handled by VectorScan's max_distance parameter
            Expr::BinaryOp {
                left,
                op: BinaryOperator::Lt | BinaryOperator::LtEq,
                right,
            } => {
                let is_numeric_literal = Self::is_numeric_literal(right);
                let is_distance_expr = Self::is_vector_distance_expr(left)
                    || matches!(&left.expr, Expr::Column { .. });
                is_distance_expr && is_numeric_literal
            }

            // Also handle reversed form: literal > distance_expr
            Expr::BinaryOp {
                left,
                op: BinaryOperator::Gt | BinaryOperator::GtEq,
                right,
            } => {
                let is_numeric_literal = Self::is_numeric_literal(left);
                let is_distance_expr = Self::is_vector_distance_expr(right)
                    || matches!(&right.expr, Expr::Column { .. });
                is_distance_expr && is_numeric_literal
            }

            // Everything else is complex
            _ => false,
        }
    }

    /// Check if an expression is a vector distance operator or function
    fn is_vector_distance_expr(expr: &TypedExpr) -> bool {
        match &expr.expr {
            Expr::BinaryOp { op, .. } => matches!(
                op,
                BinaryOperator::VectorL2Distance
                    | BinaryOperator::VectorCosineDistance
                    | BinaryOperator::VectorInnerProduct
            ),
            Expr::Function { name, .. } => matches!(
                name.to_uppercase().as_str(),
                "VECTOR_L2_DISTANCE" | "VECTOR_COSINE_DISTANCE" | "VECTOR_INNER_PRODUCT"
            ),
            _ => false,
        }
    }

    /// Check if an expression is a numeric literal
    fn is_numeric_literal(expr: &TypedExpr) -> bool {
        matches!(
            &expr.expr,
            Expr::Literal(Literal::Double(_))
                | Expr::Literal(Literal::Int(_))
                | Expr::Literal(Literal::BigInt(_))
        )
    }
}
