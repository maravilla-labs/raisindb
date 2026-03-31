//! Cast operation constant folding

use crate::analyzer::{DataType, Literal, TypedExpr};

/// Fold a cast operation with literal operand
pub(super) fn fold_cast(literal: &Literal, target_type: &DataType) -> Option<TypedExpr> {
    match (literal, target_type.base_type()) {
        // Text to Int
        (Literal::Text(s), DataType::Int) => {
            if let Ok(n) = s.parse::<i32>() {
                Some(TypedExpr::literal(Literal::Int(n)))
            } else {
                None
            }
        }

        // Int to Text
        (Literal::Int(n), DataType::Text) => Some(TypedExpr::literal(Literal::Text(n.to_string()))),

        // Int to BigInt
        (Literal::Int(n), DataType::BigInt) => Some(TypedExpr::literal(Literal::BigInt(*n as i64))),

        // Int to Double
        (Literal::Int(n), DataType::Double) => Some(TypedExpr::literal(Literal::Double(*n as f64))),

        // Identity casts
        (Literal::Int(n), DataType::Int) => Some(TypedExpr::literal(Literal::Int(*n))),
        (Literal::Text(s), DataType::Text) => Some(TypedExpr::literal(Literal::Text(s.clone()))),

        _ => None,
    }
}
