//! Hierarchy predicate rewriting logic

use super::{CanonicalPredicate, ComparisonOp};
use crate::analyzer::{BinaryOperator, Expr, Literal, TypedExpr};

pub fn rewrite_hierarchy_predicates(expr: TypedExpr) -> Vec<CanonicalPredicate> {
    match expr.expr.clone() {
        Expr::Function { name, args, .. } if name.to_uppercase() == "CHILD_OF" => {
            rewrite_child_of(&args, expr)
        }
        Expr::Function { name, args, .. } if name.to_uppercase() == "PATH_STARTS_WITH" => {
            rewrite_path_starts_with(&args, expr)
        }
        Expr::Function { name, args, .. } if name.to_uppercase() == "REFERENCES" => {
            rewrite_references(&args, expr)
        }
        Expr::Function { name, args, .. } if name.to_uppercase() == "ST_DWITHIN" => {
            rewrite_st_dwithin(&args, expr)
        }
        Expr::BinaryOp {
            ref left,
            op: BinaryOperator::Eq,
            ref right,
        } => rewrite_equality(left, right, expr),
        Expr::BinaryOp {
            ref left,
            op:
                op @ (BinaryOperator::Gt
                | BinaryOperator::GtEq
                | BinaryOperator::Lt
                | BinaryOperator::LtEq),
            ref right,
        } => rewrite_comparison(left, &op, right, expr),
        Expr::JsonContains {
            ref object,
            ref pattern,
        } => rewrite_json_contains(object, pattern, expr),
        Expr::JsonKeyExists { .. } => vec![CanonicalPredicate::Other(expr)],
        Expr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } => {
            let mut p = rewrite_hierarchy_predicates(left.as_ref().clone());
            p.extend(rewrite_hierarchy_predicates(right.as_ref().clone()));
            p
        }
        _ => vec![CanonicalPredicate::Other(expr)],
    }
}

fn rewrite_child_of(args: &[TypedExpr], expr: TypedExpr) -> Vec<CanonicalPredicate> {
    if args.len() == 1 {
        if let Expr::Literal(lit) = &args[0].expr {
            let parent_path = match lit {
                Literal::Path(p) => p.clone(),
                Literal::Text(t) => t.clone(),
                _ => return vec![CanonicalPredicate::Other(expr)],
            };
            return vec![CanonicalPredicate::ChildOf { parent_path }];
        }
    }
    vec![CanonicalPredicate::Other(expr)]
}

fn rewrite_path_starts_with(args: &[TypedExpr], expr: TypedExpr) -> Vec<CanonicalPredicate> {
    if args.len() == 2 {
        if let (Expr::Column { table, column }, Expr::Literal(lit)) = (&args[0].expr, &args[1].expr)
        {
            let prefix = match lit {
                Literal::Path(p) => p.clone(),
                Literal::Text(t) => t.clone(),
                _ => return vec![CanonicalPredicate::Other(expr)],
            };
            return vec![CanonicalPredicate::PrefixRange {
                table: table.clone(),
                path_col: column.clone(),
                prefix,
            }];
        }
    }
    vec![CanonicalPredicate::Other(expr)]
}

fn rewrite_references(args: &[TypedExpr], expr: TypedExpr) -> Vec<CanonicalPredicate> {
    if args.len() == 1 {
        if let Expr::Literal(Literal::Text(target)) = &args[0].expr {
            if let Some((ws, path)) = target.split_once(':') {
                return vec![CanonicalPredicate::References {
                    target_workspace: ws.to_string(),
                    target_path: path.to_string(),
                }];
            }
        }
    }
    vec![CanonicalPredicate::Other(expr)]
}

/// Extract geometry source (table, column, property_name) from an expression.
///
/// Handles direct property access and CAST(... AS GEOMETRY) wrappers:
/// - `properties->>'location'` (JsonExtractText)
/// - `properties->'location'` (JsonExtract)
/// - `CAST(properties->>'location' AS GEOMETRY)`
/// - Direct column reference
/// Extract geometry source (table, column, property_name) from an expression.
///
/// Handles direct property access and CAST(... AS GEOMETRY) wrappers:
/// - `properties->>'location'` (JsonExtractText)
/// - `properties->'location'` (JsonExtract)
/// - `CAST(properties->>'location' AS GEOMETRY)`
/// - Direct column reference
///
/// Returns `(table, geometry_column, property_name)` if matched.
pub fn extract_geometry_source(expr: &Expr) -> Option<(String, String, String)> {
    match expr {
        // Unwrap CAST(... AS GEOMETRY) only — reject other target types
        Expr::Cast {
            expr: inner,
            target_type: crate::analyzer::DataType::Geometry,
        } => extract_geometry_source(&inner.expr),

        Expr::JsonExtractText { object, key } => {
            if let Expr::Column { table, column } = &object.expr {
                let prop = match &key.expr {
                    Expr::Literal(Literal::Text(t)) => t.clone(),
                    _ => return None,
                };
                Some((table.clone(), column.clone(), prop))
            } else {
                None
            }
        }
        Expr::JsonExtract { object, key } => {
            if let Expr::Column { table, column } = &object.expr {
                let prop = match &key.expr {
                    Expr::Literal(Literal::Text(t)) => t.clone(),
                    _ => return None,
                };
                Some((table.clone(), column.clone(), prop))
            } else {
                None
            }
        }
        Expr::Column { table, column } => {
            Some((table.clone(), "properties".to_string(), column.clone()))
        }
        _ => None,
    }
}

fn rewrite_st_dwithin(args: &[TypedExpr], expr: TypedExpr) -> Vec<CanonicalPredicate> {
    if args.len() != 3 {
        return vec![CanonicalPredicate::Other(expr)];
    }
    let (table, geometry_column, property_name) = match extract_geometry_source(&args[0].expr) {
        Some(v) => v,
        None => return vec![CanonicalPredicate::Other(expr)],
    };
    let (center_lon, center_lat) = match &args[1].expr {
        Expr::Function { name, args: pa, .. }
            if name.to_uppercase() == "ST_POINT" || name.to_uppercase() == "ST_MAKEPOINT" =>
        {
            if pa.len() == 2 {
                let lon = match &pa[0].expr {
                    Expr::Literal(Literal::Double(f)) => *f,
                    Expr::Literal(Literal::Int(i)) => *i as f64,
                    Expr::Literal(Literal::BigInt(i)) => *i as f64,
                    _ => return vec![CanonicalPredicate::Other(expr)],
                };
                let lat = match &pa[1].expr {
                    Expr::Literal(Literal::Double(f)) => *f,
                    Expr::Literal(Literal::Int(i)) => *i as f64,
                    Expr::Literal(Literal::BigInt(i)) => *i as f64,
                    _ => return vec![CanonicalPredicate::Other(expr)],
                };
                (lon, lat)
            } else {
                return vec![CanonicalPredicate::Other(expr)];
            }
        }
        _ => return vec![CanonicalPredicate::Other(expr)],
    };
    let radius_meters = match &args[2].expr {
        Expr::Literal(Literal::Double(f)) => *f,
        Expr::Literal(Literal::Int(i)) => *i as f64,
        Expr::Literal(Literal::BigInt(i)) => *i as f64,
        _ => return vec![CanonicalPredicate::Other(expr)],
    };
    vec![CanonicalPredicate::SpatialDWithin {
        table,
        geometry_column,
        property_name,
        center_lon,
        center_lat,
        radius_meters,
    }]
}

fn rewrite_equality(
    left: &TypedExpr,
    right: &TypedExpr,
    expr: TypedExpr,
) -> Vec<CanonicalPredicate> {
    if let Expr::Function { name, args, .. } = &left.expr {
        if name.to_uppercase() == "DEPTH" && args.len() == 1 {
            if let Expr::Column { table, column } = &args[0].expr {
                if let Expr::Literal(Literal::Int(k)) = &right.expr {
                    return vec![CanonicalPredicate::DepthEq {
                        table: table.clone(),
                        path_col: column.clone(),
                        depth_value: *k,
                    }];
                }
            }
        }
        if name.to_uppercase() == "PARENT" && args.len() == 1 {
            if let Expr::Column { table, column } = &args[0].expr {
                if let Expr::Literal(lit) = &right.expr {
                    let pp = match lit {
                        Literal::Path(p) => p.clone(),
                        Literal::Text(t) => t.clone(),
                        _ => return vec![CanonicalPredicate::Other(expr)],
                    };
                    let prefix = if pp == "/" {
                        "/".into()
                    } else {
                        format!("{}/", pp.trim_end_matches('/'))
                    };
                    let pd = pp.split('/').filter(|s| !s.is_empty()).count();
                    return vec![
                        CanonicalPredicate::PrefixRange {
                            table: table.clone(),
                            path_col: column.clone(),
                            prefix,
                        },
                        CanonicalPredicate::DepthEq {
                            table: table.clone(),
                            path_col: column.clone(),
                            depth_value: (pd + 1) as i32,
                        },
                    ];
                }
            }
        }
    }
    if let Expr::Column { table, column } = &left.expr {
        return vec![CanonicalPredicate::ColumnEq {
            table: table.clone(),
            column: column.clone(),
            value: right.clone(),
        }];
    }
    vec![CanonicalPredicate::Other(expr)]
}

fn rewrite_comparison(
    left: &TypedExpr,
    op: &BinaryOperator,
    right: &TypedExpr,
    expr: TypedExpr,
) -> Vec<CanonicalPredicate> {
    if let Expr::Column { table, column } = &left.expr {
        if let Some(cop) = ComparisonOp::from_binary_op(op) {
            return vec![CanonicalPredicate::RangeCompare {
                table: table.clone(),
                column: column.clone(),
                op: cop,
                value: right.clone(),
            }];
        }
    }
    if let Expr::Column { table, column } = &right.expr {
        if let Some(cop) = ComparisonOp::from_binary_op(op) {
            return vec![CanonicalPredicate::RangeCompare {
                table: table.clone(),
                column: column.clone(),
                op: cop.reverse(),
                value: left.clone(),
            }];
        }
    }
    vec![CanonicalPredicate::Other(expr)]
}

fn rewrite_json_contains(
    object: &TypedExpr,
    pattern: &TypedExpr,
    expr: TypedExpr,
) -> Vec<CanonicalPredicate> {
    if let Expr::Column { table, column } = &object.expr {
        if let Expr::Literal(Literal::JsonB(jv)) = &pattern.expr {
            if let Some(obj) = jv.as_object() {
                if obj.len() == 1 {
                    let (k, v) = obj.iter().next().unwrap();
                    return vec![CanonicalPredicate::JsonPropertyEq {
                        table: table.clone(),
                        json_col: column.clone(),
                        key: k.clone(),
                        value: v.clone(),
                    }];
                }
            }
        }
    }
    vec![CanonicalPredicate::Other(expr)]
}
