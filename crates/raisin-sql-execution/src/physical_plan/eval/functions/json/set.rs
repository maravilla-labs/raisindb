//! JSONB_SET function - set value at path in JSONB

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

/// Set a value at a path in a JSONB object
///
/// # SQL Signature
/// `JSONB_SET(target, path, new_value [, create_missing]) -> JSONB`
///
/// # Arguments
/// * `target` - JSONB value to modify
/// * `path` - Path to the key to set (PostgreSQL format: '{key}' or '{a,b,c}')
/// * `new_value` - New value to set at the path
/// * `create_missing` - Optional boolean, if true (default) creates missing keys
///
/// # Returns
/// * Modified JSONB value
/// * NULL if target is NULL
///
/// # Examples
/// ```sql
/// SELECT JSONB_SET('{"a": 1}', '{b}', '2') -> '{"a": 1, "b": 2}'
/// SELECT JSONB_SET('{"a": {"b": 1}}', '{a,b}', '2') -> '{"a": {"b": 2}}'
/// SELECT JSONB_SET('{"views": 0}', '{views}', to_jsonb(1)) -> '{"views": 1}'
/// ```
pub struct JsonbSetFunction;

impl SqlFunction for JsonbSetFunction {
    fn name(&self) -> &str {
        "JSONB_SET"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Json
    }

    fn signature(&self) -> &str {
        "JSONB_SET(target, path, new_value [, create_missing]) -> JSONB"
    }

    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() < 3 || args.len() > 4 {
            return Err(Error::Validation(
                "JSONB_SET requires 3 or 4 arguments (target, path, new_value [, create_missing])"
                    .to_string(),
            ));
        }

        let target_val = eval_expr(&args[0], row)?;
        let path_val = eval_expr(&args[1], row)?;
        let new_value_val = eval_expr(&args[2], row)?;

        // Optional create_missing parameter (default true)
        let create_missing = if args.len() == 4 {
            match eval_expr(&args[3], row)? {
                Literal::Boolean(b) => b,
                Literal::Null => true,
                _ => {
                    return Err(Error::Validation(
                        "JSONB_SET fourth argument must be boolean".to_string(),
                    ))
                }
            }
        } else {
            true
        };

        // Handle NULLs
        if matches!(target_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        let Literal::JsonB(mut target) = target_val else {
            return Err(Error::Validation(
                "JSONB_SET first argument must be JSONB".to_string(),
            ));
        };

        let Literal::Text(path_str) = path_val else {
            return Err(Error::Validation(
                "JSONB_SET second argument must be TEXT path (e.g., '{key}' or '{a,b}')"
                    .to_string(),
            ));
        };

        let new_value = match new_value_val {
            Literal::JsonB(j) => j,
            Literal::Null => serde_json::Value::Null,
            Literal::Text(s) => serde_json::Value::String(s),
            Literal::Int(i) => serde_json::Value::Number(i.into()),
            Literal::BigInt(i) => serde_json::Value::Number(i.into()),
            Literal::Double(f) => serde_json::Number::from_f64(f)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            Literal::Boolean(b) => serde_json::Value::Bool(b),
            _ => {
                return Err(Error::Validation(
                    "JSONB_SET third argument must be a JSON-compatible value".to_string(),
                ))
            }
        };

        // Parse path: '{key}' or '{a,b,c}'
        let keys = parse_path(&path_str)?;

        // Set the value at the path
        set_at_path(&mut target, &keys, new_value, create_missing)?;

        Ok(Literal::JsonB(target))
    }
}

/// Parse PostgreSQL-style path: '{key}' or '{a,b,c}'
fn parse_path(path: &str) -> Result<Vec<String>, Error> {
    let trimmed = path.trim();
    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return Err(Error::Validation(format!(
            "Invalid path format '{}'. Expected '{{key}}' or '{{a,b,c}}'",
            path
        )));
    }

    let inner = &trimmed[1..trimmed.len() - 1];
    if inner.is_empty() {
        return Err(Error::Validation("Empty path".to_string()));
    }

    Ok(inner.split(',').map(|s| s.trim().to_string()).collect())
}

/// Set value at nested path in JSON
fn set_at_path(
    target: &mut serde_json::Value,
    keys: &[String],
    value: serde_json::Value,
    create_missing: bool,
) -> Result<(), Error> {
    if keys.is_empty() {
        return Err(Error::Validation("Empty path".to_string()));
    }

    let mut current = target;

    // Navigate to parent of target key
    for key in &keys[..keys.len() - 1] {
        match current {
            serde_json::Value::Object(map) => {
                if !map.contains_key(key) {
                    if create_missing {
                        map.insert(
                            key.clone(),
                            serde_json::Value::Object(serde_json::Map::new()),
                        );
                    } else {
                        return Ok(()); // Path doesn't exist, do nothing
                    }
                }
                current = map.get_mut(key).unwrap();
            }
            serde_json::Value::Array(arr) => {
                let idx: usize = key
                    .parse()
                    .map_err(|_| Error::Validation(format!("Invalid array index: {}", key)))?;
                if idx >= arr.len() {
                    return Ok(()); // Index out of bounds
                }
                current = &mut arr[idx];
            }
            _ => return Ok(()), // Cannot traverse further
        }
    }

    // Set the final key
    let final_key = &keys[keys.len() - 1];
    match current {
        serde_json::Value::Object(map) => {
            if create_missing || map.contains_key(final_key) {
                map.insert(final_key.clone(), value);
            }
        }
        serde_json::Value::Array(arr) => {
            let idx: usize = final_key
                .parse()
                .map_err(|_| Error::Validation(format!("Invalid array index: {}", final_key)))?;
            if idx < arr.len() {
                arr[idx] = value;
            }
        }
        _ => {} // Cannot set on scalar
    }

    Ok(())
}
