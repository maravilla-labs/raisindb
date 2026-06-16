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
    RangeCompare {
        table: String,
        column: String,
        op: ComparisonOp,
        value: TypedExpr,
    },
    PropertyPrefixRange {
        table: String,
        column: String,
        prefix: String,
    },
    SpatialDWithin {
        table: String,
        geometry_column: String,
        property_name: String,
        center_lon: f64,
        center_lat: f64,
        radius_meters: f64,
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
            CanonicalPredicate::SpatialDWithin {
                table,
                geometry_column,
                property_name,
                center_lon,
                center_lat,
                radius_meters,
            } => {
                let col =
                    TypedExpr::column(table.clone(), geometry_column.clone(), DataType::JsonB);
                let key = TypedExpr::literal(Literal::Text(property_name.clone()));
                let json_extract = TypedExpr::new(
                    Expr::JsonExtractText {
                        object: Box::new(col),
                        key: Box::new(key),
                    },
                    DataType::Text,
                );
                // Wrap in CAST(... AS GEOMETRY) so runtime evaluation converts Text→Geometry
                let je = TypedExpr::new(
                    Expr::Cast {
                        expr: Box::new(json_extract),
                        target_type: DataType::Geometry,
                    },
                    DataType::Geometry,
                );
                let lon = TypedExpr::literal(Literal::Double(*center_lon));
                let lat = TypedExpr::literal(Literal::Double(*center_lat));
                let pt = TypedExpr::new(
                    Expr::Function {
                        name: "ST_POINT".into(),
                        args: vec![lon, lat],
                        signature: FunctionSignature {
                            name: "ST_POINT".into(),
                            params: vec![DataType::Double, DataType::Double],
                            return_type: DataType::Geometry,
                            is_deterministic: true,
                            category: FunctionCategory::Geospatial,
                        },
                        filter: None,
                    },
                    DataType::Geometry,
                );
                let rad = TypedExpr::literal(Literal::Double(*radius_meters));
                TypedExpr::new(
                    Expr::Function {
                        name: "ST_DWITHIN".into(),
                        args: vec![je, pt, rad],
                        signature: FunctionSignature {
                            name: "ST_DWITHIN".into(),
                            params: vec![DataType::Geometry, DataType::Geometry, DataType::Double],
                            return_type: DataType::Boolean,
                            is_deterministic: true,
                            category: FunctionCategory::Geospatial,
                        },
                        filter: None,
                    },
                    DataType::Boolean,
                )
            }
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
