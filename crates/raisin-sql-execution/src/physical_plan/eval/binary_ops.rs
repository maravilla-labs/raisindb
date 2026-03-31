//! Binary and unary operation evaluation

use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{BinaryOperator, Literal, TypedExpr, UnaryOperator};

use super::helpers::{
    arithmetic_op, compare_literals, is_zero, literals_equal, logical_and, logical_or,
};
use super::vector_ops::{dot_product, extract_vector, l2_distance};

/// Evaluate a binary operation
pub(super) fn eval_binary_op(
    left: &TypedExpr,
    op: &BinaryOperator,
    right: &TypedExpr,
    row: &Row,
) -> Result<Literal, Error> {
    // Import eval_expr from parent module
    use super::core::eval_expr;

    let left_val = eval_expr(left, row)?;
    let right_val = eval_expr(right, row)?;

    match op {
        // Arithmetic operators
        BinaryOperator::Add => {
            // Special handling for timestamp arithmetic
            match (&left_val, &right_val) {
                // TIMESTAMPTZ + INTERVAL → TIMESTAMPTZ
                (Literal::Timestamp(ts), Literal::Interval(duration))
                | (Literal::Interval(duration), Literal::Timestamp(ts)) => {
                    Ok(Literal::Timestamp(*ts + *duration))
                }
                // Otherwise use numeric arithmetic
                _ => arithmetic_op(&left_val, &right_val, |a, b| a + b),
            }
        }
        BinaryOperator::Subtract => {
            // Special handling for timestamp arithmetic
            match (&left_val, &right_val) {
                // TIMESTAMPTZ - INTERVAL → TIMESTAMPTZ
                (Literal::Timestamp(ts), Literal::Interval(duration)) => {
                    Ok(Literal::Timestamp(*ts - *duration))
                }
                // TIMESTAMPTZ - TIMESTAMPTZ → INTERVAL
                (Literal::Timestamp(ts1), Literal::Timestamp(ts2)) => {
                    Ok(Literal::Interval(*ts1 - *ts2))
                }
                // Otherwise use numeric arithmetic
                _ => arithmetic_op(&left_val, &right_val, |a, b| a - b),
            }
        }
        BinaryOperator::Multiply => arithmetic_op(&left_val, &right_val, |a, b| a * b),
        BinaryOperator::Divide => {
            if is_zero(&right_val) {
                Err(Error::Validation("Division by zero".to_string()))
            } else {
                arithmetic_op(&left_val, &right_val, |a, b| a / b)
            }
        }
        BinaryOperator::Modulo => arithmetic_op(&left_val, &right_val, |a, b| a % b),

        // Comparison operators
        BinaryOperator::Eq => Ok(Literal::Boolean(literals_equal(&left_val, &right_val)?)),
        BinaryOperator::NotEq => Ok(Literal::Boolean(!literals_equal(&left_val, &right_val)?)),
        BinaryOperator::Lt | BinaryOperator::LtEq | BinaryOperator::Gt | BinaryOperator::GtEq => {
            Ok(Literal::Boolean(compare_literals(
                &left_val, &right_val, *op,
            )?))
        }

        // Logical operators
        BinaryOperator::And => logical_and(&left_val, &right_val),
        BinaryOperator::Or => logical_or(&left_val, &right_val),

        // JSON operators

        // JSON concatenation: JSONB || JSONB
        // Merges right object into left object (PostgreSQL semantics)
        BinaryOperator::JsonConcat => {
            match (&left_val, &right_val) {
                (Literal::JsonB(left_obj), Literal::JsonB(right_obj)) => {
                    // Merge right into left (shallow merge, right values override left)
                    let mut merged = left_obj.clone();

                    // If both are objects, merge keys
                    if let (
                        serde_json::Value::Object(left_map),
                        serde_json::Value::Object(right_map),
                    ) = (&mut merged, right_obj)
                    {
                        for (key, value) in right_map.iter() {
                            left_map.insert(key.clone(), value.clone());
                        }
                        Ok(Literal::JsonB(merged))
                    } else {
                        // If either is not an object, right overwrites left (PostgreSQL behavior)
                        Ok(Literal::JsonB(right_obj.clone()))
                    }
                }
                (Literal::Null, Literal::JsonB(obj)) | (Literal::JsonB(obj), Literal::Null) => {
                    // NULL || JSONB or JSONB || NULL returns the non-NULL value
                    Ok(Literal::JsonB(obj.clone()))
                }
                (Literal::Null, Literal::Null) => Ok(Literal::Null),
                _ => Err(Error::Validation(
                    "JSON concatenation (||) requires JSONB operands".to_string(),
                )),
            }
        }

        // String concatenation: Text || Text → Text
        BinaryOperator::StringConcat => {
            match (&left_val, &right_val) {
                // NULL handling: NULL || anything = NULL (PostgreSQL semantics)
                (Literal::Null, _) | (_, Literal::Null) => Ok(Literal::Null),
                // String concatenation variants
                (Literal::Text(l), Literal::Text(r)) => Ok(Literal::Text(format!("{}{}", l, r))),
                (Literal::Text(l), Literal::Path(r)) => Ok(Literal::Text(format!("{}{}", l, r))),
                (Literal::Path(l), Literal::Text(r)) => Ok(Literal::Text(format!("{}{}", l, r))),
                (Literal::Path(l), Literal::Path(r)) => Ok(Literal::Text(format!("{}{}", l, r))),
                // Coerce other types to string (PostgreSQL allows this)
                (l, r) => Ok(Literal::Text(format!(
                    "{}{}",
                    literal_to_string(l),
                    literal_to_string(r)
                ))),
            }
        }

        // Other JSON operators (handled in Expr match above)
        BinaryOperator::JsonExtract | BinaryOperator::JsonContains => Err(Error::Validation(
            "JSON operators should be handled at expression level".to_string(),
        )),

        // Full-text search operator (should be handled at query planning level)
        BinaryOperator::TextSearchMatch => Err(Error::Validation(
            "Text search operator @@ should be handled at query planning level".to_string(),
        )),

        // Vector distance operators
        // Handle NULL values: if either vector is NULL, distance is NULL
        BinaryOperator::VectorL2Distance => {
            // L2 (Euclidean) distance: sqrt(sum((a[i] - b[i])^2))
            if matches!(left_val, Literal::Null) || matches!(right_val, Literal::Null) {
                return Ok(Literal::Null);
            }
            let v1 = extract_vector(&left_val)?;
            let v2 = extract_vector(&right_val)?;
            let distance = l2_distance(&v1, &v2);
            Ok(Literal::Double(distance as f64))
        }
        BinaryOperator::VectorCosineDistance => {
            // Cosine distance: 1 - dot_product(a, b)
            // Assumes vectors are already normalized (as they are at storage time)
            if matches!(left_val, Literal::Null) || matches!(right_val, Literal::Null) {
                return Ok(Literal::Null);
            }
            let v1 = extract_vector(&left_val)?;
            let v2 = extract_vector(&right_val)?;
            let distance = 1.0 - dot_product(&v1, &v2);
            Ok(Literal::Double(distance as f64))
        }
        BinaryOperator::VectorInnerProduct => {
            // Inner product (negative dot product for pgvector compatibility)
            if matches!(left_val, Literal::Null) || matches!(right_val, Literal::Null) {
                return Ok(Literal::Null);
            }
            let v1 = extract_vector(&left_val)?;
            let v2 = extract_vector(&right_val)?;
            let product = -dot_product(&v1, &v2);
            Ok(Literal::Double(product as f64))
        }
    }
}

/// Evaluate a unary operation
pub(super) fn eval_unary_op(
    op: &UnaryOperator,
    expr: &TypedExpr,
    row: &Row,
) -> Result<Literal, Error> {
    // Import eval_expr from parent module
    use super::core::eval_expr;

    let value = eval_expr(expr, row)?;

    match op {
        UnaryOperator::Not => match value {
            Literal::Boolean(b) => Ok(Literal::Boolean(!b)),
            _ => Err(Error::Validation(
                "NOT operator requires boolean operand".to_string(),
            )),
        },
        UnaryOperator::Negate => match value {
            Literal::Int(i) => Ok(Literal::Int(-i)),
            Literal::BigInt(i) => Ok(Literal::BigInt(-i)),
            Literal::Double(f) => Ok(Literal::Double(-f)),
            _ => Err(Error::Validation(
                "Negation requires numeric operand".to_string(),
            )),
        },
    }
}

/// Convert a Literal to its string representation for string concatenation
fn literal_to_string(lit: &Literal) -> String {
    match lit {
        Literal::Null => String::new(), // Should not reach here due to NULL handling above
        Literal::Boolean(b) => b.to_string(),
        Literal::Int(i) => i.to_string(),
        Literal::BigInt(i) => i.to_string(),
        Literal::Double(f) => f.to_string(),
        Literal::Text(s) => s.clone(),
        Literal::Uuid(s) => s.clone(),
        Literal::Path(s) => s.clone(),
        Literal::JsonB(v) => v.to_string(),
        Literal::Vector(v) => format!("{:?}", v),
        Literal::Geometry(v) => v.to_string(),
        Literal::Timestamp(ts) => ts.to_rfc3339(),
        Literal::Interval(d) => format!("{}s", d.num_seconds()),
        Literal::Parameter(p) => p.clone(),
    }
}
