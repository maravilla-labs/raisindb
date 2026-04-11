//! Compound index matching and constant expression evaluation

use super::{
    CanonicalPredicate, CompoundIndexDefinition, Error, Expr, Literal, PhysicalPlanner, SchemaStats,
    TypedExpr,
};

/// Result of a successful compound index match: (index_name, matched_equality_columns, ascending)
pub type CompoundIndexMatch = (String, Vec<(String, String)>, bool);

impl PhysicalPlanner {
    /// Extract a string literal argument for table functions
    pub(super) fn extract_string_literal(
        arg: Option<&TypedExpr>,
        function: &str,
        position: usize,
    ) -> Result<String, Error> {
        let expr = arg.ok_or_else(|| {
            Error::Validation(format!(
                "{} table function expects at least {} arguments",
                function,
                position + 1
            ))
        })?;

        match &expr.expr {
            Expr::Literal(Literal::Text(value)) | Expr::Literal(Literal::Path(value)) => {
                Ok(value.clone())
            }
            _ => Err(Error::Validation(format!(
                "Argument {} for table function {} must be a string literal",
                position + 1,
                function
            ))),
        }
    }

    /// Set compound indexes for the current query context
    ///
    /// This should be called before planning queries that may benefit from compound indexes.
    /// The indexes are typically loaded from NodeType schemas.
    pub fn set_compound_indexes(&mut self, indexes: Vec<CompoundIndexDefinition>) {
        self.compound_indexes = indexes;
    }

    /// Set pre-computed schema statistics for data-driven selectivity estimation.
    ///
    /// When set, equality predicates on `node_type` and `archetype` columns use
    /// `1 / count` instead of the default 0.05 heuristic. Call this before
    /// planning queries for best results.
    pub fn set_schema_statistics(&mut self, stats: SchemaStats) {
        self.schema_stats = Some(stats);
    }

    /// Try to match a compound index for the given query pattern
    ///
    /// Returns Some((index_name, equality_columns)) if a compound index matches,
    /// None otherwise.
    ///
    /// A compound index matches when:
    /// 1. All leading equality columns in the index have matching equality predicates
    /// 2. If ORDER BY is present, it matches the trailing ordering column
    pub(super) fn try_match_compound_index(
        &self,
        predicates: &[CanonicalPredicate],
        order_by: Option<(&str, bool)>,
    ) -> Option<CompoundIndexMatch> {
        let mut equality_map: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();

        for pred in predicates {
            match pred {
                CanonicalPredicate::ColumnEq { column, value, .. } => {
                    let prop_name = if column.eq_ignore_ascii_case("node_type") {
                        "__node_type".to_string()
                    } else {
                        column.clone()
                    };
                    if let Expr::Literal(lit) = &value.expr {
                        let value_str = match lit {
                            Literal::Text(s) => s.clone(),
                            Literal::Int(i) => i.to_string(),
                            Literal::BigInt(i) => i.to_string(),
                            Literal::Double(f) => f.to_string(),
                            Literal::Boolean(b) => b.to_string(),
                            _ => continue,
                        };
                        equality_map.insert(prop_name, value_str);
                    }
                }
                CanonicalPredicate::JsonPropertyEq { key, value, .. } => {
                    let value_str = match value {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::Bool(b) => b.to_string(),
                        _ => continue,
                    };
                    equality_map.insert(key.clone(), value_str);
                }
                _ => {}
            }
        }

        if equality_map.is_empty() {
            return None;
        }

        for index in &self.compound_indexes {
            let equality_column_count = if index.has_order_column {
                index.columns.len().saturating_sub(1)
            } else {
                index.columns.len()
            };

            let mut matched_columns: Vec<(String, String)> = Vec::new();
            let mut all_match = true;

            for i in 0..equality_column_count {
                let col = &index.columns[i];
                if let Some(value) = equality_map.get(&col.property) {
                    matched_columns.push((col.property.clone(), value.clone()));
                } else {
                    all_match = false;
                    break;
                }
            }

            if !all_match {
                continue;
            }

            let ascending = if index.has_order_column {
                if let Some((order_col, is_asc)) = order_by {
                    let order_column = &index.columns[index.columns.len() - 1];
                    let order_col_normalized = if order_col.eq_ignore_ascii_case("created_at") {
                        "__created_at"
                    } else if order_col.eq_ignore_ascii_case("updated_at") {
                        "__updated_at"
                    } else {
                        order_col
                    };

                    if order_column.property != order_col_normalized {
                        continue;
                    }

                    is_asc
                } else {
                    false
                }
            } else {
                true
            };

            tracing::info!(
                "   Matched compound index '{}' with {} equality columns",
                index.name,
                matched_columns.len()
            );

            return Some((index.name.clone(), matched_columns, ascending));
        }

        None
    }

    /// Check if an expression can be evaluated at plan time (without row context)
    ///
    /// Returns true for literals, temporal functions (NOW(), CURRENT_TIMESTAMP),
    /// and arithmetic on constant expressions.
    #[allow(clippy::only_used_in_recursion)]
    pub(super) fn is_constant_expr(&self, expr: &TypedExpr) -> bool {
        match &expr.expr {
            Expr::Literal(_) => true,
            Expr::Function { name, args, .. } => {
                let name_upper = name.to_uppercase();
                let is_constant_fn = matches!(
                    name_upper.as_str(),
                    "NOW" | "CURRENT_TIMESTAMP" | "CURRENT_DATE" | "CURRENT_TIME"
                );
                is_constant_fn && args.iter().all(|a| self.is_constant_expr(a))
            }
            Expr::BinaryOp { left, right, .. } => {
                self.is_constant_expr(left) && self.is_constant_expr(right)
            }
            Expr::UnaryOp { expr, .. } => self.is_constant_expr(expr),
            Expr::Column { .. } => false,
            _ => false,
        }
    }

    /// Evaluate a constant expression to a Literal at planning time
    ///
    /// Returns None if the expression cannot be evaluated at plan time.
    pub(super) fn evaluate_constant_expr(&self, expr: &TypedExpr) -> Option<Literal> {
        match &expr.expr {
            Expr::Literal(lit) => Some(lit.clone()),
            Expr::Function { name, .. } => {
                let name_upper = name.to_uppercase();
                match name_upper.as_str() {
                    "NOW" | "CURRENT_TIMESTAMP" => Some(Literal::Timestamp(chrono::Utc::now())),
                    "CURRENT_DATE" => {
                        let today = chrono::Utc::now().date_naive();
                        let datetime = today.and_hms_opt(0, 0, 0)?;
                        Some(Literal::Timestamp(
                            chrono::DateTime::from_naive_utc_and_offset(datetime, chrono::Utc),
                        ))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }
}
