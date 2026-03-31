//! Value comparison and binary operation evaluation

use crate::error::EvalError;
use crate::value::Value;

/// Check if two values are equal
pub fn values_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Null, Value::Null) => true,
        (Value::Boolean(a), Value::Boolean(b)) => a == b,
        (Value::Integer(a), Value::Integer(b)) => a == b,
        (Value::Float(a), Value::Float(b)) => (a - b).abs() < f64::EPSILON,
        (Value::Integer(a), Value::Float(b)) | (Value::Float(b), Value::Integer(a)) => {
            (*a as f64 - b).abs() < f64::EPSILON
        }
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| values_equal(x, y))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, v)| b.get(k).is_some_and(|bv| values_equal(v, bv)))
        }
        _ => false,
    }
}

/// Compare two values using a comparison function
pub fn compare_values<F>(left: &Value, right: &Value, cmp_fn: F) -> Result<Value, EvalError>
where
    F: Fn(std::cmp::Ordering) -> bool,
{
    let ordering = match (left, right) {
        (Value::Integer(a), Value::Integer(b)) => a.cmp(b),
        (Value::Float(a), Value::Float(b)) => a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal),
        (Value::Integer(a), Value::Float(b)) => (*a as f64)
            .partial_cmp(b)
            .unwrap_or(std::cmp::Ordering::Equal),
        (Value::Float(a), Value::Integer(b)) => a
            .partial_cmp(&(*b as f64))
            .unwrap_or(std::cmp::Ordering::Equal),
        (Value::String(a), Value::String(b)) => a.cmp(b),
        _ => {
            return Err(EvalError::IncomparableTypes {
                left_type: left.type_name().to_string(),
                right_type: right.type_name().to_string(),
            })
        }
    };

    Ok(Value::Boolean(cmp_fn(ordering)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_values_equal_null() {
        assert!(values_equal(&Value::Null, &Value::Null));
        assert!(!values_equal(&Value::Null, &Value::Boolean(false)));
    }

    #[test]
    fn test_values_equal_boolean() {
        assert!(values_equal(&Value::Boolean(true), &Value::Boolean(true)));
        assert!(!values_equal(&Value::Boolean(true), &Value::Boolean(false)));
    }

    #[test]
    fn test_values_equal_integers() {
        assert!(values_equal(&Value::Integer(42), &Value::Integer(42)));
        assert!(!values_equal(&Value::Integer(42), &Value::Integer(43)));
    }

    #[test]
    fn test_values_equal_floats() {
        assert!(values_equal(&Value::Float(3.14), &Value::Float(3.14)));
        assert!(!values_equal(&Value::Float(3.14), &Value::Float(2.71)));
    }

    #[test]
    fn test_values_equal_int_float() {
        assert!(values_equal(&Value::Integer(42), &Value::Float(42.0)));
        assert!(values_equal(&Value::Float(42.0), &Value::Integer(42)));
        assert!(!values_equal(&Value::Integer(42), &Value::Float(42.5)));
    }

    #[test]
    fn test_values_equal_strings() {
        assert!(values_equal(
            &Value::String("hello".to_string()),
            &Value::String("hello".to_string())
        ));
        assert!(!values_equal(
            &Value::String("hello".to_string()),
            &Value::String("world".to_string())
        ));
    }

    #[test]
    fn test_values_equal_arrays() {
        let arr1 = Value::Array(vec![Value::Integer(1), Value::Integer(2)]);
        let arr2 = Value::Array(vec![Value::Integer(1), Value::Integer(2)]);
        let arr3 = Value::Array(vec![Value::Integer(1), Value::Integer(3)]);
        assert!(values_equal(&arr1, &arr2));
        assert!(!values_equal(&arr1, &arr3));
    }

    #[test]
    fn test_compare_integers() {
        assert_eq!(
            compare_values(&Value::Integer(5), &Value::Integer(3), |o| o.is_gt()).unwrap(),
            Value::Boolean(true)
        );
        assert_eq!(
            compare_values(&Value::Integer(3), &Value::Integer(5), |o| o.is_lt()).unwrap(),
            Value::Boolean(true)
        );
    }

    #[test]
    fn test_compare_strings() {
        assert_eq!(
            compare_values(
                &Value::String("b".to_string()),
                &Value::String("a".to_string()),
                |o| o.is_gt()
            )
            .unwrap(),
            Value::Boolean(true)
        );
    }

    #[test]
    fn test_compare_incompatible_types() {
        let result = compare_values(&Value::String("a".to_string()), &Value::Integer(1), |o| {
            o.is_gt()
        });
        assert!(result.is_err());
    }
}
