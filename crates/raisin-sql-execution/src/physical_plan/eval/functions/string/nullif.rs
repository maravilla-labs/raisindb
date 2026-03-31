//! NULLIF function - return NULL if two values are equal

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

/// Return NULL if two values are equal, otherwise return the first value
///
/// # SQL Signature
/// `NULLIF(value1, value2) -> ANY`
///
/// # Arguments
/// * `value1` - Value to return if not equal to value2
/// * `value2` - Value to compare against
///
/// # Returns
/// * NULL if value1 = value2
/// * value1 if value1 <> value2
///
/// # Examples
/// ```sql
/// SELECT NULLIF(5, 5) -> NULL
/// SELECT NULLIF(5, 0) -> 5
/// SELECT NULLIF('hello', 'world') -> 'hello'
/// SELECT NULLIF('hello', 'hello') -> NULL
/// SELECT NULLIF(NULL, 'x') -> NULL
/// SELECT NULLIF('x', NULL) -> 'x'
/// ```
///
/// # Notes
/// This is a standard SQL function (SQL-92) commonly used for:
/// - Avoiding division by zero: `a / NULLIF(b, 0)` returns NULL instead of error
/// - Converting empty strings to NULL: `NULLIF(column, '')`
/// - Converting special values to NULL: `NULLIF(status, 'null')`
pub struct NullIfFunction;

impl SqlFunction for NullIfFunction {
    fn name(&self) -> &str {
        "NULLIF"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::String
    }

    fn signature(&self) -> &str {
        "NULLIF(value1, value2) -> ANY"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        // Validate argument count - exactly 2 required
        if args.len() != 2 {
            return Err(Error::Validation(
                "NULLIF requires exactly 2 arguments".to_string(),
            ));
        }

        let val1 = eval_expr(&args[0], row)?;
        let val2 = eval_expr(&args[1], row)?;

        // If both are equal, return NULL
        if literals_equal(&val1, &val2) {
            Ok(Literal::Null)
        } else {
            Ok(val1)
        }
    }
}

/// Compare two literals for equality
///
/// Handles type coercion similar to SQL comparison semantics.
fn literals_equal(a: &Literal, b: &Literal) -> bool {
    match (a, b) {
        // NULL is never equal to anything (including NULL)
        (Literal::Null, _) | (_, Literal::Null) => false,

        // Same type comparisons
        (Literal::Boolean(a), Literal::Boolean(b)) => a == b,
        (Literal::Int(a), Literal::Int(b)) => a == b,
        (Literal::BigInt(a), Literal::BigInt(b)) => a == b,
        (Literal::Double(a), Literal::Double(b)) => a == b,
        (Literal::Text(a), Literal::Text(b)) => a == b,
        (Literal::Uuid(a), Literal::Uuid(b)) => a == b,
        (Literal::Path(a), Literal::Path(b)) => a == b,

        // Numeric type coercion
        (Literal::Int(a), Literal::BigInt(b)) => (*a as i64) == *b,
        (Literal::BigInt(a), Literal::Int(b)) => *a == (*b as i64),
        (Literal::Int(a), Literal::Double(b)) => (*a as f64) == *b,
        (Literal::Double(a), Literal::Int(b)) => *a == (*b as f64),
        (Literal::BigInt(a), Literal::Double(b)) => (*a as f64) == *b,
        (Literal::Double(a), Literal::BigInt(b)) => *a == (*b as f64),

        // String type coercion (Path and Uuid can be compared to Text)
        (Literal::Text(a), Literal::Path(b)) | (Literal::Path(b), Literal::Text(a)) => a == b,
        (Literal::Text(a), Literal::Uuid(b)) | (Literal::Uuid(b), Literal::Text(a)) => a == b,

        // JSON comparison
        (Literal::JsonB(a), Literal::JsonB(b)) => a == b,

        // Different types that can't be compared
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_sql::analyzer::DataType;

    fn eval_nullif(val1: Literal, val2: Literal) -> Result<Literal, Error> {
        let func = NullIfFunction;
        let expr1 = TypedExpr::new(
            raisin_sql::analyzer::Expr::Literal(val1.clone()),
            DataType::Text,
        );
        let expr2 = TypedExpr::new(
            raisin_sql::analyzer::Expr::Literal(val2.clone()),
            DataType::Text,
        );
        let row = Row::new();
        func.evaluate(&[expr1, expr2], &row)
    }

    #[test]
    fn test_nullif_equal_integers() {
        let result = eval_nullif(Literal::Int(5), Literal::Int(5)).unwrap();
        assert!(matches!(result, Literal::Null));
    }

    #[test]
    fn test_nullif_different_integers() {
        let result = eval_nullif(Literal::Int(5), Literal::Int(0)).unwrap();
        assert!(matches!(result, Literal::Int(5)));
    }

    #[test]
    fn test_nullif_equal_strings() {
        let result = eval_nullif(
            Literal::Text("hello".to_string()),
            Literal::Text("hello".to_string()),
        )
        .unwrap();
        assert!(matches!(result, Literal::Null));
    }

    #[test]
    fn test_nullif_different_strings() {
        let result = eval_nullif(
            Literal::Text("hello".to_string()),
            Literal::Text("world".to_string()),
        )
        .unwrap();
        match result {
            Literal::Text(s) => assert_eq!(s, "hello"),
            _ => panic!("Expected Text"),
        }
    }

    #[test]
    fn test_nullif_first_null() {
        let result = eval_nullif(Literal::Null, Literal::Text("x".to_string())).unwrap();
        // NULL is never equal to anything, so return first value (NULL)
        assert!(matches!(result, Literal::Null));
    }

    #[test]
    fn test_nullif_second_null() {
        let result = eval_nullif(Literal::Text("x".to_string()), Literal::Null).unwrap();
        // NULL is never equal to anything, so return first value
        match result {
            Literal::Text(s) => assert_eq!(s, "x"),
            _ => panic!("Expected Text"),
        }
    }

    #[test]
    fn test_nullif_with_null_string() {
        // This is the key use case: convert 'null' string to SQL NULL
        let result = eval_nullif(
            Literal::Text("null".to_string()),
            Literal::Text("null".to_string()),
        )
        .unwrap();
        assert!(matches!(result, Literal::Null));
    }
}
