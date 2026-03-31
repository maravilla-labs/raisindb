// TODO(v0.2): Clean up unused code and address lifetime elision warnings
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(mismatched_lifetime_syntaxes)]

//! # Raisin Expression Language (REL)
//!
//! A simple expression language for evaluating conditions in RaisinDB.
//!
//! ## Quick Start
//!
//! ```rust
//! use raisin_rel::{parse, evaluate, EvalContext, Value};
//! use std::collections::HashMap;
//!
//! // Parse an expression
//! let expr = parse("input.value > 10 && input.status == 'active'").unwrap();
//!
//! // Create evaluation context
//! let mut input = HashMap::new();
//! input.insert("value".to_string(), Value::Integer(42));
//! input.insert("status".to_string(), Value::String("active".to_string()));
//!
//! let mut ctx = EvalContext::new();
//! ctx.set("input", Value::Object(input));
//!
//! // Evaluate
//! let result = evaluate(&expr, &ctx).unwrap();
//! assert_eq!(result, Value::Boolean(true));
//! ```
//!
//! ## Expression Syntax
//!
//! ### Literals
//! - Strings: `'hello'` or `"world"`
//! - Numbers: `42`, `3.14`, `-10`
//! - Booleans: `true`, `false`
//! - Null: `null`
//! - Arrays: `[1, 2, 3]`
//! - Objects: `{key: 'value', num: 42}`
//!
//! ### Operators
//! - Comparison: `==`, `!=`, `>`, `<`, `>=`, `<=`
//! - Logical: `&&` (AND), `||` (OR), `!` (NOT)
//! - Unary: `-` (negation)
//!
//! ### Field Access
//! - Property: `input.value`, `context.user.name`
//! - Index: `input.tags[0]`, `data["key"]`
//!
//! ### Functions
//! - `contains(field, value)` - string contains
//! - `startsWith(field, value)` - string prefix
//! - `endsWith(field, value)` - string suffix
//!
//! ## Example Expressions
//!
//! ```text
//! input.value > 10
//! input.status == 'active'
//! input.value > 10 && input.status == 'active'
//! contains(input.name, 'test')
//! (input.priority >= 5 || input.urgent == true) && input.enabled == true
//! input.tags[0] == 'important'
//! ```

pub mod ast;
pub mod error;
pub mod eval;
mod parser;
pub mod value;

// Re-exports for convenience
pub use ast::{BinOp, Expr, Literal, RelDirection, UnOp};
pub use error::{EvalError, ParseError, RelError};
pub use eval::{
    evaluate, evaluate_async, requires_async, EvalContext, NoOpResolver, RelationResolver,
};
pub use value::Value;

/// Parse an expression string into an AST
///
/// # Example
///
/// ```rust
/// use raisin_rel::parse;
///
/// let expr = parse("x > 10").unwrap();
/// ```
pub fn parse(input: &str) -> Result<Expr, ParseError> {
    parser::parse(input)
}

/// Parse and evaluate an expression in one step
///
/// This is a convenience function that combines parsing and evaluation.
///
/// # Example
///
/// ```rust
/// use raisin_rel::{eval, EvalContext, Value};
///
/// let mut ctx = EvalContext::new();
/// ctx.set("x", Value::Integer(42));
///
/// let result = eval("x > 10", &ctx).unwrap();
/// assert_eq!(result, Value::Boolean(true));
/// ```
pub fn eval(input: &str, ctx: &EvalContext) -> Result<Value, RelError> {
    let expr = parse(input)?;
    Ok(evaluate(&expr, ctx)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_parse_and_eval() {
        let mut input = HashMap::new();
        input.insert("value".to_string(), Value::Integer(42));
        input.insert("status".to_string(), Value::String("active".to_string()));

        let mut ctx = EvalContext::new();
        ctx.set("input", Value::Object(input));

        let result = eval("input.value > 10 && input.status == 'active'", &ctx).unwrap();
        assert_eq!(result, Value::Boolean(true));
    }

    #[test]
    fn test_from_json() {
        let json = serde_json::json!({
            "input": {
                "value": 42,
                "status": "active"
            }
        });

        let ctx = EvalContext::from_json(json).unwrap();
        let result = eval("input.value > 10", &ctx).unwrap();
        assert_eq!(result, Value::Boolean(true));
    }

    #[test]
    fn test_methods() {
        let json = serde_json::json!({
            "name": "hello world"
        });

        let ctx = EvalContext::from_json(json).unwrap();

        // Method call syntax (new)
        assert_eq!(
            eval("name.contains('world')", &ctx).unwrap(),
            Value::Boolean(true)
        );
        assert_eq!(
            eval("name.startsWith('hello')", &ctx).unwrap(),
            Value::Boolean(true)
        );
        assert_eq!(
            eval("name.endsWith('world')", &ctx).unwrap(),
            Value::Boolean(true)
        );
    }

    #[test]
    fn test_complex_conditions() {
        let json = serde_json::json!({
            "input": {
                "priority": 7,
                "urgent": false,
                "enabled": true
            }
        });

        let ctx = EvalContext::from_json(json).unwrap();

        let result = eval(
            "(input.priority >= 5 || input.urgent == true) && input.enabled == true",
            &ctx,
        )
        .unwrap();
        assert_eq!(result, Value::Boolean(true));
    }

    #[test]
    fn test_ast_json_serialization() {
        let expr = parse("input.balbal == 34 && a.asdf <= 3").unwrap();
        let json = serde_json::to_string_pretty(&expr).unwrap();
        println!("AST JSON:\n{}", json);
        // This test just prints - check the output with --nocapture
    }

    #[test]
    fn test_relates_stringify() {
        // Test simple RELATES
        let expr = parse("a RELATES b VIA 'FRIENDS_WITH'").unwrap();
        assert_eq!(expr.to_string(), "a RELATES b VIA 'FRIENDS_WITH'");

        // Test with depth
        let expr = parse("a RELATES b VIA 'FRIENDS_WITH' DEPTH 1..3").unwrap();
        assert_eq!(
            expr.to_string(),
            "a RELATES b VIA 'FRIENDS_WITH' DEPTH 1..3"
        );

        // Test with direction
        let expr = parse("a RELATES b VIA 'FRIENDS_WITH' DIRECTION OUTGOING").unwrap();
        assert_eq!(
            expr.to_string(),
            "a RELATES b VIA 'FRIENDS_WITH' DIRECTION OUTGOING"
        );

        // Test with multiple types
        let expr = parse("a RELATES b VIA ['TYPE1', 'TYPE2']").unwrap();
        assert_eq!(expr.to_string(), "a RELATES b VIA ['TYPE1', 'TYPE2']");

        // Test full syntax
        let expr =
            parse("a RELATES b VIA ['TYPE1', 'TYPE2'] DEPTH 2..5 DIRECTION INCOMING").unwrap();
        assert_eq!(
            expr.to_string(),
            "a RELATES b VIA ['TYPE1', 'TYPE2'] DEPTH 2..5 DIRECTION INCOMING"
        );
    }

    #[tokio::test]
    async fn test_relates_async_evaluation() {
        let json = serde_json::json!({
            "node": {
                "created_by": "user123"
            },
            "auth": {
                "local_user_id": "user456"
            }
        });

        let ctx = EvalContext::from_json(json).unwrap();
        let resolver = NoOpResolver;

        // NoOpResolver always returns false
        let expr = parse("node.created_by RELATES auth.local_user_id VIA 'FRIENDS_WITH'").unwrap();
        let result = evaluate_async(&expr, &ctx, &resolver).await.unwrap();
        assert_eq!(result, Value::Boolean(false));
    }

    #[tokio::test]
    async fn test_relates_in_complex_expression() {
        let json = serde_json::json!({
            "input": {
                "value": 42
            },
            "node": {
                "created_by": "user123"
            },
            "auth": {
                "local_user_id": "user456"
            }
        });

        let ctx = EvalContext::from_json(json).unwrap();
        let resolver = NoOpResolver;

        // Test RELATES in boolean expression with short-circuit
        let expr = parse(
            "input.value > 10 && node.created_by RELATES auth.local_user_id VIA 'FRIENDS_WITH'",
        )
        .unwrap();
        let result = evaluate_async(&expr, &ctx, &resolver).await.unwrap();
        // First part is true, second is false (NoOpResolver), so result is false
        assert_eq!(result, Value::Boolean(false));

        // Test with OR - should short-circuit
        let expr = parse(
            "input.value > 100 || node.created_by RELATES auth.local_user_id VIA 'FRIENDS_WITH'",
        )
        .unwrap();
        let result = evaluate_async(&expr, &ctx, &resolver).await.unwrap();
        // First part is false, second is false, so result is false
        assert_eq!(result, Value::Boolean(false));
    }

    #[test]
    fn test_requires_async() {
        // Simple expressions don't require async
        let expr = parse("input.value > 10").unwrap();
        assert!(!requires_async(&expr));

        // RELATES requires async
        let expr = parse("a RELATES b VIA 'TYPE'").unwrap();
        assert!(requires_async(&expr));

        // Complex expression with RELATES requires async
        let expr = parse("input.value > 10 && a RELATES b VIA 'TYPE'").unwrap();
        assert!(requires_async(&expr));
    }
}
