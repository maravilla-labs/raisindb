//! Typed expression types for the semantic analysis pipeline
//!
//! This module contains the typed expression tree that is the output of semantic analysis.
//! It includes:
//! - Core expression types (`TypedExpr`, `Expr`, `Literal`)
//! - Binary and unary operators with type inference
//! - Window function types and frame specifications

mod expressions;
mod operators;
mod window;

pub use expressions::{Expr, Literal, TypedExpr};
pub use operators::{BinaryOperator, UnaryOperator};
pub use window::{FrameBound, FrameMode, WindowFrame, WindowFunction};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::types::DataType;

    #[test]
    fn test_literal_data_types() {
        assert_eq!(Literal::Boolean(true).data_type(), DataType::Boolean);
        assert_eq!(Literal::Int(42).data_type(), DataType::Int);
        assert_eq!(Literal::BigInt(42).data_type(), DataType::BigInt);
        assert_eq!(Literal::Double(42.0).data_type(), DataType::Double);
        assert_eq!(Literal::Text("foo".into()).data_type(), DataType::Text);
        assert_eq!(Literal::Null.data_type(), DataType::Unknown);
    }

    #[test]
    fn test_typed_expr_literal() {
        let expr = TypedExpr::literal(Literal::Int(42));
        assert_eq!(expr.data_type, DataType::Int);
        assert!(matches!(expr.expr, Expr::Literal(Literal::Int(42))));
    }

    #[test]
    fn test_typed_expr_column() {
        let expr = TypedExpr::column("nodes".into(), "id".into(), DataType::Text);
        assert_eq!(expr.data_type, DataType::Text);
        assert!(matches!(
            expr.expr,
            Expr::Column {
                table: _,
                column: _
            }
        ));
    }

    #[test]
    fn test_arithmetic_operator_result_types() {
        // INT + INT = INT
        assert_eq!(
            BinaryOperator::Add.result_type(&DataType::Int, &DataType::Int),
            Some(DataType::Int)
        );

        // INT + BIGINT = BIGINT
        assert_eq!(
            BinaryOperator::Add.result_type(&DataType::Int, &DataType::BigInt),
            Some(DataType::BigInt)
        );

        // INT + DOUBLE = DOUBLE
        assert_eq!(
            BinaryOperator::Add.result_type(&DataType::Int, &DataType::Double),
            Some(DataType::Double)
        );

        // INT + TEXT = None (incompatible)
        assert_eq!(
            BinaryOperator::Add.result_type(&DataType::Int, &DataType::Text),
            None
        );
    }

    #[test]
    fn test_comparison_operator_result_types() {
        // INT = INT -> BOOLEAN
        assert_eq!(
            BinaryOperator::Eq.result_type(&DataType::Int, &DataType::Int),
            Some(DataType::Boolean)
        );

        // PATH = TEXT -> BOOLEAN (coercible)
        assert_eq!(
            BinaryOperator::Eq.result_type(&DataType::Path, &DataType::Text),
            Some(DataType::Boolean)
        );

        // INT = TEXT -> None (incompatible)
        assert_eq!(
            BinaryOperator::Eq.result_type(&DataType::Int, &DataType::Text),
            None
        );
    }

    #[test]
    fn test_timestamp_text_comparison() {
        // TIMESTAMPTZ < TEXT -> BOOLEAN (for cursor-based pagination)
        assert_eq!(
            BinaryOperator::Lt.result_type(&DataType::TimestampTz, &DataType::Text),
            Some(DataType::Boolean)
        );

        // TEXT < TIMESTAMPTZ -> BOOLEAN
        assert_eq!(
            BinaryOperator::Lt.result_type(&DataType::Text, &DataType::TimestampTz),
            Some(DataType::Boolean)
        );

        // TIMESTAMPTZ = TEXT -> BOOLEAN
        assert_eq!(
            BinaryOperator::Eq.result_type(&DataType::TimestampTz, &DataType::Text),
            Some(DataType::Boolean)
        );

        // TIMESTAMPTZ > TEXT -> BOOLEAN
        assert_eq!(
            BinaryOperator::Gt.result_type(&DataType::TimestampTz, &DataType::Text),
            Some(DataType::Boolean)
        );
    }

    #[test]
    fn test_logical_operator_result_types() {
        // BOOLEAN AND BOOLEAN -> BOOLEAN
        assert_eq!(
            BinaryOperator::And.result_type(&DataType::Boolean, &DataType::Boolean),
            Some(DataType::Boolean)
        );

        // INT AND INT -> None
        assert_eq!(
            BinaryOperator::And.result_type(&DataType::Int, &DataType::Int),
            None
        );
    }

    #[test]
    fn test_json_operator_result_types() {
        // JSONB ->> TEXT -> TEXT?
        assert_eq!(
            BinaryOperator::JsonExtract.result_type(&DataType::JsonB, &DataType::Text),
            Some(DataType::Nullable(Box::new(DataType::Text)))
        );

        // JSONB @> JSONB -> BOOLEAN
        assert_eq!(
            BinaryOperator::JsonContains.result_type(&DataType::JsonB, &DataType::JsonB),
            Some(DataType::Boolean)
        );

        // TEXT ->> TEXT -> None
        assert_eq!(
            BinaryOperator::JsonExtract.result_type(&DataType::Text, &DataType::Text),
            None
        );
    }

    #[test]
    fn test_unary_operator_result_types() {
        // NOT BOOLEAN -> BOOLEAN
        assert_eq!(
            UnaryOperator::Not.result_type(&DataType::Boolean),
            Some(DataType::Boolean)
        );

        // NOT INT -> None
        assert_eq!(UnaryOperator::Not.result_type(&DataType::Int), None);

        // -INT -> INT
        assert_eq!(
            UnaryOperator::Negate.result_type(&DataType::Int),
            Some(DataType::Int)
        );

        // -DOUBLE -> DOUBLE
        assert_eq!(
            UnaryOperator::Negate.result_type(&DataType::Double),
            Some(DataType::Double)
        );

        // -TEXT -> None
        assert_eq!(UnaryOperator::Negate.result_type(&DataType::Text), None);
    }
}
