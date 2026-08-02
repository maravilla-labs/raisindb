//! Compound index matching and constant expression evaluation

use super::{
    CanonicalPredicate, CompoundIndexDefinition, Error, Expr, Literal, PhysicalPlanner,
    SchemaStats, TypedExpr,
};

/// Result of a successful compound index match:
/// (index_name, matched_equality_columns, ascending, claims_order)
///
/// `claims_order` is true when ALL equality columns matched and the trailing
/// order column satisfies the query's ORDER BY — only then may the scan's
/// iteration order be relied upon (e.g. for LIMIT pushdown under an ORDER BY).
pub type CompoundIndexMatch = (String, Vec<(String, String)>, bool, bool);

/// Normalise a `CHILD_OF` argument to the exact string the index writer stores
/// for `__parent_path`.
///
/// The writer uses `Node::parent_path()`, which yields no trailing slash and
/// `"/"` for a root-level node. ONE implementation, called from both the match
/// (which builds the equality value) and the residual-filter pruning (which
/// decides whether a ChildOf is already guaranteed) — if those two normalised
/// differently the index would either never match, or would drop a predicate it
/// does not actually enforce.
pub(in crate::physical_plan::planner) fn normalize_parent_path(parent_path: &str) -> &str {
    let trimmed = parent_path.trim_end_matches('/');
    if trimmed.is_empty() {
        "/"
    } else {
        trimmed
    }
}

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
    /// Returns Some((index_name, equality_columns, ascending, claims_order)) if a
    /// compound index matches, None otherwise.
    ///
    /// Matching rules:
    /// 1. A LEADING PREFIX (>= 1) of the index's equality columns must have
    ///    matching equality predicates. Full prefix matches are preferred over
    ///    partial ones; among equals, more matched columns win.
    /// 2. `claims_order` is true only when ALL equality columns matched AND the
    ///    trailing order column satisfies the query's ORDER BY — a partial match
    ///    iterates in residual-column order, which is NOT the ORDER BY order.
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
                // CHILD_OF is an EQUALITY on the containing directory, which is
                // what lets hierarchy lead a sorted index and still leave the
                // trailing column free to serve the ORDER BY. DESCENDANT_OF is
                // deliberately absent: a subtree is a path RANGE, and a range
                // cannot precede an order column in any sorted index — within
                // each distinct path the order column is sorted, but not across
                // the range. Serving that needs a materialised ancestor column.
                //
                // Normalised to match the writer, which stores
                // `Node::parent_path()`: no trailing slash, and root is "/".
                CanonicalPredicate::ChildOf { parent_path } => {
                    equality_map.insert(
                        "__parent_path".to_string(),
                        normalize_parent_path(parent_path).to_string(),
                    );
                }
                _ => {}
            }
        }

        if equality_map.is_empty() {
            return None;
        }

        // Track the best candidate: full matches beat partial ones, then more
        // matched columns win.
        let mut best: Option<(CompoundIndexMatch, bool, usize)> = None; // (match, full, count)

        for index in &self.compound_indexes {
            let equality_column_count = if index.has_order_column {
                index.columns.len().saturating_sub(1)
            } else {
                index.columns.len()
            };

            // Match a leading prefix of the equality columns.
            let mut matched_columns: Vec<(String, String)> = Vec::new();
            for i in 0..equality_column_count {
                let col = &index.columns[i];
                if let Some(value) = equality_map.get(&col.property) {
                    matched_columns.push((col.property.clone(), value.clone()));
                } else {
                    break;
                }
            }

            if matched_columns.is_empty() {
                continue;
            }
            let full = matched_columns.len() == equality_column_count;

            // Order can only be claimed on a FULL equality match — a partial
            // match iterates in unmatched-column order, not order-column order.
            let (ascending, claims_order) = if full && index.has_order_column {
                if let Some((order_col, is_asc)) = order_by {
                    let order_column = &index.columns[index.columns.len() - 1];
                    let order_col_normalized = if order_col.eq_ignore_ascii_case("created_at") {
                        "__created_at"
                    } else if order_col.eq_ignore_ascii_case("updated_at") {
                        "__updated_at"
                    } else {
                        order_col
                    };

                    if order_column.property == order_col_normalized {
                        (is_asc, true)
                    } else {
                        // Index can't serve this ORDER BY; still usable as an
                        // unordered equality filter (Sort above re-orders).
                        (true, false)
                    }
                } else {
                    (false, false)
                }
            } else {
                (true, false)
            };

            let candidate = (index.name.clone(), matched_columns, ascending, claims_order);
            let count = candidate.1.len();

            let better = match &best {
                None => true,
                Some((_, best_full, best_count)) => {
                    (full && !best_full) || (full == *best_full && count > *best_count)
                }
            };
            if better {
                best = Some((candidate, full, count));
            }
        }

        if let Some(((name, cols, asc, claims_order), full, count)) = best {
            tracing::info!(
                "   Matched compound index '{}' with {} equality columns (full={}, claims_order={})",
                name,
                count,
                full,
                claims_order
            );
            return Some((name, cols, asc, claims_order));
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
