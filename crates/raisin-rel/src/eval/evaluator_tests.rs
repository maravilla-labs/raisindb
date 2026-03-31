//! Tests for the expression evaluator

use super::evaluate;
use crate::error::EvalError;
use crate::eval::EvalContext;
use crate::parser::parse;
use crate::value::Value;

fn eval(input: &str, ctx: &EvalContext) -> Result<Value, EvalError> {
    let expr = parse(input).expect("Parse failed");
    evaluate(&expr, ctx)
}

fn empty_ctx() -> EvalContext {
    EvalContext::new()
}

fn simple_ctx() -> EvalContext {
    let mut ctx = EvalContext::new();
    ctx.set("x", Value::Integer(42));
    ctx.set("name", Value::String("test".to_string()));
    ctx.set("flag", Value::Boolean(true));
    ctx
}

fn complex_ctx() -> EvalContext {
    let json = serde_json::json!({
        "input": {
            "value": 42,
            "status": "active",
            "priority": 7,
            "urgent": false,
            "enabled": true,
            "name": "test-item",
            "tags": ["important", "urgent"]
        }
    });
    EvalContext::from_json(json).unwrap()
}

#[test]
fn test_literal() {
    let ctx = empty_ctx();
    assert_eq!(eval("42", &ctx).unwrap(), Value::Integer(42));
    assert_eq!(eval("3.14", &ctx).unwrap(), Value::Float(3.14));
    assert_eq!(
        eval("'hello'", &ctx).unwrap(),
        Value::String("hello".to_string())
    );
    assert_eq!(eval("true", &ctx).unwrap(), Value::Boolean(true));
    assert_eq!(eval("null", &ctx).unwrap(), Value::Null);
}

#[test]
fn test_variable() {
    let ctx = simple_ctx();
    assert_eq!(eval("x", &ctx).unwrap(), Value::Integer(42));
    assert_eq!(
        eval("name", &ctx).unwrap(),
        Value::String("test".to_string())
    );
}

#[test]
fn test_undefined_variable() {
    let ctx = empty_ctx();
    assert!(matches!(
        eval("undefined", &ctx).unwrap_err(),
        EvalError::UndefinedVariable(_)
    ));
}

#[test]
fn test_property_access() {
    let ctx = complex_ctx();
    assert_eq!(eval("input.value", &ctx).unwrap(), Value::Integer(42));
    assert_eq!(
        eval("input.status", &ctx).unwrap(),
        Value::String("active".to_string())
    );
}

#[test]
fn test_index_access() {
    let ctx = complex_ctx();
    assert_eq!(
        eval("input.tags[0]", &ctx).unwrap(),
        Value::String("important".to_string())
    );
    assert_eq!(
        eval("input.tags[1]", &ctx).unwrap(),
        Value::String("urgent".to_string())
    );
}

#[test]
fn test_comparison() {
    let ctx = simple_ctx();
    assert_eq!(eval("x == 42", &ctx).unwrap(), Value::Boolean(true));
    assert_eq!(eval("x != 42", &ctx).unwrap(), Value::Boolean(false));
    assert_eq!(eval("x > 10", &ctx).unwrap(), Value::Boolean(true));
    assert_eq!(eval("x < 100", &ctx).unwrap(), Value::Boolean(true));
    assert_eq!(eval("x >= 42", &ctx).unwrap(), Value::Boolean(true));
    assert_eq!(eval("x <= 42", &ctx).unwrap(), Value::Boolean(true));
}

#[test]
fn test_string_comparison() {
    let ctx = simple_ctx();
    assert_eq!(eval("name == 'test'", &ctx).unwrap(), Value::Boolean(true));
    assert_eq!(eval("name != 'other'", &ctx).unwrap(), Value::Boolean(true));
}

#[test]
fn test_logical_and() {
    let ctx = simple_ctx();
    assert_eq!(
        eval("x > 10 && x < 100", &ctx).unwrap(),
        Value::Boolean(true)
    );
    assert_eq!(
        eval("x > 100 && x < 200", &ctx).unwrap(),
        Value::Boolean(false)
    );
}

#[test]
fn test_logical_or() {
    let ctx = simple_ctx();
    assert_eq!(
        eval("x == 42 || x == 100", &ctx).unwrap(),
        Value::Boolean(true)
    );
    assert_eq!(
        eval("x == 1 || x == 2", &ctx).unwrap(),
        Value::Boolean(false)
    );
}

#[test]
fn test_not() {
    let ctx = simple_ctx();
    assert_eq!(eval("!flag", &ctx).unwrap(), Value::Boolean(false));
    assert_eq!(eval("!false", &ctx).unwrap(), Value::Boolean(true));
}

#[test]
fn test_negation() {
    let ctx = simple_ctx();
    assert_eq!(eval("-x", &ctx).unwrap(), Value::Integer(-42));
}

// === Method call tests (new method syntax) ===

#[test]
fn test_method_contains() {
    let ctx = simple_ctx();
    // String contains
    assert_eq!(
        eval("name.contains('es')", &ctx).unwrap(),
        Value::Boolean(true)
    );
    assert_eq!(
        eval("name.contains('xyz')", &ctx).unwrap(),
        Value::Boolean(false)
    );
}

#[test]
fn test_method_starts_with() {
    let ctx = simple_ctx();
    assert_eq!(
        eval("name.startsWith('te')", &ctx).unwrap(),
        Value::Boolean(true)
    );
    assert_eq!(
        eval("name.startsWith('st')", &ctx).unwrap(),
        Value::Boolean(false)
    );
}

#[test]
fn test_method_ends_with() {
    let ctx = simple_ctx();
    assert_eq!(
        eval("name.endsWith('st')", &ctx).unwrap(),
        Value::Boolean(true)
    );
    assert_eq!(
        eval("name.endsWith('te')", &ctx).unwrap(),
        Value::Boolean(false)
    );
}

#[test]
fn test_method_to_lower_case() {
    let mut ctx = EvalContext::new();
    ctx.set("name", Value::String("HELLO".to_string()));
    assert_eq!(
        eval("name.toLowerCase()", &ctx).unwrap(),
        Value::String("hello".to_string())
    );
}

#[test]
fn test_method_to_upper_case() {
    let ctx = simple_ctx();
    assert_eq!(
        eval("name.toUpperCase()", &ctx).unwrap(),
        Value::String("TEST".to_string())
    );
}

#[test]
fn test_method_trim() {
    let mut ctx = EvalContext::new();
    ctx.set("name", Value::String("  hello  ".to_string()));
    assert_eq!(
        eval("name.trim()", &ctx).unwrap(),
        Value::String("hello".to_string())
    );
}

#[test]
fn test_method_length() {
    let ctx = simple_ctx();
    assert_eq!(eval("name.length()", &ctx).unwrap(), Value::Integer(4)); // "test" = 4
}

#[test]
fn test_method_is_empty() {
    let mut ctx = EvalContext::new();
    ctx.set("empty", Value::String("".to_string()));
    ctx.set("nonempty", Value::String("x".to_string()));
    assert_eq!(eval("empty.isEmpty()", &ctx).unwrap(), Value::Boolean(true));
    assert_eq!(
        eval("nonempty.isEmpty()", &ctx).unwrap(),
        Value::Boolean(false)
    );
}

#[test]
fn test_method_chaining() {
    let mut ctx = EvalContext::new();
    ctx.set("name", Value::String("  HELLO WORLD  ".to_string()));
    // Chain: trim().toLowerCase().contains('hello')
    assert_eq!(
        eval("name.trim().toLowerCase().contains('hello')", &ctx).unwrap(),
        Value::Boolean(true)
    );
}

#[test]
fn test_array_contains() {
    let ctx = complex_ctx();
    assert_eq!(
        eval("input.tags.contains('important')", &ctx).unwrap(),
        Value::Boolean(true)
    );
    assert_eq!(
        eval("input.tags.contains('missing')", &ctx).unwrap(),
        Value::Boolean(false)
    );
}

#[test]
fn test_array_first_last() {
    let ctx = complex_ctx();
    assert_eq!(
        eval("input.tags.first()", &ctx).unwrap(),
        Value::String("important".to_string())
    );
    assert_eq!(
        eval("input.tags.last()", &ctx).unwrap(),
        Value::String("urgent".to_string())
    );
}

#[test]
fn test_array_index_of() {
    let ctx = complex_ctx();
    assert_eq!(
        eval("input.tags.indexOf('important')", &ctx).unwrap(),
        Value::Integer(0)
    );
    assert_eq!(
        eval("input.tags.indexOf('urgent')", &ctx).unwrap(),
        Value::Integer(1)
    );
    assert_eq!(
        eval("input.tags.indexOf('missing')", &ctx).unwrap(),
        Value::Integer(-1)
    );
}

#[test]
fn test_array_join() {
    let ctx = complex_ctx();
    assert_eq!(
        eval("input.tags.join(', ')", &ctx).unwrap(),
        Value::String("important, urgent".to_string())
    );
}

#[test]
fn test_path_parent() {
    let mut ctx = EvalContext::new();
    ctx.set("path", Value::String("/content/blog/post1".to_string()));
    assert_eq!(
        eval("path.parent()", &ctx).unwrap(),
        Value::String("/content/blog".to_string())
    );
    assert_eq!(
        eval("path.parent(2)", &ctx).unwrap(),
        Value::String("/content".to_string())
    );
}

#[test]
fn test_path_depth() {
    let mut ctx = EvalContext::new();
    ctx.set("path", Value::String("/content/blog/post1".to_string()));
    assert_eq!(eval("path.depth()", &ctx).unwrap(), Value::Integer(3));
}

#[test]
fn test_path_descendant_of() {
    let mut ctx = EvalContext::new();
    ctx.set("path", Value::String("/content/blog/post1".to_string()));
    assert_eq!(
        eval("path.descendantOf('/content')", &ctx).unwrap(),
        Value::Boolean(true)
    );
    assert_eq!(
        eval("path.descendantOf('/other')", &ctx).unwrap(),
        Value::Boolean(false)
    );
}

#[test]
fn test_path_child_of() {
    let mut ctx = EvalContext::new();
    ctx.set("path", Value::String("/content/blog".to_string()));
    assert_eq!(
        eval("path.childOf('/content')", &ctx).unwrap(),
        Value::Boolean(true)
    );

    ctx.set("deep", Value::String("/content/blog/post1".to_string()));
    assert_eq!(
        eval("deep.childOf('/content')", &ctx).unwrap(),
        Value::Boolean(false) // Not a direct child
    );
}

#[test]
fn test_complex_expression() {
    let ctx = complex_ctx();

    // Real-world examples with method syntax
    assert_eq!(
        eval("input.value > 10 && input.status == 'active'", &ctx).unwrap(),
        Value::Boolean(true)
    );

    assert_eq!(
        eval(
            "(input.priority >= 5 || input.urgent == true) && input.enabled == true",
            &ctx
        )
        .unwrap(),
        Value::Boolean(true)
    );

    // Method call syntax
    assert_eq!(
        eval("input.name.contains('test') && input.value > 0", &ctx).unwrap(),
        Value::Boolean(true)
    );

    assert_eq!(
        eval("input.tags[0] == 'important'", &ctx).unwrap(),
        Value::Boolean(true)
    );
}

#[test]
fn test_short_circuit_and() {
    // If left is false, right should not be evaluated
    let ctx = simple_ctx();
    // This would fail if right were evaluated (undefined variable)
    assert_eq!(
        eval("false && undefined", &ctx).unwrap(),
        Value::Boolean(false)
    );
}

#[test]
fn test_short_circuit_or() {
    // If left is true, right should not be evaluated
    let ctx = simple_ctx();
    // This would fail if right were evaluated (undefined variable)
    assert_eq!(
        eval("true || undefined", &ctx).unwrap(),
        Value::Boolean(true)
    );
}

#[test]
fn test_null_safe_access() {
    let ctx = EvalContext::with_var("x", Value::Null);
    // Accessing property on null should return null, not error
    assert_eq!(eval("x.foo", &ctx).unwrap(), Value::Null);
    assert_eq!(eval("x[0]", &ctx).unwrap(), Value::Null);
}

#[test]
fn test_null_safe_method_call() {
    let ctx = EvalContext::with_var("x", Value::Null);
    // Method call on null should return null, not error (like JS ?.)
    assert_eq!(eval("x.contains('test')", &ctx).unwrap(), Value::Null);
    assert_eq!(eval("x.toLowerCase()", &ctx).unwrap(), Value::Null);
}

#[test]
fn test_null_safe_chaining() {
    let json = serde_json::json!({
        "input": {}  // Missing "name" property
    });
    let ctx = EvalContext::from_json(json).unwrap();
    // input.name is null, so input.name.contains() should return null
    assert_eq!(
        eval("input.name.contains('test')", &ctx).unwrap(),
        Value::Null
    );
}

#[test]
fn test_arithmetic_basic() {
    let ctx = empty_ctx();
    assert_eq!(eval("1 + 2", &ctx).unwrap(), Value::Integer(3));
    assert_eq!(eval("10 - 3", &ctx).unwrap(), Value::Integer(7));
    assert_eq!(eval("4 * 5", &ctx).unwrap(), Value::Integer(20));
    assert_eq!(eval("10 / 3", &ctx).unwrap(), Value::Integer(3));
    assert_eq!(eval("10 % 3", &ctx).unwrap(), Value::Integer(1));
}

#[test]
fn test_arithmetic_with_property_access() {
    let json = serde_json::json!({
        "input": {
            "user": {
                "age": 30
            }
        }
    });
    let ctx = EvalContext::from_json(json).unwrap();
    assert_eq!(
        eval("input.user.age + 5", &ctx).unwrap(),
        Value::Integer(35)
    );
    assert_eq!(
        eval("input.user.age * 2", &ctx).unwrap(),
        Value::Integer(60)
    );
}

#[test]
fn test_arithmetic_mixed_types() {
    let ctx = empty_ctx();
    assert_eq!(eval("1 + 2.5", &ctx).unwrap(), Value::Float(3.5));
    assert_eq!(eval("2.5 + 1", &ctx).unwrap(), Value::Float(3.5));
    assert_eq!(eval("10.0 / 3.0", &ctx).unwrap(), Value::Float(10.0 / 3.0));
}

#[test]
fn test_arithmetic_string_concat() {
    let ctx = empty_ctx();
    assert_eq!(
        eval("'hello' + ' ' + 'world'", &ctx).unwrap(),
        Value::String("hello world".to_string())
    );
}

#[test]
fn test_arithmetic_precedence() {
    let ctx = empty_ctx();
    // * has higher precedence than +
    assert_eq!(eval("2 + 3 * 4", &ctx).unwrap(), Value::Integer(14));
    // Parentheses override
    assert_eq!(eval("(2 + 3) * 4", &ctx).unwrap(), Value::Integer(20));
    // Left to right for same precedence
    assert_eq!(eval("10 - 3 - 2", &ctx).unwrap(), Value::Integer(5));
}

#[test]
fn test_arithmetic_division_by_zero() {
    let ctx = empty_ctx();
    assert!(matches!(
        eval("10 / 0", &ctx).unwrap_err(),
        EvalError::DivisionByZero
    ));
    assert!(matches!(
        eval("10 % 0", &ctx).unwrap_err(),
        EvalError::DivisionByZero
    ));
}

#[test]
fn test_arithmetic_with_comparison() {
    let ctx = empty_ctx();
    // Arithmetic has higher precedence than comparison
    assert_eq!(
        eval("2 + 3 > 4", &ctx).unwrap(),
        Value::Boolean(true)
    );
    assert_eq!(
        eval("2 + 3 == 5", &ctx).unwrap(),
        Value::Boolean(true)
    );
}

#[test]
fn test_arithmetic_unary_minus() {
    let ctx = empty_ctx();
    assert_eq!(eval("-5 + 3", &ctx).unwrap(), Value::Integer(-2));
    assert_eq!(eval("5 + -3", &ctx).unwrap(), Value::Integer(2));
}

#[test]
fn test_boolean_existence_check() {
    let json_with_meta = serde_json::json!({
        "input": {
            "meta": {
                "published": true
            }
        }
    });
    let ctx = EvalContext::from_json(json_with_meta).unwrap();
    // input.meta exists, so it's truthy
    assert_eq!(
        eval("input.meta && input.meta.published", &ctx).unwrap(),
        Value::Boolean(true)
    );

    let json_without_meta = serde_json::json!({
        "input": {}
    });
    let ctx = EvalContext::from_json(json_without_meta).unwrap();
    // input.meta is null, so it's falsy - short circuits
    assert_eq!(
        eval("input.meta && input.meta.published", &ctx).unwrap(),
        Value::Boolean(false)
    );
}
