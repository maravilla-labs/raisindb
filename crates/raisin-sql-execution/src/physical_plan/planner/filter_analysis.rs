//! Filter and predicate analysis
//!
//! Canonicalizes filter expressions into structured predicates and provides
//! utilities for extracting, removing, and combining predicates.

use super::{
    literal_to_json, CanonicalPredicate, ComparisonOp, Error, Expr, Literal, PhysicalPlanner,
    ScanReason, TypedExpr,
};
use raisin_sql::analyzer::{BinaryOperator, DataType};

impl PhysicalPlanner {
    pub(super) fn determine_scan_reason(&self, canonical: &[CanonicalPredicate]) -> ScanReason {
        let available = self.index_catalog.available_indexes();

        if available.is_empty() {
            return ScanReason::NoIndexAvailable;
        }

        // Check if we have predicates that could use indexes
        let has_prefix = canonical
            .iter()
            .any(|p| matches!(p, CanonicalPredicate::PrefixRange { .. }));
        let has_child_of = canonical
            .iter()
            .any(|p| matches!(p, CanonicalPredicate::ChildOf { .. }));
        let has_descendant_of = canonical
            .iter()
            .any(|p| matches!(p, CanonicalPredicate::DescendantOf { .. }));
        let has_property = canonical
            .iter()
            .any(|p| matches!(p, CanonicalPredicate::JsonPropertyEq { .. }));

        // Check path index requirements for descendant_of
        if has_descendant_of && !self.index_catalog.has_path_index() {
            return ScanReason::IndexNotEnabled {
                index_name: "path_index".to_string(),
            };
        }
        let _ = has_child_of; // Suppress unused warning - CHILD_OF uses ordered_children which is always available

        if has_prefix && !self.index_catalog.has_path_index() {
            return ScanReason::IndexNotEnabled {
                index_name: "path_index".to_string(),
            };
        }

        if has_property && !self.index_catalog.has_property_index() {
            return ScanReason::IndexNotEnabled {
                index_name: "property_index".to_string(),
            };
        }

        // No predicates that can use indexes
        if canonical
            .iter()
            .all(|p| matches!(p, CanonicalPredicate::Other(_)))
        {
            return ScanReason::UnsupportedPredicate {
                details: "no indexable predicates found".to_string(),
            };
        }

        ScanReason::NoMatchingIndex { available }
    }

    /// Analyze filter expression into canonical predicates
    pub(super) fn analyze_filter(
        &self,
        filter: &TypedExpr,
    ) -> Result<Vec<CanonicalPredicate>, Error> {
        // For now, we'll do simple pattern matching
        // In a full implementation, this would use the optimizer's hierarchy_rewrite module

        let mut predicates = Vec::new();

        // Flatten AND operations
        let conjuncts = self.flatten_ands(filter);

        for conjunct in conjuncts {
            // BETWEEN desugars to TWO range predicates (>= low AND <= high), so a
            // `created_at BETWEEN a AND b` can use the same PropertyRangeScan as
            // the explicit two-sided range form.
            if let Some((low_pred, high_pred)) = self.match_between_predicate(&conjunct) {
                predicates.push(low_pred);
                predicates.push(high_pred);
                continue;
            }
            if let Some(pred) = self.match_canonical_predicate(&conjunct) {
                predicates.push(pred);
            } else {
                predicates.push(CanonicalPredicate::Other(conjunct));
            }
        }

        Ok(predicates)
    }

    /// Desugar `col BETWEEN low AND high` into `col >= low` + `col <= high`
    /// range predicates when the operand is a plain column and both bounds are
    /// constant expressions.
    fn match_between_predicate(
        &self,
        expr: &TypedExpr,
    ) -> Option<(CanonicalPredicate, CanonicalPredicate)> {
        if let Expr::Between {
            expr: operand,
            low,
            high,
        } = &expr.expr
        {
            if let Expr::Column { table, column } = &operand.expr {
                if self.is_constant_expr(low) && self.is_constant_expr(high) {
                    return Some((
                        CanonicalPredicate::RangeCompare {
                            table: table.clone(),
                            column: column.clone(),
                            op: ComparisonOp::GtEq,
                            value: (**low).clone(),
                        },
                        CanonicalPredicate::RangeCompare {
                            table: table.clone(),
                            column: column.clone(),
                            op: ComparisonOp::LtEq,
                            value: (**high).clone(),
                        },
                    ));
                }
            }
        }
        None
    }

    /// Match a single expression to a canonical predicate
    pub(super) fn match_canonical_predicate(&self, expr: &TypedExpr) -> Option<CanonicalPredicate> {
        // Spatial shapes are recognised by the SINGLE extractor in
        // `raisin_sql::optimizer::hierarchy_rewrite::spatial`, shared with the
        // optimizer's `rewrite_hierarchy_predicates`. This used to be ~75 lines
        // duplicated byte-for-byte between the two, which is precisely how one
        // copy came to accept a spelling the other rejected. It runs first so
        // `ST_DISTANCE(...) < r` is recognised before the generic comparison arms
        // reach it.
        if let Some(spatial) =
            raisin_sql::optimizer::hierarchy_rewrite::extract_spatial_predicate(expr)
        {
            return Some(spatial);
        }

        match &expr.expr {
            // PATH_STARTS_WITH(path, prefix)
            Expr::Function { name, args, .. } if name.to_uppercase() == "PATH_STARTS_WITH" => {
                if args.len() == 2 {
                    if let (Expr::Column { table, column }, Expr::Literal(Literal::Path(prefix)))
                    | (Expr::Column { table, column }, Expr::Literal(Literal::Text(prefix))) =
                        (&args[0].expr, &args[1].expr)
                    {
                        return Some(CanonicalPredicate::PrefixRange {
                            table: table.clone(),
                            path_col: column.clone(),
                            prefix: prefix.clone(),
                        });
                    }
                }
            }

            // CHILD_OF(parent_path) - direct children scan
            Expr::Function { name, args, .. } if name.to_uppercase() == "CHILD_OF" => {
                if args.len() == 1 {
                    if let Expr::Literal(lit) = &args[0].expr {
                        let parent_path = match lit {
                            Literal::Path(p) => p.clone(),
                            Literal::Text(t) => t.clone(),
                            _ => return None,
                        };
                        return Some(CanonicalPredicate::ChildOf { parent_path });
                    }
                }
            }

            // DESCENDANT_OF(parent_path [, max_depth]) - descendants scan
            Expr::Function { name, args, .. } if name.to_uppercase() == "DESCENDANT_OF" => {
                if !args.is_empty() && args.len() <= 2 {
                    if let Expr::Literal(lit) = &args[0].expr {
                        let parent_path = match lit {
                            Literal::Path(p) => p.clone(),
                            Literal::Text(t) => t.clone(),
                            _ => return None,
                        };

                        // Extract optional max_depth parameter
                        let max_depth = if args.len() == 2 {
                            match &args[1].expr {
                                Expr::Literal(Literal::Int(n)) => Some(*n as i64),
                                Expr::Literal(Literal::BigInt(n)) => Some(*n),
                                Expr::Literal(Literal::Null) => None,
                                _ => return None,
                            }
                        } else {
                            None
                        };

                        return Some(CanonicalPredicate::DescendantOf {
                            parent_path,
                            max_depth,
                        });
                    }
                }
            }

            // REFERENCES('workspace:/path') - inbound reference scan via the reverse
            // reference index. Canonicalizing it HERE (the execution planner) is what
            // makes ReferenceIndexScan actually get selected; without this arm the
            // predicate stays `Other` and falls back to a row-eval post-filter that
            // silently needs `properties` materialized in the row.
            Expr::Function { name, args, .. } if name.to_uppercase() == "REFERENCES" => {
                if args.len() == 1 {
                    if let Expr::Literal(Literal::Text(target)) = &args[0].expr {
                        if let Some((ws, path)) = target.split_once(':') {
                            return Some(CanonicalPredicate::References {
                                target_workspace: ws.to_string(),
                                target_path: path.to_string(),
                            });
                        }
                    }
                }
            }

            // DEPTH(path) = value
            Expr::BinaryOp {
                left,
                op: BinaryOperator::Eq,
                right,
            } => {
                if let Expr::Function { name, args, .. } = &left.expr {
                    if name.to_uppercase() == "DEPTH" && args.len() == 1 {
                        if let Expr::Column { table, column } = &args[0].expr {
                            if let Expr::Literal(Literal::Int(depth_val)) = &right.expr {
                                return Some(CanonicalPredicate::DepthEq {
                                    table: table.clone(),
                                    path_col: column.clone(),
                                    depth_value: *depth_val,
                                });
                            }
                        }
                    }
                }

                // JSON property: properties->>'key' = 'value'
                //
                // NOTE: only the *bare* `properties->>'key'` form is canonicalized to
                // JsonPropertyEq here. The documented cast form
                // `properties->>'key'::String = 'value'` is intentionally left as
                // `Other` (a verbatim row-level filter). Canonicalizing the cast form
                // would let it match a compound index, which only returns correct
                // results when that index is populated —
                // an unbuilt/stale compound index would silently return zero rows.
                // Keeping the cast form as `Other` preserves the full-scan + verbatim
                // filter path callers rely on for correctness.
                if let Expr::JsonExtractText { object, key } = &left.expr {
                    if let (Expr::Column { table, column }, Expr::Literal(Literal::Text(key_str))) =
                        (&object.expr, &key.expr)
                    {
                        if let Expr::Literal(lit) = &right.expr {
                            if let Ok(json_val) = literal_to_json(lit) {
                                return Some(CanonicalPredicate::JsonPropertyEq {
                                    table: table.clone(),
                                    json_col: column.clone(),
                                    key: key_str.clone(),
                                    value: json_val,
                                });
                            }
                        }
                    }
                }

                // JSON property via $.syntax: $.properties.key::TEXT = 'value'
                // This is Cast { expr: JsonExtractPath { object, path }, target_type: Text }
                if let Expr::Cast {
                    expr: cast_expr,
                    target_type: DataType::Text,
                } = &left.expr
                {
                    if let Expr::JsonExtractPath { object, path } = &cast_expr.expr {
                        // Check if object is Cast { Column { properties }, target: JsonB }
                        if let Expr::Cast {
                            expr: inner_expr,
                            target_type: DataType::JsonB,
                        } = &object.expr
                        {
                            if let Expr::Column { table, column } = &inner_expr.expr {
                                // Extract the path key - for single-element paths like $.properties.email
                                // the path is a JsonB literal containing an Array of strings
                                if let Expr::Literal(Literal::JsonB(serde_json::Value::Array(
                                    elements,
                                ))) = &path.expr
                                {
                                    if elements.len() == 1 {
                                        if let serde_json::Value::String(key_str) = &elements[0] {
                                            if let Expr::Literal(lit) = &right.expr {
                                                if let Ok(json_val) = literal_to_json(lit) {
                                                    return Some(
                                                        CanonicalPredicate::JsonPropertyEq {
                                                            table: table.clone(),
                                                            json_col: column.clone(),
                                                            key: key_str.clone(),
                                                            value: json_val,
                                                        },
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // column = value (literal)
                if let (Expr::Column { table, column }, Expr::Literal(_)) =
                    (&left.expr, &right.expr)
                {
                    return Some(CanonicalPredicate::ColumnEq {
                        table: table.clone(),
                        column: column.clone(),
                        value: (**right).clone(),
                    });
                }

                // literal = column (reverse case)
                if let (Expr::Literal(_), Expr::Column { table, column }) =
                    (&left.expr, &right.expr)
                {
                    return Some(CanonicalPredicate::ColumnEq {
                        table: table.clone(),
                        column: column.clone(),
                        value: (**left).clone(),
                    });
                }

                // column = constant_expr (e.g., created_at = now())
                // Handle timestamp columns with constant expressions
                if let Expr::Column { table, column } = &left.expr {
                    let col_lower = column.to_lowercase();
                    if (col_lower == "created_at" || col_lower == "updated_at")
                        && self.is_constant_expr(right)
                    {
                        return Some(CanonicalPredicate::ColumnEq {
                            table: table.clone(),
                            column: column.clone(),
                            value: (**right).clone(),
                        });
                    }
                }

                // constant_expr = column (e.g., now() = created_at) - reverse case
                // Handle timestamp columns with constant expressions
                if let Expr::Column { table, column } = &right.expr {
                    let col_lower = column.to_lowercase();
                    if (col_lower == "created_at" || col_lower == "updated_at")
                        && self.is_constant_expr(left)
                    {
                        return Some(CanonicalPredicate::ColumnEq {
                            table: table.clone(),
                            column: column.clone(),
                            value: (**left).clone(),
                        });
                    }
                }
            }

            // Comparison operators: >, <, >=, <= → RangeCompare
            // Supports both literals and constant expressions like now()
            Expr::BinaryOp {
                left,
                op:
                    op @ (BinaryOperator::Gt
                    | BinaryOperator::GtEq
                    | BinaryOperator::Lt
                    | BinaryOperator::LtEq),
                right,
            } => {
                if let Some(comp_op) = ComparisonOp::from_binary_op(op) {
                    // Pattern: column OP value (e.g., created_at > now())
                    if let Expr::Column { table, column } = &left.expr {
                        // Check if right side is a constant expression
                        if self.is_constant_expr(right) {
                            return Some(CanonicalPredicate::RangeCompare {
                                table: table.clone(),
                                column: column.clone(),
                                op: comp_op,
                                value: (**right).clone(),
                            });
                        }
                    }

                    // Pattern: value OP column (e.g., now() < created_at) - reverse the operator
                    if let Expr::Column { table, column } = &right.expr {
                        if self.is_constant_expr(left) {
                            return Some(CanonicalPredicate::RangeCompare {
                                table: table.clone(),
                                column: column.clone(),
                                op: comp_op.reverse(),
                                value: (**left).clone(),
                            });
                        }
                    }

                    // Pattern: properties->>'key' OP 'text' — lexicographic JSON
                    // property range (only text literals: the property index
                    // stores raw strings and `->>` yields text, so index order
                    // matches row-eval order).
                    if let Some((table, json_col, key)) = Self::match_json_key_extract(&left.expr) {
                        if let Expr::Literal(Literal::Text(v)) = &right.expr {
                            return Some(CanonicalPredicate::JsonPropertyRange {
                                table,
                                json_col,
                                key,
                                op: comp_op,
                                value: v.clone(),
                            });
                        }
                    }

                    // Reverse: 'text' OP properties->>'key'
                    if let Some((table, json_col, key)) = Self::match_json_key_extract(&right.expr)
                    {
                        if let Expr::Literal(Literal::Text(v)) = &left.expr {
                            return Some(CanonicalPredicate::JsonPropertyRange {
                                table,
                                json_col,
                                key,
                                op: comp_op.reverse(),
                                value: v.clone(),
                            });
                        }
                    }
                }
            }

            // LIKE pattern: column LIKE 'prefix%'
            // This can be optimized to a prefix scan when the pattern is a prefix match
            Expr::Like {
                expr,
                pattern,
                negated,
            } => {
                // Only handle positive LIKE (not negated)
                if !negated {
                    if let Expr::Column { table, column } = &expr.expr {
                        if let Expr::Literal(Literal::Text(pattern_str)) = &pattern.expr {
                            // Check if this is a prefix pattern (ends with %)
                            if pattern_str.ends_with('%')
                                && !pattern_str[..pattern_str.len() - 1].contains(['%', '_'])
                            {
                                // This is a simple prefix pattern like 'value%'
                                let prefix = &pattern_str[..pattern_str.len() - 1];
                                let col_lower = column.to_lowercase();

                                // For path columns, use PrefixRange (path index)
                                if col_lower == "path" {
                                    return Some(CanonicalPredicate::PrefixRange {
                                        table: table.clone(),
                                        path_col: column.clone(),
                                        prefix: prefix.to_string(),
                                    });
                                }

                                // For indexed property columns, use PropertyPrefixRange
                                // This includes: node_type, and any JSON property
                                if col_lower == "node_type" {
                                    return Some(CanonicalPredicate::PropertyPrefixRange {
                                        table: table.clone(),
                                        column: column.clone(),
                                        prefix: prefix.to_string(),
                                    });
                                }
                            }
                        }
                    }
                }

                // Also handle JSON property LIKE: properties->>'key' LIKE 'prefix%'
                if !negated {
                    if let Expr::JsonExtractText { object, key } = &expr.expr {
                        if let Expr::Column { table, column: _ } = &object.expr {
                            if let Expr::Literal(Literal::Text(key_str)) = &key.expr {
                                if let Expr::Literal(Literal::Text(pattern_str)) = &pattern.expr {
                                    // Check if this is a prefix pattern (ends with %)
                                    if pattern_str.ends_with('%')
                                        && !pattern_str[..pattern_str.len() - 1]
                                            .contains(['%', '_'])
                                    {
                                        let prefix = &pattern_str[..pattern_str.len() - 1];
                                        // Use the JSON key as the column for property prefix scan
                                        return Some(CanonicalPredicate::PropertyPrefixRange {
                                            table: table.clone(),
                                            column: key_str.clone(),
                                            prefix: prefix.to_string(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // column IN (v1, v2, ...) with all-literal list → ColumnIn / JsonPropertyIn.
            // The physical planner expands these into a Union of per-value equality
            // scans when the column is indexed (path, id, node_type, JSON property,
            // compound). NOT IN and non-literal lists are left as `Other`.
            Expr::InList {
                expr: inner,
                list,
                negated: false,
            } if !list.is_empty() => {
                // Case A: plain column, e.g. `path IN (...)`, `node_type IN (...)`.
                if let Expr::Column { table, column } = &inner.expr {
                    let mut values = Vec::with_capacity(list.len());
                    for item in list {
                        if !matches!(&item.expr, Expr::Literal(_)) {
                            return None;
                        }
                        values.push(item.clone());
                    }
                    return Some(CanonicalPredicate::ColumnIn {
                        table: table.clone(),
                        column: column.clone(),
                        values,
                    });
                }

                // Case B: JSON property extract, e.g. `properties->>'k'::String IN (...)`.
                if let Some((table, json_col, key)) = Self::match_json_key_extract(&inner.expr) {
                    let mut values = Vec::with_capacity(list.len());
                    for item in list {
                        if let Expr::Literal(lit) = &item.expr {
                            if let Ok(json_val) = literal_to_json(lit) {
                                values.push(json_val);
                                continue;
                            }
                        }
                        return None;
                    }
                    return Some(CanonicalPredicate::JsonPropertyIn {
                        table,
                        json_col,
                        key,
                        values,
                    });
                }
            }

            // OR disjunctions over the SAME column / JSON key fold into a single
            // ColumnIn / JsonPropertyIn (`a = x OR a = y` ≡ `a IN (x, y)`), which
            // the planner then expands into a Union of indexed equality scans.
            // Heterogeneous ORs (different columns) are left as `Other`: their
            // branches can overlap, so a naive Union would double-count rows.
            Expr::BinaryOp {
                op: BinaryOperator::Or,
                ..
            } => {
                return self.fold_or_to_in(expr);
            }

            _ => {}
        }

        None
    }

    /// Fold an OR tree into a single `ColumnIn` / `JsonPropertyIn` when every
    /// disjunct is an equality or IN over the same column (or JSON key) with
    /// literal values. Returns `None` for heterogeneous or non-literal ORs.
    fn fold_or_to_in(&self, expr: &TypedExpr) -> Option<CanonicalPredicate> {
        let disjuncts = Self::flatten_ors(expr);
        if disjuncts.len() < 2 {
            return None;
        }

        let mut column_target: Option<(String, String)> = None; // (table, column_lowercase)
        let mut column_repr: Option<(String, String)> = None; // first-seen spelling
        let mut column_values: Vec<TypedExpr> = Vec::new();

        let mut json_target: Option<(String, String, String)> = None; // (table, json_col, key)
        let mut json_values: Vec<serde_json::Value> = Vec::new();

        for d in &disjuncts {
            match self.match_canonical_predicate(d)? {
                CanonicalPredicate::ColumnEq {
                    table,
                    column,
                    value,
                } => {
                    // Only literal values can participate in a Union expansion.
                    if !matches!(&value.expr, Expr::Literal(_)) {
                        return None;
                    }
                    let key = (table.clone(), column.to_lowercase());
                    match &column_target {
                        None if json_target.is_none() => {
                            column_target = Some(key);
                            column_repr = Some((table, column));
                        }
                        Some(t) if *t == key => {}
                        _ => return None,
                    }
                    column_values.push(value);
                }
                CanonicalPredicate::ColumnIn {
                    table,
                    column,
                    values,
                } => {
                    let key = (table.clone(), column.to_lowercase());
                    match &column_target {
                        None if json_target.is_none() => {
                            column_target = Some(key);
                            column_repr = Some((table, column));
                        }
                        Some(t) if *t == key => {}
                        _ => return None,
                    }
                    column_values.extend(values);
                }
                CanonicalPredicate::JsonPropertyEq {
                    table,
                    json_col,
                    key,
                    value,
                } => {
                    let target = (table, json_col, key);
                    match &json_target {
                        None if column_target.is_none() => json_target = Some(target),
                        Some(t) if *t == target => {}
                        _ => return None,
                    }
                    json_values.push(value);
                }
                CanonicalPredicate::JsonPropertyIn {
                    table,
                    json_col,
                    key,
                    values,
                } => {
                    let target = (table, json_col, key);
                    match &json_target {
                        None if column_target.is_none() => json_target = Some(target),
                        Some(t) if *t == target => {}
                        _ => return None,
                    }
                    json_values.extend(values);
                }
                _ => return None,
            }
        }

        if let Some((table, column)) = column_repr {
            return Some(CanonicalPredicate::ColumnIn {
                table,
                column,
                values: column_values,
            });
        }
        if let Some((table, json_col, key)) = json_target {
            return Some(CanonicalPredicate::JsonPropertyIn {
                table,
                json_col,
                key,
                values: json_values,
            });
        }
        None
    }

    /// Flatten OR operations (mirror of `flatten_ands`)
    fn flatten_ors(expr: &TypedExpr) -> Vec<TypedExpr> {
        match &expr.expr {
            Expr::BinaryOp {
                left,
                op: BinaryOperator::Or,
                right,
            } => {
                let mut result = Self::flatten_ors(left);
                result.extend(Self::flatten_ors(right));
                result
            }
            _ => vec![expr.clone()],
        }
    }

    /// Recognize a JSON property text-extraction expression and return
    /// `(table, json_col, key)`. Handles both the canonicalized
    /// `CAST(properties::jsonb ->> ['key'] AS text)` shape (from
    /// `properties->>'key'::String`) and the simpler `JsonExtractText` shape.
    fn match_json_key_extract(expr: &Expr) -> Option<(String, String, String)> {
        // Simple shape: properties->>'key'
        if let Expr::JsonExtractText { object, key } = expr {
            if let Expr::Column { table, column } = &object.expr {
                if let Expr::Literal(Literal::Text(key_str)) = &key.expr {
                    return Some((table.clone(), column.clone(), key_str.clone()));
                }
            }
        }

        // Canonicalized cast shape: CAST(CAST(col AS jsonb) ->> ['key'] AS text)
        if let Expr::Cast {
            expr: cast_expr,
            target_type: DataType::Text,
        } = expr
        {
            if let Expr::JsonExtractPath { object, path } = &cast_expr.expr {
                if let Expr::Cast {
                    expr: inner_expr,
                    target_type: DataType::JsonB,
                } = &object.expr
                {
                    if let Expr::Column { table, column } = &inner_expr.expr {
                        if let Expr::Literal(Literal::JsonB(serde_json::Value::Array(elements))) =
                            &path.expr
                        {
                            if elements.len() == 1 {
                                if let serde_json::Value::String(key_str) = &elements[0] {
                                    return Some((table.clone(), column.clone(), key_str.clone()));
                                }
                            }
                        }
                    }
                }
            }
        }

        None
    }

    /// Flatten AND operations
    #[allow(clippy::only_used_in_recursion)]
    pub(super) fn flatten_ands(&self, expr: &TypedExpr) -> Vec<TypedExpr> {
        match &expr.expr {
            Expr::BinaryOp {
                left,
                op: BinaryOperator::And,
                right,
            } => {
                let mut result = self.flatten_ands(left);
                result.extend(self.flatten_ands(right));
                result
            }
            _ => vec![expr.clone()],
        }
    }
}
