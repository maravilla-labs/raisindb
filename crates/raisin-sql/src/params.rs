//! SQL Parameter Substitution
//!
//! Provides safe parameter substitution for SQL queries to prevent SQL injection.
//! This module handles replacing $1, $2, etc. placeholders with properly escaped values.

use raisin_error::Error;
use serde_json::Value as JsonValue;

/// Safely substitute parameters into a SQL query
///
/// Replaces $1, $2, etc. placeholders with properly quoted and escaped values.
/// This prevents SQL injection by ensuring all user input is properly escaped.
///
/// # Arguments
/// * `sql` - SQL query with $1, $2, etc. placeholders
/// * `params` - Array of parameter values (JSON values)
///
/// # Returns
/// * SQL string with parameters safely substituted
///
/// # Errors
/// * Returns error if parameter count doesn't match placeholders
/// * Returns error if parameter index is out of bounds
///
/// # Example
/// ```
/// use raisin_sql::substitute_params;
/// use serde_json::json;
///
/// let sql = "SELECT * FROM nodes WHERE id = $1 AND name = $2";
/// let params = vec![json!("abc123"), json!("John's Document")];
/// let result = substitute_params(sql, &params).unwrap();
/// // Result: SELECT * FROM nodes WHERE id = 'abc123' AND name = 'John''s Document'
/// ```
pub fn substitute_params(sql: &str, params: &[JsonValue]) -> Result<String, Error> {
    let mut result = String::with_capacity(sql.len() + params.len() * 20);
    let mut chars = sql.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '$' {
            // Check if this is a parameter placeholder
            let mut num_str = String::new();
            while let Some(&next_ch) = chars.peek() {
                if next_ch.is_ascii_digit() {
                    num_str.push(next_ch);
                    chars.next();
                } else {
                    break;
                }
            }

            if !num_str.is_empty() {
                // Parse parameter index (1-based)
                let param_idx: usize = num_str.parse().map_err(|_| {
                    Error::Validation(format!("Invalid parameter index: ${}", num_str))
                })?;

                if param_idx == 0 {
                    return Err(Error::Validation(
                        "Parameter indices must start at $1".to_string(),
                    ));
                }

                // Convert to 0-based array index
                let array_idx = param_idx - 1;

                if array_idx >= params.len() {
                    return Err(Error::Validation(format!(
                        "Parameter ${} not provided (only {} parameters given)",
                        param_idx,
                        params.len()
                    )));
                }

                // Substitute the parameter value
                let value_str = format_param_value(&params[array_idx]);
                result.push_str(&value_str);
            } else {
                // Not a parameter, just a dollar sign
                result.push('$');
            }
        } else {
            result.push(ch);
        }
    }

    Ok(result)
}

/// Format a JSON value for safe SQL substitution
///
/// Properly quotes and escapes values based on their type:
/// - Strings: Single-quoted with escaping
/// - Numbers: Unquoted
/// - Booleans: Unquoted true/false
/// - Null: NULL keyword
/// - Arrays/Objects: JSON string representation (single-quoted, escaped)
fn format_param_value(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => "NULL".to_string(),

        JsonValue::Bool(b) => b.to_string(),

        JsonValue::Number(n) => n.to_string(),

        JsonValue::String(s) => {
            // Escape single quotes by doubling them (SQL standard)
            let escaped = s.replace('\'', "''");
            format!("'{}'", escaped)
        }

        JsonValue::Array(_) | JsonValue::Object(_) => {
            // Serialize to JSON string and treat as string parameter
            let json_str = value.to_string();
            let escaped = json_str.replace('\'', "''");
            format!("'{}'", escaped)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_substitute_single_param() {
        let sql = "SELECT * FROM nodes WHERE id = $1";
        let params = vec![json!("abc123")];
        let result = substitute_params(sql, &params).unwrap();
        assert_eq!(result, "SELECT * FROM nodes WHERE id = 'abc123'");
    }

    #[test]
    fn test_substitute_multiple_params() {
        let sql = "SELECT * FROM nodes WHERE id = $1 AND name = $2";
        let params = vec![json!("abc123"), json!("Test Node")];
        let result = substitute_params(sql, &params).unwrap();
        assert_eq!(
            result,
            "SELECT * FROM nodes WHERE id = 'abc123' AND name = 'Test Node'"
        );
    }

    #[test]
    fn test_escape_single_quotes() {
        let sql = "SELECT * FROM nodes WHERE name = $1";
        let params = vec![json!("John's Document")];
        let result = substitute_params(sql, &params).unwrap();
        assert_eq!(
            result,
            "SELECT * FROM nodes WHERE name = 'John''s Document'"
        );
    }

    #[test]
    fn test_numeric_params() {
        let sql = "SELECT * FROM nodes WHERE version = $1 AND count > $2";
        let params = vec![json!(42), json!(3.14)];
        let result = substitute_params(sql, &params).unwrap();
        assert_eq!(
            result,
            "SELECT * FROM nodes WHERE version = 42 AND count > 3.14"
        );
    }

    #[test]
    fn test_boolean_params() {
        let sql = "SELECT * FROM nodes WHERE active = $1";
        let params = vec![json!(true)];
        let result = substitute_params(sql, &params).unwrap();
        assert_eq!(result, "SELECT * FROM nodes WHERE active = true");
    }

    #[test]
    fn test_null_param() {
        let sql = "SELECT * FROM nodes WHERE parent = $1";
        let params = vec![json!(null)];
        let result = substitute_params(sql, &params).unwrap();
        assert_eq!(result, "SELECT * FROM nodes WHERE parent = NULL");
    }

    #[test]
    fn test_json_object_param() {
        let sql = "SELECT * FROM nodes WHERE properties = $1";
        let params = vec![json!({"key": "value"})];
        let result = substitute_params(sql, &params).unwrap();
        assert_eq!(
            result,
            r#"SELECT * FROM nodes WHERE properties = '{"key":"value"}'"#
        );
    }

    #[test]
    fn test_param_out_of_bounds() {
        let sql = "SELECT * FROM nodes WHERE id = $2";
        let params = vec![json!("abc123")];
        let result = substitute_params(sql, &params);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Parameter $2 not provided"));
    }

    #[test]
    fn test_zero_param_index() {
        let sql = "SELECT * FROM nodes WHERE id = $0";
        let params = vec![json!("abc123")];
        let result = substitute_params(sql, &params);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must start at $1"));
    }

    #[test]
    fn test_no_params_in_query() {
        let sql = "SELECT * FROM nodes WHERE name = 'literal'";
        let params = vec![];
        let result = substitute_params(sql, &params).unwrap();
        assert_eq!(result, "SELECT * FROM nodes WHERE name = 'literal'");
    }

    #[test]
    fn test_dollar_sign_not_parameter() {
        let sql = "SELECT * FROM nodes WHERE price = $100.00";
        let params = vec![];
        let result = substitute_params(sql, &params);
        // $100 would be treated as $1 followed by "00.00"
        // Since no params provided, this should fail
        assert!(result.is_err());
    }

    #[test]
    fn test_repeated_param_usage() {
        let sql = "SELECT * FROM nodes WHERE id = $1 OR parent_id = $1";
        let params = vec![json!("abc123")];
        let result = substitute_params(sql, &params).unwrap();
        assert_eq!(
            result,
            "SELECT * FROM nodes WHERE id = 'abc123' OR parent_id = 'abc123'"
        );
    }

    #[test]
    fn test_sql_injection_prevention() {
        // Attempt SQL injection via parameter
        let sql = "SELECT * FROM nodes WHERE id = $1";
        let params = vec![json!("abc' OR '1'='1")];
        let result = substitute_params(sql, &params).unwrap();
        // Single quotes are escaped, preventing injection
        assert_eq!(
            result,
            "SELECT * FROM nodes WHERE id = 'abc'' OR ''1''=''1'"
        );
    }

    #[test]
    fn test_complex_query_with_multiple_params() {
        let sql = r#"
            SELECT * FROM workspace
            WHERE node_type = $1
            AND path LIKE $2
            AND version > $3
            ORDER BY created_at DESC
            LIMIT $4
        "#;
        let params = vec![json!("Post"), json!("/blog/%"), json!(5), json!(10)];
        let result = substitute_params(sql, &params).unwrap();
        assert!(result.contains("node_type = 'Post'"));
        assert!(result.contains("path LIKE '/blog/%'"));
        assert!(result.contains("version > 5"));
        assert!(result.contains("LIMIT 10"));
    }
}
