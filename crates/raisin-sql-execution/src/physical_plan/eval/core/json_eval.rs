//! JSON operator evaluation
//!
//! Evaluates all JSON-related expression variants: `->`, `->>`, `@>`, `?`,
//! `?|`, `?&`, `#>`, `#>>`, `-`, `#-`, `@@`, `@?`.

use crate::physical_plan::executor::Row;
use crate::physical_plan::types::from_property_value;
use raisin_error::Error;
use raisin_sql::analyzer::{Expr, Literal, TypedExpr};

use super::super::json_ops::json_contains;

/// Helper function to recursively remove a value at a path in JSON
///
/// Navigates through a JSON structure following a path and removes the value
/// at the final path element. If the path doesn't exist, returns the original
/// value unchanged.
fn remove_at_path(
    mut value: serde_json::Value,
    path: &[serde_json::Value],
    depth: usize,
) -> serde_json::Value {
    if depth >= path.len() {
        return value;
    }

    let path_elem = &path[depth];

    // If this is the last element in the path, remove it
    if depth == path.len() - 1 {
        match (&mut value, path_elem) {
            (serde_json::Value::Object(ref mut map), serde_json::Value::String(key)) => {
                map.remove(key);
            }
            (serde_json::Value::Array(ref mut arr), serde_json::Value::Number(idx)) => {
                if let Some(i) = idx.as_u64() {
                    let idx_usize = i as usize;
                    if idx_usize < arr.len() {
                        arr.remove(idx_usize);
                    }
                }
            }
            _ => {}
        }
        return value;
    }

    // Recursive case: navigate deeper
    match (&mut value, path_elem) {
        (serde_json::Value::Object(ref mut map), serde_json::Value::String(key)) => {
            if let Some(child) = map.remove(key) {
                let modified_child = remove_at_path(child, path, depth + 1);
                map.insert(key.clone(), modified_child);
            }
        }
        (serde_json::Value::Array(ref mut arr), serde_json::Value::Number(idx)) => {
            if let Some(i) = idx.as_u64() {
                let idx_usize = i as usize;
                if idx_usize < arr.len() {
                    let child = arr[idx_usize].clone();
                    let modified_child = remove_at_path(child, path, depth + 1);
                    arr[idx_usize] = modified_child;
                }
            }
        }
        _ => {}
    }

    value
}

/// Evaluate JSON extract: `obj -> 'key'` (returns JSONB)
pub(super) fn eval_json_extract(
    object: &TypedExpr,
    key: &TypedExpr,
    row: &Row,
) -> Result<Literal, Error> {
    let obj_lit = super::eval_expr(object, row)?;
    let key_lit = super::eval_expr(key, row)?;

    match (obj_lit, key_lit) {
        (Literal::JsonB(json), Literal::Text(key)) => match json.get(&key) {
            Some(value) => Ok(Literal::JsonB(value.clone())),
            None => Ok(Literal::Null),
        },
        _ => Err(Error::Validation(
            "JSON extract (->) requires JSONB and TEXT key".to_string(),
        )),
    }
}

/// Evaluate JSON extract text: `obj ->> 'key'` (returns TEXT)
pub(super) fn eval_json_extract_text(
    object: &TypedExpr,
    key: &TypedExpr,
    row: &Row,
) -> Result<Literal, Error> {
    // Check if pre-computed (e.g., in GROUP BY)
    if let Expr::Column { table, column } = &object.expr {
        if let Expr::Literal(Literal::Text(key_str)) = &key.expr {
            let synthetic_name = format!("{}.{}_{}", table, column, key_str);
            if let Some(value) = row.get(&synthetic_name) {
                return from_property_value(value).map_err(Error::Backend);
            }
        }
    }

    let obj_lit = super::eval_expr(object, row)?;
    let key_lit = super::eval_expr(key, row)?;

    match (obj_lit, key_lit) {
        (Literal::JsonB(json), Literal::Text(key)) => {
            let value = json.get(&key);
            match value {
                Some(v) if v.is_null() => Ok(Literal::Null),
                Some(v) if v.is_string() => Ok(Literal::Text(v.as_str().unwrap().to_string())),
                Some(v) => Ok(Literal::Text(v.to_string())),
                None => Ok(Literal::Null),
            }
        }
        (Literal::Null, Literal::Text(_)) => Ok(Literal::Null),
        (obj, key) => {
            tracing::error!(
                "JSON extraction type mismatch: obj={:?} (expected JSONB), key={:?} (expected TEXT)",
                obj,
                key
            );
            Err(Error::Validation(
                "JSON extract text (->>) requires JSONB and TEXT key".to_string(),
            ))
        }
    }
}

/// Evaluate JSON contains: `obj @> pattern`
pub(super) fn eval_json_contains(
    object: &TypedExpr,
    pattern: &TypedExpr,
    row: &Row,
) -> Result<Literal, Error> {
    let obj_lit = super::eval_expr(object, row)?;
    let pattern_lit = super::eval_expr(pattern, row)?;

    match (obj_lit, pattern_lit) {
        (Literal::JsonB(obj), Literal::JsonB(pattern)) => {
            Ok(Literal::Boolean(json_contains(&obj, &pattern)))
        }
        (Literal::Null, _) | (_, Literal::Null) => Ok(Literal::Null),
        _ => Err(Error::Validation(
            "JSON contains requires JSONB arguments".to_string(),
        )),
    }
}

/// Evaluate JSON key exists: `obj ? 'key'`
pub(super) fn eval_json_key_exists(
    object: &TypedExpr,
    key: &TypedExpr,
    row: &Row,
) -> Result<Literal, Error> {
    let obj_lit = super::eval_expr(object, row)?;
    let key_lit = super::eval_expr(key, row)?;

    match (obj_lit, key_lit) {
        (Literal::JsonB(obj), Literal::Text(key)) => {
            let exists = match &obj {
                serde_json::Value::Object(map) => map.contains_key(&key),
                serde_json::Value::Array(arr) => {
                    arr.iter().any(|v| v.as_str() == Some(key.as_str()))
                }
                _ => false,
            };
            Ok(Literal::Boolean(exists))
        }
        (Literal::Null, _) | (_, Literal::Null) => Ok(Literal::Null),
        _ => Err(Error::Validation(
            "JSON key exists (?) requires JSONB and TEXT key".to_string(),
        )),
    }
}

/// Evaluate JSON any key exists: `obj ?| array`
pub(super) fn eval_json_any_key_exists(
    object: &TypedExpr,
    keys: &TypedExpr,
    row: &Row,
) -> Result<Literal, Error> {
    let obj_lit = super::eval_expr(object, row)?;
    let keys_lit = super::eval_expr(keys, row)?;

    match (obj_lit, keys_lit) {
        (Literal::JsonB(obj), Literal::JsonB(keys_json)) => {
            let serde_json::Value::Array(keys_array) = keys_json else {
                return Err(Error::Validation(
                    "JSON any key exists (?|) requires an array of keys".to_string(),
                ));
            };

            match &obj {
                serde_json::Value::Object(map) => {
                    for key_val in keys_array {
                        if let serde_json::Value::String(key) = key_val {
                            if map.contains_key(&key) {
                                return Ok(Literal::Boolean(true));
                            }
                        }
                    }
                    Ok(Literal::Boolean(false))
                }
                serde_json::Value::Array(arr) => {
                    for key_val in keys_array {
                        if let serde_json::Value::String(key) = key_val {
                            if arr.iter().any(|v| v.as_str() == Some(key.as_str())) {
                                return Ok(Literal::Boolean(true));
                            }
                        }
                    }
                    Ok(Literal::Boolean(false))
                }
                _ => Ok(Literal::Boolean(false)),
            }
        }
        (Literal::Null, _) | (_, Literal::Null) => Ok(Literal::Null),
        _ => Err(Error::Validation(
            "JSON any key exists (?|) requires JSONB and TEXT[] array".to_string(),
        )),
    }
}

/// Evaluate JSON all keys exist: `obj ?& array`
pub(super) fn eval_json_all_key_exists(
    object: &TypedExpr,
    keys: &TypedExpr,
    row: &Row,
) -> Result<Literal, Error> {
    let obj_lit = super::eval_expr(object, row)?;
    let keys_lit = super::eval_expr(keys, row)?;

    match (obj_lit, keys_lit) {
        (Literal::JsonB(obj), Literal::JsonB(keys_json)) => {
            let serde_json::Value::Array(keys_array) = keys_json else {
                return Err(Error::Validation(
                    "JSON all keys exist (?&) requires an array of keys".to_string(),
                ));
            };

            match &obj {
                serde_json::Value::Object(map) => {
                    for key_val in keys_array {
                        if let serde_json::Value::String(key) = key_val {
                            if !map.contains_key(&key) {
                                return Ok(Literal::Boolean(false));
                            }
                        } else {
                            return Ok(Literal::Boolean(false));
                        }
                    }
                    Ok(Literal::Boolean(true))
                }
                serde_json::Value::Array(arr) => {
                    for key_val in keys_array {
                        if let serde_json::Value::String(key) = key_val {
                            if !arr.iter().any(|v| v.as_str() == Some(key.as_str())) {
                                return Ok(Literal::Boolean(false));
                            }
                        } else {
                            return Ok(Literal::Boolean(false));
                        }
                    }
                    Ok(Literal::Boolean(true))
                }
                _ => Ok(Literal::Boolean(false)),
            }
        }
        (Literal::Null, _) | (_, Literal::Null) => Ok(Literal::Null),
        _ => Err(Error::Validation(
            "JSON all keys exist (?&) requires JSONB and TEXT[] array".to_string(),
        )),
    }
}

/// Evaluate JSON extract path: `obj #> path`
pub(super) fn eval_json_extract_path(
    object: &TypedExpr,
    path: &TypedExpr,
    row: &Row,
) -> Result<Literal, Error> {
    let obj_lit = super::eval_expr(object, row)?;
    let path_lit = super::eval_expr(path, row)?;

    match (obj_lit, path_lit) {
        (Literal::JsonB(obj), Literal::JsonB(path_json)) => {
            if let serde_json::Value::Array(path_array) = path_json {
                let mut current = &obj;
                for path_elem in path_array {
                    match (current, path_elem) {
                        (serde_json::Value::Object(map), serde_json::Value::String(key)) => {
                            match map.get(&key) {
                                Some(val) => current = val,
                                None => return Ok(Literal::Null),
                            }
                        }
                        (serde_json::Value::Array(arr), serde_json::Value::Number(idx)) => {
                            if let Some(i) = idx.as_u64() {
                                match arr.get(i as usize) {
                                    Some(val) => current = val,
                                    None => return Ok(Literal::Null),
                                }
                            } else {
                                return Ok(Literal::Null);
                            }
                        }
                        _ => return Ok(Literal::Null),
                    }
                }
                Ok(Literal::JsonB(current.clone()))
            } else {
                Err(Error::Validation(
                    "JSON extract path (#>) requires an array path".to_string(),
                ))
            }
        }
        (Literal::Null, _) | (_, Literal::Null) => Ok(Literal::Null),
        _ => Err(Error::Validation(
            "JSON extract path (#>) requires JSONB arguments".to_string(),
        )),
    }
}

/// Evaluate JSON extract path text: `obj #>> path`
pub(super) fn eval_json_extract_path_text(
    object: &TypedExpr,
    path: &TypedExpr,
    row: &Row,
) -> Result<Literal, Error> {
    let obj_lit = super::eval_expr(object, row)?;
    let path_lit = super::eval_expr(path, row)?;

    match (obj_lit, path_lit) {
        (Literal::JsonB(obj), Literal::JsonB(path_json)) => {
            if let serde_json::Value::Array(path_array) = path_json {
                let mut current = &obj;
                for path_elem in path_array {
                    match (current, path_elem) {
                        (serde_json::Value::Object(map), serde_json::Value::String(key)) => {
                            match map.get(&key) {
                                Some(val) => current = val,
                                None => return Ok(Literal::Null),
                            }
                        }
                        (serde_json::Value::Array(arr), serde_json::Value::Number(idx)) => {
                            if let Some(i) = idx.as_u64() {
                                match arr.get(i as usize) {
                                    Some(val) => current = val,
                                    None => return Ok(Literal::Null),
                                }
                            } else {
                                return Ok(Literal::Null);
                            }
                        }
                        _ => return Ok(Literal::Null),
                    }
                }
                match current {
                    serde_json::Value::String(s) => Ok(Literal::Text(s.clone())),
                    serde_json::Value::Null => Ok(Literal::Null),
                    other => Ok(Literal::Text(other.to_string())),
                }
            } else {
                Err(Error::Validation(
                    "JSON extract path text (#>>) requires an array path".to_string(),
                ))
            }
        }
        (Literal::Null, _) | (_, Literal::Null) => Ok(Literal::Null),
        _ => Err(Error::Validation(
            "JSON extract path text (#>>) requires JSONB arguments".to_string(),
        )),
    }
}

/// Evaluate JSON remove: `obj - key`
pub(super) fn eval_json_remove(
    object: &TypedExpr,
    key: &TypedExpr,
    row: &Row,
) -> Result<Literal, Error> {
    let obj_lit = super::eval_expr(object, row)?;
    let key_lit = super::eval_expr(key, row)?;

    match (&obj_lit, &key_lit) {
        (Literal::Null, _) | (_, Literal::Null) => Ok(Literal::Null),
        (Literal::JsonB(obj), Literal::Text(key_str)) => {
            if let serde_json::Value::Object(map) = obj {
                let mut new_map = map.clone();
                new_map.remove(key_str);
                Ok(Literal::JsonB(serde_json::Value::Object(new_map)))
            } else {
                Ok(Literal::JsonB(obj.clone()))
            }
        }
        (Literal::JsonB(obj), Literal::Int(idx)) => {
            if let serde_json::Value::Array(arr) = obj {
                let mut new_arr = arr.clone();
                let idx_usize = *idx as usize;
                if *idx >= 0 && idx_usize < new_arr.len() {
                    new_arr.remove(idx_usize);
                }
                Ok(Literal::JsonB(serde_json::Value::Array(new_arr)))
            } else {
                Ok(Literal::JsonB(obj.clone()))
            }
        }
        (Literal::JsonB(obj), Literal::BigInt(idx)) => {
            if let serde_json::Value::Array(arr) = obj {
                let mut new_arr = arr.clone();
                let idx_usize = *idx as usize;
                if *idx >= 0 && idx_usize < new_arr.len() {
                    new_arr.remove(idx_usize);
                }
                Ok(Literal::JsonB(serde_json::Value::Array(new_arr)))
            } else {
                Ok(Literal::JsonB(obj.clone()))
            }
        }
        (Literal::JsonB(obj), Literal::JsonB(keys_json)) => {
            if let (serde_json::Value::Object(map), serde_json::Value::Array(keys_array)) =
                (obj, keys_json)
            {
                let mut new_map = map.clone();
                for key_val in keys_array {
                    if let serde_json::Value::String(key) = key_val {
                        new_map.remove(key);
                    }
                }
                Ok(Literal::JsonB(serde_json::Value::Object(new_map)))
            } else {
                Ok(Literal::JsonB(obj.clone()))
            }
        }
        _ => Err(Error::Validation(format!(
            "JSONB - operator requires JSONB on left and TEXT, INT, or JSONB (array) on right, got {:?} - {:?}",
            obj_lit, key_lit
        ))),
    }
}

/// Evaluate JSON remove at path: `obj #- path`
pub(super) fn eval_json_remove_at_path(
    object: &TypedExpr,
    path: &TypedExpr,
    row: &Row,
) -> Result<Literal, Error> {
    let obj_lit = super::eval_expr(object, row)?;
    let path_lit = super::eval_expr(path, row)?;

    match (&obj_lit, &path_lit) {
        (Literal::Null, _) | (_, Literal::Null) => Ok(Literal::Null),
        (Literal::JsonB(obj), Literal::JsonB(path_json)) => {
            if let serde_json::Value::Array(path_array) = path_json {
                if path_array.is_empty() {
                    return Ok(Literal::JsonB(obj.clone()));
                }
                let result = remove_at_path(obj.clone(), path_array, 0);
                Ok(Literal::JsonB(result))
            } else {
                Err(Error::Validation(
                    "JSON remove at path (#-) requires an array path".to_string(),
                ))
            }
        }
        _ => Err(Error::Validation(format!(
            "JSON remove at path (#-) requires JSONB arguments, got {:?} #- {:?}",
            obj_lit, path_lit
        ))),
    }
}

/// Evaluate JSON path match: `obj @@ jsonpath`
pub(super) fn eval_json_path_match(
    object: &TypedExpr,
    path: &TypedExpr,
    row: &Row,
) -> Result<Literal, Error> {
    let obj_lit = super::eval_expr(object, row)?;
    let path_lit = super::eval_expr(path, row)?;

    match (&obj_lit, &path_lit) {
        (Literal::Null, _) | (_, Literal::Null) => Ok(Literal::Null),
        (Literal::JsonB(obj), Literal::Text(path_str)) => {
            let jsonpath = match serde_json_path::JsonPath::parse(path_str) {
                Ok(p) => p,
                Err(_) => return Ok(Literal::Boolean(false)),
            };
            let result = jsonpath.query(obj);
            let has_matches = !result.all().is_empty();
            Ok(Literal::Boolean(has_matches))
        }
        _ => Err(Error::Validation(format!(
            "JSON path match (@@) requires JSONB and TEXT (JSONPath), got {:?} @@ {:?}",
            obj_lit, path_lit
        ))),
    }
}

/// Evaluate JSON path exists: `obj @? jsonpath`
pub(super) fn eval_json_path_exists(
    object: &TypedExpr,
    path: &TypedExpr,
    row: &Row,
) -> Result<Literal, Error> {
    let obj_lit = super::eval_expr(object, row)?;
    let path_lit = super::eval_expr(path, row)?;

    match (&obj_lit, &path_lit) {
        (Literal::Null, _) | (_, Literal::Null) => Ok(Literal::Null),
        (Literal::JsonB(obj), Literal::Text(path_str)) => {
            let jsonpath = match serde_json_path::JsonPath::parse(path_str) {
                Ok(p) => p,
                Err(_) => return Ok(Literal::Boolean(false)),
            };
            let result = jsonpath.query(obj);
            let has_matches = !result.all().is_empty();
            Ok(Literal::Boolean(has_matches))
        }
        _ => Err(Error::Validation(format!(
            "JSON path exists (@?) requires JSONB and TEXT (JSONPath), got {:?} @? {:?}",
            obj_lit, path_lit
        ))),
    }
}
