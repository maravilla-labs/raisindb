//! Array method implementations

use crate::error::EvalError;
use crate::value::Value;

use super::super::comparison::values_equal;

/// Evaluate `first()` method
pub fn eval_first(val: &Value) -> Result<Value, EvalError> {
    match val {
        Value::Array(arr) => Ok(arr.first().cloned().unwrap_or(Value::Null)),
        _ => Err(EvalError::type_error("first", "array", val.type_name())),
    }
}

/// Evaluate `last()` method
pub fn eval_last(val: &Value) -> Result<Value, EvalError> {
    match val {
        Value::Array(arr) => Ok(arr.last().cloned().unwrap_or(Value::Null)),
        _ => Err(EvalError::type_error("last", "array", val.type_name())),
    }
}

/// Evaluate `indexOf()` method
/// Returns the index of the element in the array, or -1 if not found
pub fn eval_index_of(val: &Value, element: &Value) -> Result<Value, EvalError> {
    match val {
        Value::Array(arr) => {
            let idx = arr.iter().position(|item| values_equal(item, element));
            Ok(Value::Integer(idx.map(|i| i as i64).unwrap_or(-1)))
        }
        _ => Err(EvalError::type_error("indexOf", "array", val.type_name())),
    }
}

/// Evaluate `join()` method
/// Joins array elements into a string with an optional separator
pub fn eval_join(val: &Value, separator: Option<&Value>) -> Result<Value, EvalError> {
    match val {
        Value::Array(arr) => {
            let sep = match separator {
                Some(s) => s.as_str().unwrap_or(""),
                None => "",
            };
            let strings: Vec<String> = arr
                .iter()
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    Value::Integer(i) => i.to_string(),
                    Value::Float(f) => f.to_string(),
                    Value::Boolean(b) => b.to_string(),
                    Value::Null => "null".to_string(),
                    Value::Array(_) | Value::Object(_) => "[object]".to_string(),
                })
                .collect();
            Ok(Value::String(strings.join(sep)))
        }
        _ => Err(EvalError::type_error("join", "array", val.type_name())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_first() {
        let val = Value::Array(vec![
            Value::Integer(1),
            Value::Integer(2),
            Value::Integer(3),
        ]);
        assert_eq!(eval_first(&val).unwrap(), Value::Integer(1));

        let empty = Value::Array(vec![]);
        assert_eq!(eval_first(&empty).unwrap(), Value::Null);
    }

    #[test]
    fn test_last() {
        let val = Value::Array(vec![
            Value::Integer(1),
            Value::Integer(2),
            Value::Integer(3),
        ]);
        assert_eq!(eval_last(&val).unwrap(), Value::Integer(3));

        let empty = Value::Array(vec![]);
        assert_eq!(eval_last(&empty).unwrap(), Value::Null);
    }

    #[test]
    fn test_index_of() {
        let val = Value::Array(vec![
            Value::String("a".to_string()),
            Value::String("b".to_string()),
            Value::String("c".to_string()),
        ]);
        assert_eq!(
            eval_index_of(&val, &Value::String("b".to_string())).unwrap(),
            Value::Integer(1)
        );
        assert_eq!(
            eval_index_of(&val, &Value::String("z".to_string())).unwrap(),
            Value::Integer(-1)
        );
    }

    #[test]
    fn test_join() {
        let val = Value::Array(vec![
            Value::String("a".to_string()),
            Value::String("b".to_string()),
            Value::String("c".to_string()),
        ]);
        assert_eq!(
            eval_join(&val, Some(&Value::String(", ".to_string()))).unwrap(),
            Value::String("a, b, c".to_string())
        );
        assert_eq!(
            eval_join(&val, None).unwrap(),
            Value::String("abc".to_string())
        );
    }

    #[test]
    fn test_join_mixed_types() {
        let val = Value::Array(vec![
            Value::Integer(1),
            Value::Boolean(true),
            Value::String("x".to_string()),
        ]);
        assert_eq!(
            eval_join(&val, Some(&Value::String("-".to_string()))).unwrap(),
            Value::String("1-true-x".to_string())
        );
    }
}
