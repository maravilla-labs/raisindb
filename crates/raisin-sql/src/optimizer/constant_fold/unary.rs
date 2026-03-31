//! Unary operation constant folding

use crate::analyzer::{Literal, TypedExpr, UnaryOperator};

/// Fold a unary operation with literal operand
pub(super) fn fold_unary_op(op: UnaryOperator, operand: &Literal) -> Option<TypedExpr> {
    match (op, operand) {
        (UnaryOperator::Not, Literal::Boolean(b)) => Some(TypedExpr::literal(Literal::Boolean(!b))),
        (UnaryOperator::Negate, Literal::Int(n)) => Some(TypedExpr::literal(Literal::Int(-n))),
        (UnaryOperator::Negate, Literal::BigInt(n)) => {
            Some(TypedExpr::literal(Literal::BigInt(-n)))
        }
        (UnaryOperator::Negate, Literal::Double(n)) => {
            Some(TypedExpr::literal(Literal::Double(-n)))
        }
        _ => None,
    }
}
