//! Predicate extraction, removal, and combination utilities
//!
//! Provides methods to extract specific predicate types from canonicalized
//! predicates and to combine/remove predicates.

use super::{CanonicalPredicate, ComparisonOp, Expr, Literal, PhysicalPlanner, TypedExpr};
use raisin_sql::analyzer::{BinaryOperator, DataType};

impl PhysicalPlanner {
    /// Locate a full-text search predicate.
    ///
    /// Returns `(position, language, query, limit)`. The POSITION is part of the
    /// answer on purpose: the caller must strip exactly this predicate from the
    /// residual filter and keep every other one. A separate
    /// `remove_fulltext_predicate` would be a second matcher free to drift from
    /// this one, and the two disagreeing is how a predicate ends up applied
    /// nowhere.
    pub(super) fn find_fulltext_predicate(
        &self,
        predicates: &[CanonicalPredicate],
    ) -> Option<(usize, String, String, usize)> {
        // Look for FULLTEXT_MATCH(query, language) function call in predicates
        for (position, pred) in predicates.iter().enumerate() {
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
                            return Some((position, lang, q, 1000)); // Default limit
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
                // Pseudo-property columns indexed by index_node_properties:
                // node_type, archetype, name, created_by, updated_by → the
                // corresponding __-prefixed entry in the property index.
                CanonicalPredicate::ColumnEq { column, value, .. }
                    if matches!(
                        column.to_lowercase().as_str(),
                        "node_type" | "archetype" | "name" | "created_by" | "updated_by"
                    ) =>
                {
                    // Extract string value from literal
                    if let raisin_sql::analyzer::Expr::Literal(
                        raisin_sql::analyzer::Literal::Text(s),
                    ) = &value.expr
                    {
                        let prop_name = format!("__{}", column.to_lowercase());
                        return Some((prop_name, s.clone()));
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

    /// Combine canonical predicates back into a filter expression
    pub(super) fn combine_canonical_predicates(
        &self,
        predicates: &[CanonicalPredicate],
    ) -> Option<TypedExpr> {
        if predicates.is_empty() {
            return None;
        }

        // EVERY predicate handed to this function becomes part of the row-level
        // residual filter — including hierarchy predicates (ChildOf, PrefixRange,
        // DescendantOf, References), which round-trip through `to_expr()` to
        // row-evaluable CHILD_OF / PATH_STARTS_WITH / DESCENDANT_OF / REFERENCES
        // calls.
        //
        // Each build_*_scan method is responsible for removing exactly the
        // predicate its scan GUARANTEES (and nothing else) from `remaining`
        // before calling this. Historically this function also dropped all
        // hierarchy predicates as a "safety net"; that silently discarded them
        // whenever a DIFFERENT scan won the access-path choice (e.g.
        // `WHERE path LIKE '/a/%' AND node_type = 'X'` planned as a
        // PropertyIndexScan returned every 'X' node across ALL subtrees).
        //
        // SpatialDWithin follows the same rule: build_spatial_scan removes it
        // when the spatial index drives the scan; otherwise it survives here as
        // a row-level ST_DWITHIN filter.
        let exprs: Vec<TypedExpr> = predicates.iter().map(|p| p.to_expr()).collect();

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

    /// Whether a VectorScan itself GUARANTEES this predicate, so it may be
    /// dropped from the residual filter.
    ///
    /// Exactly one thing qualifies: `embedding IS NOT NULL`. Membership of the
    /// HNSW index IS that predicate — a node with no vector is not a candidate.
    ///
    /// Nothing else belongs here. The distance threshold is handled separately
    /// (it becomes `max_distance`), and every other conjunct — `path = ...`,
    /// `node_type = ...`, a property equality — is applied as a residual filter
    /// above the scan. This used to be an `is_simple_predicate` gate that
    /// answered "can we use a VectorScan at all", which made a *performance*
    /// judgement decide *correctness*: a predicate it called simple was then
    /// consumed by nobody and silently vanished.
    pub(super) fn vector_scan_guarantees(expr: &TypedExpr) -> bool {
        matches!(
            &expr.expr,
            Expr::IsNotNull { expr } if matches!(
                &expr.expr,
                Expr::Column { column, .. } if column == "embedding"
            )
        )
    }
}
