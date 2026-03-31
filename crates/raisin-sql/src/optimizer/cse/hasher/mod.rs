//! Expression hashing for structural equality detection
//!
//! This module provides stable hashing for TypedExpr to detect structurally
//! identical expressions in SQL queries. Two expressions hash to the same value
//! if they represent the same computation, regardless of their memory location.

pub(crate) mod expr_hashing;
mod type_hashing;

pub use expr_hashing::ExprHasher;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::functions::{FunctionCategory, FunctionSignature};
    use crate::analyzer::{typed_expr::*, DataType, TypedExpr};

    #[test]
    fn test_identical_columns_hash_equal() {
        let expr1 = TypedExpr::column(
            "author".to_string(),
            "properties".to_string(),
            DataType::JsonB,
        );
        let expr2 = TypedExpr::column(
            "author".to_string(),
            "properties".to_string(),
            DataType::JsonB,
        );

        assert_eq!(ExprHasher::hash_expr(&expr1), ExprHasher::hash_expr(&expr2));
    }

    #[test]
    fn test_different_columns_hash_different() {
        let expr1 = TypedExpr::column(
            "author".to_string(),
            "properties".to_string(),
            DataType::JsonB,
        );
        let expr2 = TypedExpr::column("author".to_string(), "name".to_string(), DataType::Text);

        assert_ne!(ExprHasher::hash_expr(&expr1), ExprHasher::hash_expr(&expr2));
    }

    #[test]
    fn test_identical_json_extract_hash_equal() {
        let object = TypedExpr::column(
            "author".to_string(),
            "properties".to_string(),
            DataType::JsonB,
        );
        let key = TypedExpr::literal(Literal::Text("username".to_string()));

        let expr1 = TypedExpr::new(
            Expr::JsonExtractText {
                object: Box::new(object.clone()),
                key: Box::new(key.clone()),
            },
            DataType::Nullable(Box::new(DataType::Text)),
        );

        let expr2 = TypedExpr::new(
            Expr::JsonExtractText {
                object: Box::new(object),
                key: Box::new(key),
            },
            DataType::Nullable(Box::new(DataType::Text)),
        );

        assert_eq!(ExprHasher::hash_expr(&expr1), ExprHasher::hash_expr(&expr2));
    }

    #[test]
    fn test_different_json_keys_hash_different() {
        let object = TypedExpr::column(
            "author".to_string(),
            "properties".to_string(),
            DataType::JsonB,
        );

        let expr1 = TypedExpr::new(
            Expr::JsonExtractText {
                object: Box::new(object.clone()),
                key: Box::new(TypedExpr::literal(Literal::Text("username".to_string()))),
            },
            DataType::Nullable(Box::new(DataType::Text)),
        );

        let expr2 = TypedExpr::new(
            Expr::JsonExtractText {
                object: Box::new(object),
                key: Box::new(TypedExpr::literal(Literal::Text("displayName".to_string()))),
            },
            DataType::Nullable(Box::new(DataType::Text)),
        );

        assert_ne!(ExprHasher::hash_expr(&expr1), ExprHasher::hash_expr(&expr2));
    }

    #[test]
    fn test_identical_functions_hash_equal() {
        let path_lit = TypedExpr::literal(Literal::Path("/content/blog".to_string()));

        let expr1 = TypedExpr::new(
            Expr::Function {
                name: "DEPTH".to_string(),
                args: vec![path_lit.clone()],
                signature: FunctionSignature {
                    name: "DEPTH".to_string(),
                    params: vec![DataType::Path],
                    return_type: DataType::Int,
                    is_deterministic: true,
                    category: FunctionCategory::Hierarchy,
                },
                filter: None,
            },
            DataType::Int,
        );

        let expr2 = TypedExpr::new(
            Expr::Function {
                name: "DEPTH".to_string(),
                args: vec![path_lit],
                signature: FunctionSignature {
                    name: "DEPTH".to_string(),
                    params: vec![DataType::Path],
                    return_type: DataType::Int,
                    is_deterministic: true,
                    category: FunctionCategory::Hierarchy,
                },
                filter: None,
            },
            DataType::Int,
        );

        assert_eq!(ExprHasher::hash_expr(&expr1), ExprHasher::hash_expr(&expr2));
    }

    #[test]
    fn test_literals_hash_correctly() {
        let int1 = TypedExpr::literal(Literal::Int(42));
        let int2 = TypedExpr::literal(Literal::Int(42));
        let int3 = TypedExpr::literal(Literal::Int(43));

        assert_eq!(ExprHasher::hash_expr(&int1), ExprHasher::hash_expr(&int2));
        assert_ne!(ExprHasher::hash_expr(&int1), ExprHasher::hash_expr(&int3));
    }

    #[test]
    fn test_binary_ops_hash_correctly() {
        let left = TypedExpr::literal(Literal::Int(1));
        let right = TypedExpr::literal(Literal::Int(2));

        let add1 = TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(left.clone()),
                op: BinaryOperator::Add,
                right: Box::new(right.clone()),
            },
            DataType::Int,
        );

        let add2 = TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(left.clone()),
                op: BinaryOperator::Add,
                right: Box::new(right.clone()),
            },
            DataType::Int,
        );

        let multiply = TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(left),
                op: BinaryOperator::Multiply,
                right: Box::new(right),
            },
            DataType::Int,
        );

        assert_eq!(ExprHasher::hash_expr(&add1), ExprHasher::hash_expr(&add2));
        assert_ne!(
            ExprHasher::hash_expr(&add1),
            ExprHasher::hash_expr(&multiply)
        );
    }

    #[test]
    fn test_commutative_operators_canonical_hashing() {
        let a = TypedExpr::column("t".to_string(), "a".to_string(), DataType::Int);
        let b = TypedExpr::column("t".to_string(), "b".to_string(), DataType::Int);

        // a + b == b + a
        let a_plus_b = TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(a.clone()),
                op: BinaryOperator::Add,
                right: Box::new(b.clone()),
            },
            DataType::Int,
        );

        let b_plus_a = TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(b.clone()),
                op: BinaryOperator::Add,
                right: Box::new(a.clone()),
            },
            DataType::Int,
        );

        assert_eq!(
            ExprHasher::hash_expr(&a_plus_b),
            ExprHasher::hash_expr(&b_plus_a),
            "a + b and b + a should hash to the same value"
        );

        // a * b == b * a
        let a_times_b = TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(a.clone()),
                op: BinaryOperator::Multiply,
                right: Box::new(b.clone()),
            },
            DataType::Int,
        );

        let b_times_a = TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(b.clone()),
                op: BinaryOperator::Multiply,
                right: Box::new(a.clone()),
            },
            DataType::Int,
        );

        assert_eq!(
            ExprHasher::hash_expr(&a_times_b),
            ExprHasher::hash_expr(&b_times_a),
            "a * b and b * a should hash to the same value"
        );

        // a AND b == b AND a
        let a_bool = TypedExpr::column("t".to_string(), "a".to_string(), DataType::Boolean);
        let b_bool = TypedExpr::column("t".to_string(), "b".to_string(), DataType::Boolean);

        let a_and_b = TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(a_bool.clone()),
                op: BinaryOperator::And,
                right: Box::new(b_bool.clone()),
            },
            DataType::Boolean,
        );

        let b_and_a = TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(b_bool.clone()),
                op: BinaryOperator::And,
                right: Box::new(a_bool.clone()),
            },
            DataType::Boolean,
        );

        assert_eq!(
            ExprHasher::hash_expr(&a_and_b),
            ExprHasher::hash_expr(&b_and_a),
            "a AND b and b AND a should hash to the same value"
        );

        // a - b != b - a (non-commutative)
        let a_minus_b = TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(a.clone()),
                op: BinaryOperator::Subtract,
                right: Box::new(b.clone()),
            },
            DataType::Int,
        );

        let b_minus_a = TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(b.clone()),
                op: BinaryOperator::Subtract,
                right: Box::new(a),
            },
            DataType::Int,
        );

        assert_ne!(
            ExprHasher::hash_expr(&a_minus_b),
            ExprHasher::hash_expr(&b_minus_a),
            "a - b and b - a should hash to different values (not commutative)"
        );
    }
}
