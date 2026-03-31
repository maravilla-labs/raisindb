//! String method implementations

use crate::error::EvalError;
use crate::value::Value;

/// Evaluate `startsWith()` method
pub fn eval_starts_with(val: &Value, prefix: &Value) -> Result<Value, EvalError> {
    let s = val
        .as_str()
        .ok_or_else(|| EvalError::type_error("startsWith", "string", val.type_name()))?;
    let prefix_str = prefix
        .as_str()
        .ok_or_else(|| EvalError::type_error("startsWith", "string", prefix.type_name()))?;
    Ok(Value::Boolean(s.starts_with(prefix_str)))
}

/// Evaluate `endsWith()` method
pub fn eval_ends_with(val: &Value, suffix: &Value) -> Result<Value, EvalError> {
    let s = val
        .as_str()
        .ok_or_else(|| EvalError::type_error("endsWith", "string", val.type_name()))?;
    let suffix_str = suffix
        .as_str()
        .ok_or_else(|| EvalError::type_error("endsWith", "string", suffix.type_name()))?;
    Ok(Value::Boolean(s.ends_with(suffix_str)))
}

/// Evaluate `toLowerCase()` method
pub fn eval_to_lower_case(val: &Value) -> Result<Value, EvalError> {
    let s = val
        .as_str()
        .ok_or_else(|| EvalError::type_error("toLowerCase", "string", val.type_name()))?;
    Ok(Value::String(s.to_lowercase()))
}

/// Evaluate `toUpperCase()` method
pub fn eval_to_upper_case(val: &Value) -> Result<Value, EvalError> {
    let s = val
        .as_str()
        .ok_or_else(|| EvalError::type_error("toUpperCase", "string", val.type_name()))?;
    Ok(Value::String(s.to_uppercase()))
}

/// Evaluate `trim()` method
pub fn eval_trim(val: &Value) -> Result<Value, EvalError> {
    let s = val
        .as_str()
        .ok_or_else(|| EvalError::type_error("trim", "string", val.type_name()))?;
    Ok(Value::String(s.trim().to_string()))
}

/// Evaluate `substring()` method
pub fn eval_substring(val: &Value, start: &Value, end: Option<&Value>) -> Result<Value, EvalError> {
    let s = val
        .as_str()
        .ok_or_else(|| EvalError::type_error("substring", "string", val.type_name()))?;
    let start_idx = start
        .as_integer()
        .ok_or_else(|| EvalError::type_error("substring", "integer", start.type_name()))?
        as usize;
    let end_idx = match end {
        Some(e) => e
            .as_integer()
            .ok_or_else(|| EvalError::type_error("substring", "integer", e.type_name()))?
            as usize,
        None => s.len(),
    };

    // Clamp indices to valid range
    let start_idx = start_idx.min(s.len());
    let end_idx = end_idx.min(s.len()).max(start_idx);

    Ok(Value::String(s[start_idx..end_idx].to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_starts_with() {
        let val = Value::String("hello world".to_string());
        assert_eq!(
            eval_starts_with(&val, &Value::String("hello".to_string())).unwrap(),
            Value::Boolean(true)
        );
        assert_eq!(
            eval_starts_with(&val, &Value::String("world".to_string())).unwrap(),
            Value::Boolean(false)
        );
    }

    #[test]
    fn test_ends_with() {
        let val = Value::String("hello world".to_string());
        assert_eq!(
            eval_ends_with(&val, &Value::String("world".to_string())).unwrap(),
            Value::Boolean(true)
        );
        assert_eq!(
            eval_ends_with(&val, &Value::String("hello".to_string())).unwrap(),
            Value::Boolean(false)
        );
    }

    #[test]
    fn test_to_lower_case() {
        let val = Value::String("HELLO".to_string());
        assert_eq!(
            eval_to_lower_case(&val).unwrap(),
            Value::String("hello".to_string())
        );
    }

    #[test]
    fn test_to_upper_case() {
        let val = Value::String("hello".to_string());
        assert_eq!(
            eval_to_upper_case(&val).unwrap(),
            Value::String("HELLO".to_string())
        );
    }

    #[test]
    fn test_trim() {
        let val = Value::String("  hello  ".to_string());
        assert_eq!(eval_trim(&val).unwrap(), Value::String("hello".to_string()));
    }

    #[test]
    fn test_substring() {
        let val = Value::String("hello world".to_string());
        assert_eq!(
            eval_substring(&val, &Value::Integer(0), Some(&Value::Integer(5))).unwrap(),
            Value::String("hello".to_string())
        );
        assert_eq!(
            eval_substring(&val, &Value::Integer(6), None).unwrap(),
            Value::String("world".to_string())
        );
    }

    #[test]
    fn test_substring_clamping() {
        let val = Value::String("hello".to_string());
        // Start beyond length
        assert_eq!(
            eval_substring(&val, &Value::Integer(100), None).unwrap(),
            Value::String("".to_string())
        );
        // End beyond length
        assert_eq!(
            eval_substring(&val, &Value::Integer(0), Some(&Value::Integer(100))).unwrap(),
            Value::String("hello".to_string())
        );
    }
}
