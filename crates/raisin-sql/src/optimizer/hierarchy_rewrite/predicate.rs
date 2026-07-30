//! Canonical predicate types for hierarchy queries

use super::ComparisonOp;
use crate::analyzer::{BinaryOperator, DataType, Expr, Literal, TypedExpr};

#[derive(Debug, Clone)]
pub enum CanonicalPredicate {
    PrefixRange {
        table: String,
        path_col: String,
        prefix: String,
    },
    DepthEq {
        table: String,
        path_col: String,
        depth_value: i32,
    },
    ChildOf {
        parent_path: String,
    },
    DescendantOf {
        parent_path: String,
        max_depth: Option<i64>,
    },
    ColumnEq {
        table: String,
        column: String,
        value: TypedExpr,
    },
    JsonPropertyEq {
        table: String,
        json_col: String,
        key: String,
        value: serde_json::Value,
    },
    /// `column IN (v1, v2, ...)` over a plain column (path, id, node_type, ...).
    /// Expanded by the physical planner into a `Union` of per-value equality
    /// scans when the column is backed by an index; otherwise reconstructed as a
    /// row-level `IN` filter via `to_expr`.
    ColumnIn {
        table: String,
        column: String,
        values: Vec<TypedExpr>,
    },
    /// `properties->>'key' IN (v1, v2, ...)` over a JSON property.
    JsonPropertyIn {
        table: String,
        json_col: String,
        key: String,
        values: Vec<serde_json::Value>,
    },
    RangeCompare {
        table: String,
        column: String,
        op: ComparisonOp,
        value: TypedExpr,
    },
    /// `properties->>'key' <op> 'text'` — lexicographic range over a JSON
    /// property. Only text values are canonicalized (the property index stores
    /// raw strings and `->>` yields text, so lexicographic order is consistent
    /// between the index scan and row-level evaluation).
    JsonPropertyRange {
        table: String,
        json_col: String,
        key: String,
        op: ComparisonOp,
        value: String,
    },
    PropertyPrefixRange {
        table: String,
        column: String,
        prefix: String,
    },
    /// A proximity predicate the spatial index can drive.
    ///
    /// Produced only by `super::spatial::extract_spatial_predicate` — the single
    /// extractor both the optimizer and the execution planner call.
    SpatialDWithin {
        table: String,
        geometry_column: String,
        property_name: String,
        center_lon: f64,
        center_lat: f64,
        radius_meters: f64,
        /// The planner's licence to drop this predicate from the residual
        /// filter.
        ///
        /// `true` only when a *complete* index scan over `(center_lon,
        /// center_lat, radius_meters)` returns EXACTLY this predicate's match
        /// set. `false` whenever the scan parameters were widened — a non-point
        /// query geometry reduced to its envelope centre with an inflated
        /// radius, or a strict `ST_DISTANCE < r` whose boundary ring the scan's
        /// `<= r` post-filter includes. An inexact predicate makes the scan a
        /// *candidate source only*, and [`Self::to_expr`] must be re-applied per
        /// row.
        ///
        /// Note this is necessary but NOT sufficient to strip: the index also
        /// has to be built and the cell plan has to be a proven cover. See
        /// `build_spatial_scan`.
        exact: bool,
        /// The verbatim source expression.
        ///
        /// Kept rather than reconstructed because the reconstruction is only
        /// equivalent for the canonical `ST_DWITHIN(geom, ST_POINT(..), r)`
        /// spelling; for every widened form it would be the *widened* predicate,
        /// which as a residual filter would fail to reject the extra rows the
        /// widening let in.
        original: Box<TypedExpr>,
    },
    References {
        target_workspace: String,
        target_path: String,
    },
    Other(TypedExpr),
}

impl CanonicalPredicate {
    pub fn to_expr(&self) -> TypedExpr {
        use crate::analyzer::functions::{FunctionCategory, FunctionSignature};
        match self {
            CanonicalPredicate::PrefixRange {
                table,
                path_col,
                prefix,
            } => {
                let col = TypedExpr::column(table.clone(), path_col.clone(), DataType::Path);
                let pfx = TypedExpr::literal(Literal::Path(prefix.clone()));
                TypedExpr::new(
                    Expr::Function {
                        name: "PATH_STARTS_WITH".into(),
                        args: vec![col, pfx],
                        signature: FunctionSignature {
                            name: "PATH_STARTS_WITH".into(),
                            params: vec![DataType::Path, DataType::Path],
                            return_type: DataType::Boolean,
                            is_deterministic: true,
                            category: FunctionCategory::Hierarchy,
                        },
                        filter: None,
                    },
                    DataType::Boolean,
                )
            }
            CanonicalPredicate::DepthEq {
                table,
                path_col,
                depth_value,
            } => {
                let col = TypedExpr::column(table.clone(), path_col.clone(), DataType::Path);
                let df = TypedExpr::new(
                    Expr::Function {
                        name: "DEPTH".into(),
                        args: vec![col],
                        signature: FunctionSignature {
                            name: "DEPTH".into(),
                            params: vec![DataType::Path],
                            return_type: DataType::Int,
                            is_deterministic: true,
                            category: FunctionCategory::Hierarchy,
                        },
                        filter: None,
                    },
                    DataType::Int,
                );
                TypedExpr::new(
                    Expr::BinaryOp {
                        left: Box::new(df),
                        op: BinaryOperator::Eq,
                        right: Box::new(TypedExpr::literal(Literal::Int(*depth_value))),
                    },
                    DataType::Boolean,
                )
            }
            CanonicalPredicate::ColumnEq {
                table,
                column,
                value,
            } => {
                let col = TypedExpr::column(table.clone(), column.clone(), value.data_type.clone());
                TypedExpr::new(
                    Expr::BinaryOp {
                        left: Box::new(col),
                        op: BinaryOperator::Eq,
                        right: Box::new(value.clone()),
                    },
                    DataType::Boolean,
                )
            }
            CanonicalPredicate::ChildOf { parent_path } => {
                let p = TypedExpr::literal(Literal::Path(parent_path.clone()));
                TypedExpr::new(
                    Expr::Function {
                        name: "CHILD_OF".into(),
                        args: vec![p],
                        signature: FunctionSignature {
                            name: "CHILD_OF".into(),
                            params: vec![DataType::Path],
                            return_type: DataType::Boolean,
                            is_deterministic: true,
                            category: FunctionCategory::Hierarchy,
                        },
                        filter: None,
                    },
                    DataType::Boolean,
                )
            }
            CanonicalPredicate::DescendantOf {
                parent_path,
                max_depth,
            } => {
                let p = TypedExpr::literal(Literal::Path(parent_path.clone()));
                let (args, params) = if let Some(d) = max_depth {
                    (
                        vec![p, TypedExpr::literal(Literal::BigInt(*d))],
                        vec![DataType::Path, DataType::BigInt],
                    )
                } else {
                    (vec![p], vec![DataType::Path])
                };
                TypedExpr::new(
                    Expr::Function {
                        name: "DESCENDANT_OF".into(),
                        args,
                        signature: FunctionSignature {
                            name: "DESCENDANT_OF".into(),
                            params,
                            return_type: DataType::Boolean,
                            is_deterministic: true,
                            category: FunctionCategory::Hierarchy,
                        },
                        filter: None,
                    },
                    DataType::Boolean,
                )
            }
            CanonicalPredicate::JsonPropertyEq {
                table,
                json_col,
                key,
                value,
            } => {
                // Reconstruct the original `properties->>'key' = value` predicate.
                //
                // NOTE: this must round-trip back to text extraction, NOT
                // `properties @> value`. The `@>` form drops the key entirely and,
                // for a scalar pattern against an object, `json_contains` is always
                // false (object == scalar) — so any JsonPropertyEq that survives as
                // a row-level filter (e.g. when a more selective predicate such as
                // `path =` wins the scan) silently matched zero rows. `->>` yields
                // text, so we compare against the value rendered as plain text
                // (mirroring PropertyIndexScan's value encoding) rather than a
                // JSON-encoded literal.
                let col = TypedExpr::column(table.clone(), json_col.clone(), DataType::JsonB);
                let key_expr = TypedExpr::literal(Literal::Text(key.clone()));
                let extract = TypedExpr::new(
                    Expr::JsonExtractText {
                        object: Box::new(col),
                        key: Box::new(key_expr),
                    },
                    DataType::Nullable(Box::new(DataType::Text)),
                );
                let value_text = match value {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                let rhs = TypedExpr::literal(Literal::Text(value_text));
                TypedExpr::new(
                    Expr::BinaryOp {
                        left: Box::new(extract),
                        op: BinaryOperator::Eq,
                        right: Box::new(rhs),
                    },
                    DataType::Boolean,
                )
            }
            CanonicalPredicate::ColumnIn {
                table,
                column,
                values,
            } => {
                let col = TypedExpr::column(
                    table.clone(),
                    column.clone(),
                    values
                        .first()
                        .map(|v| v.data_type.clone())
                        .unwrap_or(DataType::Text),
                );
                TypedExpr::new(
                    Expr::InList {
                        expr: Box::new(col),
                        list: values.clone(),
                        negated: false,
                    },
                    DataType::Boolean,
                )
            }
            CanonicalPredicate::JsonPropertyIn {
                table,
                json_col,
                key,
                values,
            } => {
                // Reconstruct `properties->>'key' IN (...)` as a text-extraction IN
                // filter (same reasoning as JsonPropertyEq: `->>` yields text, so
                // compare against the values rendered as plain text).
                let col = TypedExpr::column(table.clone(), json_col.clone(), DataType::JsonB);
                let key_expr = TypedExpr::literal(Literal::Text(key.clone()));
                let extract = TypedExpr::new(
                    Expr::JsonExtractText {
                        object: Box::new(col),
                        key: Box::new(key_expr),
                    },
                    DataType::Nullable(Box::new(DataType::Text)),
                );
                let list: Vec<TypedExpr> = values
                    .iter()
                    .map(|value| {
                        let value_text = match value {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        TypedExpr::literal(Literal::Text(value_text))
                    })
                    .collect();
                TypedExpr::new(
                    Expr::InList {
                        expr: Box::new(extract),
                        list,
                        negated: false,
                    },
                    DataType::Boolean,
                )
            }
            CanonicalPredicate::RangeCompare {
                table,
                column,
                op,
                value,
            } => {
                let col = TypedExpr::column(table.clone(), column.clone(), value.data_type.clone());
                TypedExpr::new(
                    Expr::BinaryOp {
                        left: Box::new(col),
                        op: op.to_binary_op(),
                        right: Box::new(value.clone()),
                    },
                    DataType::Boolean,
                )
            }
            CanonicalPredicate::JsonPropertyRange {
                table,
                json_col,
                key,
                op,
                value,
            } => {
                // Reconstruct `properties->>'key' <op> 'value'` (text comparison,
                // same shape as JsonPropertyEq::to_expr).
                let col = TypedExpr::column(table.clone(), json_col.clone(), DataType::JsonB);
                let key_expr = TypedExpr::literal(Literal::Text(key.clone()));
                let extract = TypedExpr::new(
                    Expr::JsonExtractText {
                        object: Box::new(col),
                        key: Box::new(key_expr),
                    },
                    DataType::Nullable(Box::new(DataType::Text)),
                );
                let rhs = TypedExpr::literal(Literal::Text(value.clone()));
                TypedExpr::new(
                    Expr::BinaryOp {
                        left: Box::new(extract),
                        op: op.to_binary_op(),
                        right: Box::new(rhs),
                    },
                    DataType::Boolean,
                )
            }
            CanonicalPredicate::PropertyPrefixRange {
                table,
                column,
                prefix,
            } => {
                let col = TypedExpr::column(table.clone(), column.clone(), DataType::Text);
                let pat = TypedExpr::literal(Literal::Text(format!("{}%", prefix)));
                TypedExpr::new(
                    Expr::Like {
                        expr: Box::new(col),
                        pattern: Box::new(pat),
                        negated: false,
                    },
                    DataType::Boolean,
                )
            }
            // Return the verbatim source expression. Reconstructing a canonical
            // `ST_DWITHIN(CAST(properties->>'k' AS GEOMETRY), ST_POINT(..), r)`
            // here would silently substitute the WIDENED window for what the user
            // wrote, and a residual filter built from the widened window cannot
            // reject the rows the widening admitted.
            CanonicalPredicate::SpatialDWithin { original, .. } => (**original).clone(),
            CanonicalPredicate::References {
                target_workspace,
                target_path,
            } => {
                let t = TypedExpr::literal(Literal::Text(format!(
                    "{}:{}",
                    target_workspace, target_path
                )));
                TypedExpr::new(
                    Expr::Function {
                        name: "REFERENCES".into(),
                        args: vec![t],
                        signature: FunctionSignature {
                            name: "REFERENCES".into(),
                            params: vec![DataType::Text],
                            return_type: DataType::Boolean,
                            is_deterministic: true,
                            category: FunctionCategory::Hierarchy,
                        },
                        filter: None,
                    },
                    DataType::Boolean,
                )
            }
            CanonicalPredicate::Other(expr) => expr.clone(),
        }
    }
}
