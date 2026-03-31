//! Binary operation constant folding

use crate::analyzer::{BinaryOperator, Literal, TypedExpr};

/// Fold a binary operation with literal operands
pub(super) fn fold_binary_op(
    left: &Literal,
    op: BinaryOperator,
    right: &Literal,
) -> Option<TypedExpr> {
    match (left, op, right) {
        // Integer arithmetic
        (Literal::Int(a), BinaryOperator::Add, Literal::Int(b)) => {
            Some(TypedExpr::literal(Literal::Int(a + b)))
        }
        (Literal::Int(a), BinaryOperator::Subtract, Literal::Int(b)) => {
            Some(TypedExpr::literal(Literal::Int(a - b)))
        }
        (Literal::Int(a), BinaryOperator::Multiply, Literal::Int(b)) => {
            Some(TypedExpr::literal(Literal::Int(a * b)))
        }
        (Literal::Int(a), BinaryOperator::Divide, Literal::Int(b)) if *b != 0 => {
            Some(TypedExpr::literal(Literal::Int(a / b)))
        }
        (Literal::Int(a), BinaryOperator::Modulo, Literal::Int(b)) if *b != 0 => {
            Some(TypedExpr::literal(Literal::Int(a % b)))
        }

        // Integer comparisons
        (Literal::Int(a), BinaryOperator::Eq, Literal::Int(b)) => {
            Some(TypedExpr::literal(Literal::Boolean(a == b)))
        }
        (Literal::Int(a), BinaryOperator::NotEq, Literal::Int(b)) => {
            Some(TypedExpr::literal(Literal::Boolean(a != b)))
        }
        (Literal::Int(a), BinaryOperator::Lt, Literal::Int(b)) => {
            Some(TypedExpr::literal(Literal::Boolean(a < b)))
        }
        (Literal::Int(a), BinaryOperator::LtEq, Literal::Int(b)) => {
            Some(TypedExpr::literal(Literal::Boolean(a <= b)))
        }
        (Literal::Int(a), BinaryOperator::Gt, Literal::Int(b)) => {
            Some(TypedExpr::literal(Literal::Boolean(a > b)))
        }
        (Literal::Int(a), BinaryOperator::GtEq, Literal::Int(b)) => {
            Some(TypedExpr::literal(Literal::Boolean(a >= b)))
        }

        // Boolean logic
        (Literal::Boolean(a), BinaryOperator::And, Literal::Boolean(b)) => {
            Some(TypedExpr::literal(Literal::Boolean(*a && *b)))
        }
        (Literal::Boolean(a), BinaryOperator::Or, Literal::Boolean(b)) => {
            Some(TypedExpr::literal(Literal::Boolean(*a || *b)))
        }

        // String comparisons
        (Literal::Text(a), BinaryOperator::Eq, Literal::Text(b)) => {
            Some(TypedExpr::literal(Literal::Boolean(a == b)))
        }
        (Literal::Text(a), BinaryOperator::NotEq, Literal::Text(b)) => {
            Some(TypedExpr::literal(Literal::Boolean(a != b)))
        }

        _ => None,
    }
}
