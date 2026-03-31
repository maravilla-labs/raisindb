//! Expression evaluator
//!
//! Core dispatch logic for evaluating expressions. Method implementations
//! live in the `methods` submodule; comparison helpers live in `comparison`.

use super::comparison::{compare_values, values_equal};
use super::context::EvalContext;
use super::methods::{
    eval_ancestor, eval_ancestor_of, eval_child_of, eval_contains, eval_depth, eval_descendant_of,
    eval_ends_with, eval_first, eval_index_of, eval_is_empty, eval_is_not_empty, eval_join,
    eval_last, eval_length, eval_parent, eval_starts_with, eval_substring, eval_to_lower_case,
    eval_to_upper_case, eval_trim,
};
use crate::ast::{BinOp, Expr, Literal, UnOp};
use crate::error::EvalError;
use crate::value::Value;

/// Evaluate an expression against a context
pub fn evaluate(expr: &Expr, ctx: &EvalContext) -> Result<Value, EvalError> {
    match expr {
        Expr::Literal(lit) => eval_literal(lit),
        Expr::Variable(name) => eval_variable(name, ctx),
        Expr::PropertyAccess { object, property } => eval_property_access(object, property, ctx),
        Expr::IndexAccess { object, index } => eval_index_access(object, index, ctx),
        Expr::BinaryOp { left, op, right } => eval_binary(left, *op, right, ctx),
        Expr::UnaryOp { op, expr } => eval_unary(*op, expr, ctx),
        Expr::MethodCall {
            object,
            method,
            args,
        } => eval_method(object, method, args, ctx),
        Expr::Grouped(inner) => evaluate(inner, ctx),
        Expr::Relates { .. } => Err(EvalError::type_error(
            "RELATES evaluation",
            "async evaluation with RelationResolver",
            "synchronous evaluate() function - use evaluate_async instead",
        )),
    }
}

/// Evaluate a literal to a value
fn eval_literal(lit: &Literal) -> Result<Value, EvalError> {
    Ok(match lit {
        Literal::Null => Value::Null,
        Literal::Boolean(b) => Value::Boolean(*b),
        Literal::Integer(i) => Value::Integer(*i),
        Literal::Float(f) => Value::Float(*f),
        Literal::String(s) => Value::String(s.clone()),
        Literal::Array(items) => {
            let values: Result<Vec<Value>, EvalError> = items.iter().map(eval_literal).collect();
            Value::Array(values?)
        }
        Literal::Object(fields) => {
            let mut map = std::collections::HashMap::new();
            for (key, value) in fields {
                map.insert(key.clone(), eval_literal(value)?);
            }
            Value::Object(map)
        }
    })
}

/// Evaluate a variable reference
fn eval_variable(name: &str, ctx: &EvalContext) -> Result<Value, EvalError> {
    ctx.get(name)
        .cloned()
        .ok_or_else(|| EvalError::undefined_variable(name))
}

/// Evaluate property access (object.property)
/// Null-safe: accessing property on null, missing property, or non-object returns null (like JS ?.)
fn eval_property_access(
    object: &Expr,
    property: &str,
    ctx: &EvalContext,
) -> Result<Value, EvalError> {
    let obj_value = evaluate(object, ctx)?;

    match &obj_value {
        Value::Object(map) => Ok(map.get(property).cloned().unwrap_or(Value::Null)),
        Value::Null => Ok(Value::Null), // Null-safe property access
        _ => Ok(Value::Null),           // Non-object.property -> null
    }
}

/// Evaluate index access (object[index])
fn eval_index_access(object: &Expr, index: &Expr, ctx: &EvalContext) -> Result<Value, EvalError> {
    let obj_value = evaluate(object, ctx)?;
    let index_value = evaluate(index, ctx)?;

    match &obj_value {
        Value::Array(arr) => {
            let idx = index_value
                .as_integer()
                .ok_or_else(|| EvalError::InvalidIndexType(index_value.type_name().to_string()))?;

            if idx < 0 {
                return Err(EvalError::index_out_of_bounds(idx, arr.len()));
            }

            arr.get(idx as usize)
                .cloned()
                .ok_or_else(|| EvalError::index_out_of_bounds(idx, arr.len()))
        }
        Value::Object(map) => {
            let key = index_value
                .as_str()
                .ok_or_else(|| EvalError::InvalidIndexType(index_value.type_name().to_string()))?;

            map.get(key)
                .cloned()
                .ok_or_else(|| EvalError::property_not_found(key, "object"))
        }
        Value::Null => Ok(Value::Null), // Null-safe index access
        other => Err(EvalError::type_error(
            "index access",
            "array or object",
            other.type_name(),
        )),
    }
}

/// Evaluate a binary operation
fn eval_binary(
    left: &Expr,
    op: BinOp,
    right: &Expr,
    ctx: &EvalContext,
) -> Result<Value, EvalError> {
    // Short-circuit evaluation for logical operators
    match op {
        BinOp::And => {
            let left_val = evaluate(left, ctx)?;
            if !left_val.is_truthy() {
                return Ok(Value::Boolean(false));
            }
            let right_val = evaluate(right, ctx)?;
            return Ok(Value::Boolean(right_val.is_truthy()));
        }
        BinOp::Or => {
            let left_val = evaluate(left, ctx)?;
            if left_val.is_truthy() {
                return Ok(Value::Boolean(true));
            }
            let right_val = evaluate(right, ctx)?;
            return Ok(Value::Boolean(right_val.is_truthy()));
        }
        _ => {}
    }

    let left_val = evaluate(left, ctx)?;
    let right_val = evaluate(right, ctx)?;

    eval_binary_values(&left_val, op, &right_val)
}

/// Evaluate a binary operation on already-evaluated values.
/// Shared by both sync and async evaluators.
pub fn eval_binary_values(
    left_val: &Value,
    op: BinOp,
    right_val: &Value,
) -> Result<Value, EvalError> {
    match op {
        BinOp::Eq => Ok(Value::Boolean(values_equal(left_val, right_val))),
        BinOp::Neq => Ok(Value::Boolean(!values_equal(left_val, right_val))),
        BinOp::Lt => compare_values(left_val, right_val, |cmp| cmp.is_lt()),
        BinOp::Gt => compare_values(left_val, right_val, |cmp| cmp.is_gt()),
        BinOp::Lte => compare_values(left_val, right_val, |cmp| cmp.is_le()),
        BinOp::Gte => compare_values(left_val, right_val, |cmp| cmp.is_ge()),
        BinOp::Add => eval_add(left_val, right_val),
        BinOp::Sub => eval_sub(left_val, right_val),
        BinOp::Mul => eval_mul(left_val, right_val),
        BinOp::Div => eval_div(left_val, right_val),
        BinOp::Mod => eval_mod(left_val, right_val),
        BinOp::And | BinOp::Or => unreachable!("Logical ops handled by caller"),
    }
}

/// Evaluate a unary operation
fn eval_unary(op: UnOp, expr: &Expr, ctx: &EvalContext) -> Result<Value, EvalError> {
    let value = evaluate(expr, ctx)?;

    match op {
        UnOp::Not => Ok(Value::Boolean(!value.is_truthy())),
        UnOp::Neg => match value {
            Value::Integer(i) => Ok(Value::Integer(-i)),
            Value::Float(f) => Ok(Value::Float(-f)),
            other => Err(EvalError::type_error(
                "negation",
                "number",
                other.type_name(),
            )),
        },
    }
}

/// Evaluate a method call (expr.method(args))
/// Null-safe: calling method on null returns null (like JS ?.)
fn eval_method(
    object: &Expr,
    method: &str,
    args: &[Expr],
    ctx: &EvalContext,
) -> Result<Value, EvalError> {
    let obj_val = evaluate(object, ctx)?;

    // Null-safe: method call on null returns null
    if obj_val == Value::Null {
        return Ok(Value::Null);
    }

    match method {
        // === Universal methods (work on String, Array, Object) ===
        "length" => eval_length(&obj_val),
        "isEmpty" => eval_is_empty(&obj_val),
        "isNotEmpty" => eval_is_not_empty(&obj_val),

        // === Polymorphic method: contains ===
        "contains" => {
            check_arg_count(method, args, 1)?;
            let needle = evaluate(&args[0], ctx)?;
            eval_contains(&obj_val, &needle)
        }

        // === String methods ===
        "startsWith" => {
            check_arg_count(method, args, 1)?;
            let prefix = evaluate(&args[0], ctx)?;
            eval_starts_with(&obj_val, &prefix)
        }
        "endsWith" => {
            check_arg_count(method, args, 1)?;
            let suffix = evaluate(&args[0], ctx)?;
            eval_ends_with(&obj_val, &suffix)
        }
        "toLowerCase" => {
            check_arg_count(method, args, 0)?;
            eval_to_lower_case(&obj_val)
        }
        "toUpperCase" => {
            check_arg_count(method, args, 0)?;
            eval_to_upper_case(&obj_val)
        }
        "trim" => {
            check_arg_count(method, args, 0)?;
            eval_trim(&obj_val)
        }
        "substring" => {
            if args.is_empty() || args.len() > 2 {
                return Err(EvalError::wrong_arg_count(method, 1, args.len()));
            }
            let start = evaluate(&args[0], ctx)?;
            let end = if args.len() > 1 {
                Some(evaluate(&args[1], ctx)?)
            } else {
                None
            };
            eval_substring(&obj_val, &start, end.as_ref())
        }

        // === Array methods ===
        "first" => {
            check_arg_count(method, args, 0)?;
            eval_first(&obj_val)
        }
        "last" => {
            check_arg_count(method, args, 0)?;
            eval_last(&obj_val)
        }
        "indexOf" => {
            check_arg_count(method, args, 1)?;
            let element = evaluate(&args[0], ctx)?;
            eval_index_of(&obj_val, &element)
        }
        "join" => {
            let separator = if !args.is_empty() {
                Some(evaluate(&args[0], ctx)?)
            } else {
                None
            };
            eval_join(&obj_val, separator.as_ref())
        }

        // === Path methods ===
        "parent" => {
            let levels = if !args.is_empty() {
                Some(evaluate(&args[0], ctx)?)
            } else {
                None
            };
            eval_parent(&obj_val, levels.as_ref())
        }
        "ancestor" => {
            check_arg_count(method, args, 1)?;
            let depth = evaluate(&args[0], ctx)?;
            eval_ancestor(&obj_val, &depth)
        }
        "ancestorOf" => {
            check_arg_count(method, args, 1)?;
            let other = evaluate(&args[0], ctx)?;
            eval_ancestor_of(&obj_val, &other)
        }
        "descendantOf" => {
            check_arg_count(method, args, 1)?;
            let parent = evaluate(&args[0], ctx)?;
            eval_descendant_of(&obj_val, &parent)
        }
        "childOf" => {
            check_arg_count(method, args, 1)?;
            let parent = evaluate(&args[0], ctx)?;
            eval_child_of(&obj_val, &parent)
        }
        "depth" => {
            check_arg_count(method, args, 0)?;
            eval_depth(&obj_val)
        }

        _ => Err(EvalError::unknown_method(method)),
    }
}

/// Check argument count
fn check_arg_count(method: &str, args: &[Expr], expected: usize) -> Result<(), EvalError> {
    if args.len() != expected {
        Err(EvalError::wrong_arg_count(method, expected, args.len()))
    } else {
        Ok(())
    }
}

/// Evaluate addition: int+int, float+float, mixed->float, string+string (concat)
fn eval_add(left: &Value, right: &Value) -> Result<Value, EvalError> {
    match (left, right) {
        (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(a + b)),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
        (Value::Integer(a), Value::Float(b)) => Ok(Value::Float(*a as f64 + b)),
        (Value::Float(a), Value::Integer(b)) => Ok(Value::Float(a + *b as f64)),
        (Value::String(a), Value::String(b)) => Ok(Value::String(format!("{}{}", a, b))),
        _ => Err(EvalError::type_error(
            "addition",
            "numbers or strings",
            format!("{} and {}", left.type_name(), right.type_name()),
        )),
    }
}

/// Evaluate subtraction: numeric only, with type coercion
fn eval_sub(left: &Value, right: &Value) -> Result<Value, EvalError> {
    match (left, right) {
        (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(a - b)),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
        (Value::Integer(a), Value::Float(b)) => Ok(Value::Float(*a as f64 - b)),
        (Value::Float(a), Value::Integer(b)) => Ok(Value::Float(a - *b as f64)),
        _ => Err(EvalError::type_error(
            "subtraction",
            "numbers",
            format!("{} and {}", left.type_name(), right.type_name()),
        )),
    }
}

/// Evaluate multiplication: numeric only, with type coercion
fn eval_mul(left: &Value, right: &Value) -> Result<Value, EvalError> {
    match (left, right) {
        (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(a * b)),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
        (Value::Integer(a), Value::Float(b)) => Ok(Value::Float(*a as f64 * b)),
        (Value::Float(a), Value::Integer(b)) => Ok(Value::Float(a * *b as f64)),
        _ => Err(EvalError::type_error(
            "multiplication",
            "numbers",
            format!("{} and {}", left.type_name(), right.type_name()),
        )),
    }
}

/// Evaluate division: numeric only, with division-by-zero error
fn eval_div(left: &Value, right: &Value) -> Result<Value, EvalError> {
    match (left, right) {
        (Value::Integer(_), Value::Integer(0)) => Err(EvalError::DivisionByZero),
        (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(a / b)),
        (Value::Float(a), Value::Float(b)) => {
            if *b == 0.0 {
                return Err(EvalError::DivisionByZero);
            }
            Ok(Value::Float(a / b))
        }
        (Value::Integer(a), Value::Float(b)) => {
            if *b == 0.0 {
                return Err(EvalError::DivisionByZero);
            }
            Ok(Value::Float(*a as f64 / b))
        }
        (Value::Float(a), Value::Integer(b)) => {
            if *b == 0 {
                return Err(EvalError::DivisionByZero);
            }
            Ok(Value::Float(a / *b as f64))
        }
        _ => Err(EvalError::type_error(
            "division",
            "numbers",
            format!("{} and {}", left.type_name(), right.type_name()),
        )),
    }
}

/// Evaluate modulo: numeric only, with division-by-zero error
fn eval_mod(left: &Value, right: &Value) -> Result<Value, EvalError> {
    match (left, right) {
        (Value::Integer(_), Value::Integer(0)) => Err(EvalError::DivisionByZero),
        (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(a % b)),
        (Value::Float(a), Value::Float(b)) => {
            if *b == 0.0 {
                return Err(EvalError::DivisionByZero);
            }
            Ok(Value::Float(a % b))
        }
        (Value::Integer(a), Value::Float(b)) => {
            if *b == 0.0 {
                return Err(EvalError::DivisionByZero);
            }
            Ok(Value::Float(*a as f64 % b))
        }
        (Value::Float(a), Value::Integer(b)) => {
            if *b == 0 {
                return Err(EvalError::DivisionByZero);
            }
            Ok(Value::Float(a % *b as f64))
        }
        _ => Err(EvalError::type_error(
            "modulo",
            "numbers",
            format!("{} and {}", left.type_name(), right.type_name()),
        )),
    }
}
