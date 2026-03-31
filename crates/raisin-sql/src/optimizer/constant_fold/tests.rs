use super::*;
use crate::analyzer::functions::{FunctionCategory, FunctionSignature};
use crate::analyzer::BinaryOperator;

#[test]
fn test_fold_integer_arithmetic() {
    // 1 + 2 → 3
    let expr = TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(TypedExpr::literal(Literal::Int(1))),
            op: BinaryOperator::Add,
            right: Box::new(TypedExpr::literal(Literal::Int(2))),
        },
        DataType::Int,
    );

    let folded = fold_constants(expr);
    assert!(matches!(folded.expr, Expr::Literal(Literal::Int(3))));
}

#[test]
fn test_fold_boolean_logic() {
    // true AND false → false
    let expr = TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(TypedExpr::literal(Literal::Boolean(true))),
            op: BinaryOperator::And,
            right: Box::new(TypedExpr::literal(Literal::Boolean(false))),
        },
        DataType::Boolean,
    );

    let folded = fold_constants(expr);
    assert!(matches!(
        folded.expr,
        Expr::Literal(Literal::Boolean(false))
    ));
}

#[test]
fn test_fold_depth_function() {
    // DEPTH('/content/blog') → 2
    let arg = TypedExpr::literal(Literal::Path("/content/blog".to_string()));
    let expr = TypedExpr::new(
        Expr::Function {
            name: "DEPTH".to_string(),
            args: vec![arg],
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

    let folded = fold_constants(expr);
    assert!(matches!(folded.expr, Expr::Literal(Literal::Int(2))));
}

#[test]
fn test_fold_parent_function() {
    // PARENT('/content/blog/post1') → '/content/blog'
    let arg = TypedExpr::literal(Literal::Path("/content/blog/post1".to_string()));
    let expr = TypedExpr::new(
        Expr::Function {
            name: "PARENT".to_string(),
            args: vec![arg],
            signature: FunctionSignature {
                name: "PARENT".to_string(),
                params: vec![DataType::Path],
                return_type: DataType::Nullable(Box::new(DataType::Path)),
                is_deterministic: true,
                category: FunctionCategory::Hierarchy,
            },
            filter: None,
        },
        DataType::Nullable(Box::new(DataType::Path)),
    );

    let folded = fold_constants(expr);
    if let Expr::Literal(Literal::Path(p)) = &folded.expr {
        assert_eq!(p, "/content/blog");
    } else {
        panic!("Expected folded path literal");
    }
}

#[test]
fn test_fold_path_starts_with() {
    // PATH_STARTS_WITH('/content/blog', '/content/') → true
    let path_arg = TypedExpr::literal(Literal::Path("/content/blog".to_string()));
    let prefix_arg = TypedExpr::literal(Literal::Path("/content/".to_string()));

    let expr = TypedExpr::new(
        Expr::Function {
            name: "PATH_STARTS_WITH".to_string(),
            args: vec![path_arg, prefix_arg],
            signature: FunctionSignature {
                name: "PATH_STARTS_WITH".to_string(),
                params: vec![DataType::Path, DataType::Path],
                return_type: DataType::Boolean,
                is_deterministic: true,
                category: FunctionCategory::Hierarchy,
            },
            filter: None,
        },
        DataType::Boolean,
    );

    let folded = fold_constants(expr);
    assert!(matches!(folded.expr, Expr::Literal(Literal::Boolean(true))));
}

#[test]
fn test_fold_lower_function() {
    // LOWER('HELLO') → 'hello'
    let arg = TypedExpr::literal(Literal::Text("HELLO".to_string()));
    let expr = TypedExpr::new(
        Expr::Function {
            name: "LOWER".to_string(),
            args: vec![arg],
            signature: FunctionSignature {
                name: "LOWER".to_string(),
                params: vec![DataType::Text],
                return_type: DataType::Text,
                is_deterministic: true,
                category: FunctionCategory::Scalar,
            },
            filter: None,
        },
        DataType::Text,
    );

    let folded = fold_constants(expr);
    if let Expr::Literal(Literal::Text(s)) = &folded.expr {
        assert_eq!(s, "hello");
    } else {
        panic!("Expected folded text literal");
    }
}

#[test]
fn test_fold_is_null() {
    // NULL IS NULL → true
    let expr = TypedExpr::new(
        Expr::IsNull {
            expr: Box::new(TypedExpr::literal(Literal::Null)),
        },
        DataType::Boolean,
    );

    let folded = fold_constants(expr);
    assert!(matches!(folded.expr, Expr::Literal(Literal::Boolean(true))));

    // 42 IS NULL → false
    let expr = TypedExpr::new(
        Expr::IsNull {
            expr: Box::new(TypedExpr::literal(Literal::Int(42))),
        },
        DataType::Boolean,
    );

    let folded = fold_constants(expr);
    assert!(matches!(
        folded.expr,
        Expr::Literal(Literal::Boolean(false))
    ));
}

#[test]
fn test_fold_cast() {
    // CAST(42 AS TEXT) → '42'
    let expr = TypedExpr::new(
        Expr::Cast {
            expr: Box::new(TypedExpr::literal(Literal::Int(42))),
            target_type: DataType::Text,
        },
        DataType::Text,
    );

    let folded = fold_constants(expr);
    if let Expr::Literal(Literal::Text(s)) = &folded.expr {
        assert_eq!(s, "42");
    } else {
        panic!("Expected folded text literal");
    }
}

#[test]
fn test_fold_nested_arithmetic() {
    // (1 + 2) * 3 → 3 * 3 → 9
    let inner = TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(TypedExpr::literal(Literal::Int(1))),
            op: BinaryOperator::Add,
            right: Box::new(TypedExpr::literal(Literal::Int(2))),
        },
        DataType::Int,
    );

    let expr = TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(inner),
            op: BinaryOperator::Multiply,
            right: Box::new(TypedExpr::literal(Literal::Int(3))),
        },
        DataType::Int,
    );

    let folded = fold_constants(expr);
    assert!(matches!(folded.expr, Expr::Literal(Literal::Int(9))));
}

#[test]
fn test_no_fold_with_column() {
    // id + 2 → can't fold (id is not a constant)
    let expr = TypedExpr::new(
        Expr::BinaryOp {
            left: Box::new(TypedExpr::column(
                "nodes".to_string(),
                "id".to_string(),
                DataType::Int,
            )),
            op: BinaryOperator::Add,
            right: Box::new(TypedExpr::literal(Literal::Int(2))),
        },
        DataType::Int,
    );

    let folded = fold_constants(expr);
    // Should remain a BinaryOp
    assert!(matches!(folded.expr, Expr::BinaryOp { .. }));
}
