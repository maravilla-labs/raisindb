//! Universal methods that work on multiple value types (String, Array, Object)

use crate::error::EvalError;
use crate::value::Value;

use super::super::comparison::values_equal;

/// Evaluate `length()` method
/// Works on String, Array, and Object types
pub fn eval_length(val: &Value) -> Result<Value, EvalError> {
    match val {
        Value::String(s) => Ok(Value::Integer(s.len() as i64)),
        Value::Array(arr) => Ok(Value::Integer(arr.len() as i64)),
        Value::Object(obj) => Ok(Value::Integer(obj.len() as i64)),
        Value::Null => Ok(Value::Integer(0)),
        _ => Err(EvalError::type_error(
            "length",
            "string, array, or object",
            val.type_name(),
        )),
    }
}

/// Evaluate `isEmpty()` method
/// Returns true if the value is null, empty string, empty array, or empty object
pub fn eval_is_empty(val: &Value) -> Result<Value, EvalError> {
    Ok(Value::Boolean(match val {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(arr) => arr.is_empty(),
        Value::Object(obj) => obj.is_empty(),
        _ => false,
    }))
}

/// Evaluate `isNotEmpty()` method
/// Returns the negation of isEmpty()
pub fn eval_is_not_empty(val: &Value) -> Result<Value, EvalError> {
    let is_empty = eval_is_empty(val)?;
    match is_empty {
        Value::Boolean(b) => Ok(Value::Boolean(!b)),
        _ => Ok(Value::Boolean(false)),
    }
}

/// Evaluate `contains()` method (polymorphic)
/// For strings: checks if the string contains the substring
/// For arrays: checks if the array contains the element
pub fn eval_contains(val: &Value, needle: &Value) -> Result<Value, EvalError> {
    match val {
        Value::String(s) => {
            let needle_str = needle
                .as_str()
                .ok_or_else(|| EvalError::type_error("contains", "string", needle.type_name()))?;
            Ok(Value::Boolean(s.contains(needle_str)))
        }
        Value::Array(arr) => {
            // Check if array contains the element
            let found = arr.iter().any(|item| values_equal(item, needle));
            Ok(Value::Boolean(found))
        }
        _ => Err(EvalError::type_error(
            "contains",
            "string or array",
            val.type_name(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_length_string() {
        let val = Value::String("hello".to_string());
        assert_eq!(eval_length(&val).unwrap(), Value::Integer(5));
    }

    #[test]
    fn test_length_array() {
        let val = Value::Array(vec![Value::Integer(1), Value::Integer(2)]);
        assert_eq!(eval_length(&val).unwrap(), Value::Integer(2));
    }

    #[test]
    fn test_length_null() {
        assert_eq!(eval_length(&Value::Null).unwrap(), Value::Integer(0));
    }

    #[test]
    fn test_is_empty() {
        assert_eq!(
            eval_is_empty(&Value::String("".to_string())).unwrap(),
            Value::Boolean(true)
        );
        assert_eq!(
            eval_is_empty(&Value::String("x".to_string())).unwrap(),
            Value::Boolean(false)
        );
        assert_eq!(
            eval_is_empty(&Value::Array(vec![])).unwrap(),
            Value::Boolean(true)
        );
    }

    #[test]
    fn test_contains_string() {
        let val = Value::String("hello world".to_string());
        let needle = Value::String("world".to_string());
        assert_eq!(eval_contains(&val, &needle).unwrap(), Value::Boolean(true));
    }

    #[test]
    fn test_contains_array() {
        let val = Value::Array(vec![
            Value::Integer(1),
            Value::Integer(2),
            Value::Integer(3),
        ]);
        assert_eq!(
            eval_contains(&val, &Value::Integer(2)).unwrap(),
            Value::Boolean(true)
        );
        assert_eq!(
            eval_contains(&val, &Value::Integer(5)).unwrap(),
            Value::Boolean(false)
        );
    }
}
