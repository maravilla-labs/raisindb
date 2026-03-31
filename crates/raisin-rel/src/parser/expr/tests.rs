use super::expr;
use crate::ast::{BinOp, Expr, Literal, RelDirection, UnOp};
use crate::parser::common::Span;

fn span(s: &str) -> Span {
    Span::new(s)
}

fn parse(s: &str) -> Expr {
    let (rem, e) = expr(span(s)).unwrap();
    assert!(rem.fragment().is_empty(), "Unparsed: {}", rem.fragment());
    e
}

#[test]
fn test_literal_expr() {
    let e = parse("42");
    assert!(matches!(e, Expr::Literal(Literal::Integer(42))));

    let e = parse("'hello'");
    assert!(matches!(e, Expr::Literal(Literal::String(s)) if s == "hello"));

    let e = parse("true");
    assert!(matches!(e, Expr::Literal(Literal::Boolean(true))));
}

#[test]
fn test_variable() {
    let e = parse("input");
    assert!(matches!(e, Expr::Variable(name) if name == "input"));

    let e = parse("myVar123");
    assert!(matches!(e, Expr::Variable(name) if name == "myVar123"));
}

#[test]
fn test_property_access() {
    let e = parse("input.value");
    assert!(matches!(
        e,
        Expr::PropertyAccess { property, .. } if property == "value"
    ));

    let e = parse("input.user.name");
    // Should be (input.user).name
    assert!(matches!(e, Expr::PropertyAccess { property, .. } if property == "name"));
}

#[test]
fn test_index_access() {
    let e = parse("input[0]");
    assert!(matches!(e, Expr::IndexAccess { .. }));

    let e = parse("data['key']");
    assert!(matches!(e, Expr::IndexAccess { .. }));
}

#[test]
fn test_comparison() {
    let e = parse("x == 10");
    assert!(matches!(e, Expr::BinaryOp { op: BinOp::Eq, .. }));

    let e = parse("x != 10");
    assert!(matches!(e, Expr::BinaryOp { op: BinOp::Neq, .. }));

    let e = parse("x > 10");
    assert!(matches!(e, Expr::BinaryOp { op: BinOp::Gt, .. }));

    let e = parse("x < 10");
    assert!(matches!(e, Expr::BinaryOp { op: BinOp::Lt, .. }));

    let e = parse("x >= 10");
    assert!(matches!(e, Expr::BinaryOp { op: BinOp::Gte, .. }));

    let e = parse("x <= 10");
    assert!(matches!(e, Expr::BinaryOp { op: BinOp::Lte, .. }));
}

#[test]
fn test_logical_and() {
    let e = parse("a && b");
    assert!(matches!(e, Expr::BinaryOp { op: BinOp::And, .. }));
}

#[test]
fn test_logical_or() {
    let e = parse("a || b");
    assert!(matches!(e, Expr::BinaryOp { op: BinOp::Or, .. }));
}

#[test]
fn test_precedence() {
    // && has higher precedence than ||
    let e = parse("a || b && c");
    // Should be a || (b && c)
    if let Expr::BinaryOp { op, right, .. } = e {
        assert_eq!(op, BinOp::Or);
        assert!(matches!(*right, Expr::BinaryOp { op: BinOp::And, .. }));
    } else {
        panic!("Expected BinaryOp");
    }

    // Comparison has higher precedence than &&
    let e = parse("a > 10 && b < 5");
    if let Expr::BinaryOp { op, left, right } = e {
        assert_eq!(op, BinOp::And);
        assert!(matches!(*left, Expr::BinaryOp { op: BinOp::Gt, .. }));
        assert!(matches!(*right, Expr::BinaryOp { op: BinOp::Lt, .. }));
    } else {
        panic!("Expected BinaryOp");
    }
}

#[test]
fn test_unary_not() {
    let e = parse("!true");
    assert!(matches!(e, Expr::UnaryOp { op: UnOp::Not, .. }));

    let e = parse("!a && b");
    // Should be (!a) && b
    if let Expr::BinaryOp { op, left, .. } = e {
        assert_eq!(op, BinOp::And);
        assert!(matches!(*left, Expr::UnaryOp { op: UnOp::Not, .. }));
    } else {
        panic!("Expected BinaryOp");
    }
}

#[test]
fn test_parentheses() {
    let e = parse("(a || b) && c");
    // Should be (a || b) && c
    if let Expr::BinaryOp { op, left, .. } = e {
        assert_eq!(op, BinOp::And);
        if let Expr::Grouped(inner) = *left {
            assert!(matches!(*inner, Expr::BinaryOp { op: BinOp::Or, .. }));
        } else {
            panic!("Expected Grouped");
        }
    } else {
        panic!("Expected BinaryOp");
    }
}

#[test]
fn test_method_call() {
    // Simple method call with argument
    let e = parse("name.contains('test')");
    if let Expr::MethodCall { method, args, .. } = e {
        assert_eq!(method, "contains");
        assert_eq!(args.len(), 1);
    } else {
        panic!("Expected MethodCall, got {:?}", e);
    }

    // Method call with no arguments
    let e = parse("name.toLowerCase()");
    if let Expr::MethodCall { method, args, .. } = e {
        assert_eq!(method, "toLowerCase");
        assert_eq!(args.len(), 0);
    } else {
        panic!("Expected MethodCall");
    }

    // Chained property access with method call
    let e = parse("input.text.contains('hello')");
    if let Expr::MethodCall {
        method,
        args,
        object,
    } = e
    {
        assert_eq!(method, "contains");
        assert_eq!(args.len(), 1);
        // object should be input.text (property access)
        assert!(matches!(*object, Expr::PropertyAccess { property, .. } if property == "text"));
    } else {
        panic!("Expected MethodCall");
    }
}

#[test]
fn test_chained_method_calls() {
    // Method call chain: input.name.trim().toLowerCase()
    let e = parse("input.name.trim().toLowerCase()");
    if let Expr::MethodCall { method, object, .. } = e {
        assert_eq!(method, "toLowerCase");
        // object should be another method call (trim)
        if let Expr::MethodCall {
            method: inner_method,
            ..
        } = *object
        {
            assert_eq!(inner_method, "trim");
        } else {
            panic!("Expected inner MethodCall");
        }
    } else {
        panic!("Expected MethodCall");
    }

    // More complex: input.name.trim().toLowerCase().contains('admin')
    let e = parse("input.name.trim().toLowerCase().contains('admin')");
    assert!(matches!(e, Expr::MethodCall { method, .. } if method == "contains"));
}

#[test]
fn test_complex_expression() {
    // Real-world example from ConditionBuilder
    let e = parse("input.value > 10 && input.status == 'active'");
    assert!(matches!(e, Expr::BinaryOp { op: BinOp::And, .. }));

    let e = parse("(input.priority >= 5 || input.urgent == true) && input.enabled == true");
    if let Expr::BinaryOp { op, left, right } = e {
        assert_eq!(op, BinOp::And);
        assert!(matches!(*left, Expr::Grouped(_)));
        assert!(matches!(*right, Expr::BinaryOp { op: BinOp::Eq, .. }));
    } else {
        panic!("Expected BinaryOp");
    }

    // Method call syntax in complex expression
    let e = parse("input.name.contains('test') && input.count > 0");
    assert!(matches!(e, Expr::BinaryOp { op: BinOp::And, .. }));

    // Path method with comparison
    let e = parse("input.node.path.parent() == '/content/blog'");
    assert!(matches!(e, Expr::BinaryOp { op: BinOp::Eq, .. }));

    // Chained methods in expression
    let e = parse("input.name.trim().toLowerCase().contains('admin') || input.role == 'superuser'");
    assert!(matches!(e, Expr::BinaryOp { op: BinOp::Or, .. }));
}

#[test]
fn test_array_index() {
    let e = parse("input.tags[0]");
    if let Expr::IndexAccess { object, index } = e {
        assert!(matches!(*object, Expr::PropertyAccess { property, .. } if property == "tags"));
        assert!(matches!(*index, Expr::Literal(Literal::Integer(0))));
    } else {
        panic!("Expected IndexAccess");
    }
}

#[test]
fn test_whitespace_handling() {
    // Note: trailing whitespace is handled by the parser module's parse() function
    // The expr() function only handles leading/internal whitespace
    let e = parse("  input.value   >   10");
    assert!(matches!(e, Expr::BinaryOp { op: BinOp::Gt, .. }));

    let e = parse("a  &&  b  ||  c");
    assert!(matches!(e, Expr::BinaryOp { op: BinOp::Or, .. }));
}

#[test]
fn test_negative_numbers() {
    let e = parse("-42");
    // This is parsed as unary minus on 42
    assert!(matches!(e, Expr::UnaryOp { op: UnOp::Neg, .. }));

    let e = parse("x > -10");
    if let Expr::BinaryOp { right, .. } = e {
        assert!(matches!(*right, Expr::UnaryOp { op: UnOp::Neg, .. }));
    } else {
        panic!("Expected BinaryOp");
    }
}

#[test]
fn test_arithmetic_parsing() {
    let e = parse("1 + 2");
    assert!(matches!(e, Expr::BinaryOp { op: BinOp::Add, .. }));

    let e = parse("10 - 3");
    assert!(matches!(e, Expr::BinaryOp { op: BinOp::Sub, .. }));

    let e = parse("4 * 5");
    assert!(matches!(e, Expr::BinaryOp { op: BinOp::Mul, .. }));

    let e = parse("10 / 3");
    assert!(matches!(e, Expr::BinaryOp { op: BinOp::Div, .. }));

    let e = parse("10 % 3");
    assert!(matches!(e, Expr::BinaryOp { op: BinOp::Mod, .. }));
}

#[test]
fn test_arithmetic_precedence() {
    // * has higher precedence than +
    let e = parse("2 + 3 * 4");
    if let Expr::BinaryOp { op, left, right } = e {
        assert_eq!(op, BinOp::Add);
        assert!(matches!(*left, Expr::Literal(Literal::Integer(2))));
        assert!(matches!(*right, Expr::BinaryOp { op: BinOp::Mul, .. }));
    } else {
        panic!("Expected BinaryOp");
    }

    // Comparison has lower precedence than arithmetic
    let e = parse("a + 5 > 10");
    if let Expr::BinaryOp { op, left, right } = e {
        assert_eq!(op, BinOp::Gt);
        assert!(matches!(*left, Expr::BinaryOp { op: BinOp::Add, .. }));
        assert!(matches!(*right, Expr::Literal(Literal::Integer(10))));
    } else {
        panic!("Expected BinaryOp");
    }
}

#[test]
fn test_arithmetic_with_property_access() {
    let e = parse("input.user.age + 5");
    if let Expr::BinaryOp { op, left, right } = e {
        assert_eq!(op, BinOp::Add);
        assert!(matches!(*left, Expr::PropertyAccess { property, .. } if property == "age"));
        assert!(matches!(*right, Expr::Literal(Literal::Integer(5))));
    } else {
        panic!("Expected BinaryOp");
    }
}

#[test]
fn test_relates_simple() {
    let e = parse("node.created_by RELATES auth.local_user_id VIA 'FRIENDS_WITH'");
    if let Expr::Relates {
        relation_types,
        min_depth,
        max_depth,
        direction,
        ..
    } = e
    {
        assert_eq!(relation_types, vec!["FRIENDS_WITH"]);
        assert_eq!(min_depth, 1);
        assert_eq!(max_depth, 1);
        assert_eq!(direction, RelDirection::Any);
    } else {
        panic!("Expected Relates, got {:?}", e);
    }
}

#[test]
fn test_relates_with_depth() {
    let e = parse("node.created_by RELATES auth.local_user_id VIA 'FRIENDS_WITH' DEPTH 1..2");
    if let Expr::Relates {
        relation_types,
        min_depth,
        max_depth,
        direction,
        ..
    } = e
    {
        assert_eq!(relation_types, vec!["FRIENDS_WITH"]);
        assert_eq!(min_depth, 1);
        assert_eq!(max_depth, 2);
        assert_eq!(direction, RelDirection::Any);
    } else {
        panic!("Expected Relates, got {:?}", e);
    }
}

#[test]
fn test_relates_with_direction() {
    let e =
        parse("node.created_by RELATES auth.local_user_id VIA 'FRIENDS_WITH' DIRECTION OUTGOING");
    if let Expr::Relates {
        relation_types,
        min_depth,
        max_depth,
        direction,
        ..
    } = e
    {
        assert_eq!(relation_types, vec!["FRIENDS_WITH"]);
        assert_eq!(min_depth, 1);
        assert_eq!(max_depth, 1);
        assert_eq!(direction, RelDirection::Outgoing);
    } else {
        panic!("Expected Relates, got {:?}", e);
    }
}

#[test]
fn test_relates_multiple_types() {
    let e = parse("a RELATES b VIA ['FRIENDS_WITH', 'FOLLOWS']");
    if let Expr::Relates { relation_types, .. } = e {
        assert_eq!(relation_types, vec!["FRIENDS_WITH", "FOLLOWS"]);
    } else {
        panic!("Expected Relates, got {:?}", e);
    }
}

#[test]
fn test_relates_full() {
    let e = parse(
        "node.created_by RELATES auth.local_user_id VIA ['FRIENDS_WITH', 'FOLLOWS'] DEPTH 1..3 DIRECTION INCOMING",
    );
    if let Expr::Relates {
        relation_types,
        min_depth,
        max_depth,
        direction,
        ..
    } = e
    {
        assert_eq!(relation_types, vec!["FRIENDS_WITH", "FOLLOWS"]);
        assert_eq!(min_depth, 1);
        assert_eq!(max_depth, 3);
        assert_eq!(direction, RelDirection::Incoming);
    } else {
        panic!("Expected Relates, got {:?}", e);
    }
}

#[test]
fn test_relates_in_boolean_expr() {
    let e =
        parse("input.value > 10 && node.created_by RELATES auth.local_user_id VIA 'FRIENDS_WITH'");
    if let Expr::BinaryOp { op, left, right } = e {
        assert_eq!(op, BinOp::And);
        assert!(matches!(*left, Expr::BinaryOp { op: BinOp::Gt, .. }));
        assert!(matches!(*right, Expr::Relates { .. }));
    } else {
        panic!("Expected BinaryOp, got {:?}", e);
    }
}
