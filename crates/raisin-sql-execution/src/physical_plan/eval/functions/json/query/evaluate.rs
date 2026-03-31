//! Core evaluation logic for JSON_QUERY
//!
//! Contains `JsonQueryFunction`, its `SqlFunction` trait implementation,
//! the JSONPath cache, and helper methods for ON EMPTY / ON ERROR handling.

use super::clauses::{OnEmptyBehavior, OnErrorBehavior, WrapperClause};
use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Expr, Literal, TypedExpr};
use serde_json_path::JsonPath;
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

/// Global cache for compiled JSONPath expressions
///
/// Shared with JSON_VALUE for consistency. Since JSONPath strings are typically
/// constant literals in queries, we compile them once and reuse across evaluations.
static JSONPATH_CACHE: OnceLock<RwLock<HashMap<String, JsonPath>>> = OnceLock::new();

/// Get or initialize the JSONPath cache
fn get_jsonpath_cache() -> &'static RwLock<HashMap<String, JsonPath>> {
    JSONPATH_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Extract a JSON object or array from JSON using JSONPath
///
/// # SQL Signature
/// `JSON_QUERY(jsonb, path [, wrapper_clause] [, on_empty] [, on_error]) -> JSONB`
///
/// # Arguments
/// * `jsonb` - JSONB value to query
/// * `path` - JSONPath expression as TEXT
/// * `wrapper_clause` - Optional TEXT: 'WITH WRAPPER', 'WITHOUT WRAPPER', 'WITH CONDITIONAL WRAPPER'
/// * `on_empty` - Optional TEXT: 'NULL', 'ERROR', 'EMPTY ARRAY', 'EMPTY OBJECT' (Phase 3)
/// * `on_error` - Optional TEXT: 'NULL', 'ERROR', 'EMPTY ARRAY', 'EMPTY OBJECT' (Phase 3)
///
/// # Returns
/// * JSON object or array as JSONB
/// * Behavior on empty result controlled by on_empty parameter
/// * Behavior on error controlled by on_error parameter
/// * NULL if path matches a scalar value (use JSON_VALUE for scalars)
/// * Multiple matches handled based on wrapper clause
///
/// # Examples
/// ```sql
/// -- Basic extraction (Phase 1)
/// SELECT JSON_QUERY('{"user": {"name": "Alice", "age": 30}}', '$.user')
/// -- Returns: '{"name":"Alice","age":30}'
///
/// SELECT JSON_QUERY('{"tags": ["rust", "sql", "json"]}', '$.tags')
/// -- Returns: '["rust","sql","json"]'
///
/// -- With wrapper clause (Phase 2)
/// SELECT JSON_QUERY('[{"id": 1}, {"id": 2}]', '$[*]', 'WITH WRAPPER')
/// -- Returns: '[{"id":1},{"id":2}]' (wrapped in array)
///
/// SELECT JSON_QUERY('[{"id": 1}]', '$[*]', 'WITH CONDITIONAL WRAPPER')
/// -- Returns: '{"id":1}' (single match, no wrapping)
///
/// SELECT JSON_QUERY('[{"id": 1}, {"id": 2}]', '$[*]', 'WITH CONDITIONAL WRAPPER')
/// -- Returns: '[{"id":1},{"id":2}]' (multiple matches, wrapped)
///
/// SELECT JSON_QUERY('[{"id": 1}, {"id": 2}]', '$[*]', 'WITHOUT WRAPPER')
/// -- Returns: NULL (multiple matches without wrapper)
///
/// -- Error/empty handling (Phase 3)
/// SELECT JSON_QUERY('{}', '$.missing', 'WITHOUT WRAPPER', 'EMPTY ARRAY')
/// -- Returns: [] (empty array on missing path)
///
/// SELECT JSON_QUERY('{}', '$.missing', 'WITHOUT WRAPPER', 'ERROR')
/// -- Returns: Error (raises error on missing path)
///
/// SELECT JSON_QUERY(data, '$.path', 'WITH WRAPPER', 'NULL', 'EMPTY OBJECT')
/// -- Returns: {} if JSONPath parsing fails, NULL if path doesn't exist
/// ```
///
/// # Notes
/// - Follows SQL:2016 standard behavior
/// - Only returns objects/arrays (not scalar values)
/// - Use JSON_VALUE for extracting scalar values
/// - Phase 1: Basic 2-argument implementation
/// - Phase 2: Wrapper clause support (3-argument form)
/// - Phase 3: ON EMPTY and ON ERROR handling (4-5 argument forms)
/// - Future: RETURNING clause
pub struct JsonQueryFunction;

impl SqlFunction for JsonQueryFunction {
    fn name(&self) -> &str {
        "JSON_QUERY"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Json
    }

    fn signature(&self) -> &str {
        "JSON_QUERY(jsonb, path) -> JSONB"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        // Validate argument count (2-5 arguments)
        if args.len() < 2 || args.len() > 5 {
            return Err(Error::Validation(
                "JSON_QUERY requires 2-5 arguments (jsonb, path [, wrapper] [, on_empty] [, on_error])".to_string(),
            ));
        }

        let json_val = eval_expr(&args[0], row)?;
        let path_val = eval_expr(&args[1], row)?;

        // Parse optional wrapper clause (3rd argument)
        let wrapper_clause = if args.len() >= 3 {
            let wrapper_val = eval_expr(&args[2], row)?;

            // Handle NULL wrapper clause -> use default (WITHOUT WRAPPER)
            if matches!(wrapper_val, Literal::Null) {
                WrapperClause::Without
            } else {
                let Literal::Text(wrapper_str) = wrapper_val else {
                    return Err(Error::Validation(
                        "JSON_QUERY third argument (wrapper clause) must be TEXT".to_string(),
                    ));
                };
                WrapperClause::from_str(&wrapper_str)?
            }
        } else {
            // Default: WITHOUT WRAPPER (SQL:2016 standard default)
            WrapperClause::Without
        };

        // Parse optional ON EMPTY clause (4th argument) - Phase 3
        let on_empty = if args.len() >= 4 {
            let on_empty_val = eval_expr(&args[3], row)?;

            if matches!(on_empty_val, Literal::Null) {
                OnEmptyBehavior::Null
            } else {
                let Literal::Text(on_empty_str) = on_empty_val else {
                    return Err(Error::Validation(
                        "JSON_QUERY fourth argument (ON EMPTY) must be TEXT".to_string(),
                    ));
                };
                OnEmptyBehavior::from_str(&on_empty_str)?
            }
        } else {
            // Default: NULL ON EMPTY (SQL:2016 standard default)
            OnEmptyBehavior::Null
        };

        // Parse optional ON ERROR clause (5th argument) - Phase 3
        let on_error = if args.len() >= 5 {
            let on_error_val = eval_expr(&args[4], row)?;

            if matches!(on_error_val, Literal::Null) {
                OnErrorBehavior::Null
            } else {
                let Literal::Text(on_error_str) = on_error_val else {
                    return Err(Error::Validation(
                        "JSON_QUERY fifth argument (ON ERROR) must be TEXT".to_string(),
                    ));
                };
                OnErrorBehavior::from_str(&on_error_str)?
            }
        } else {
            // Default: NULL ON ERROR (SQL:2016 standard default)
            OnErrorBehavior::Null
        };

        // Handle NULL propagation for json and path
        if matches!(json_val, Literal::Null) || matches!(path_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        // Validate types
        let Literal::JsonB(json) = json_val else {
            return Err(Error::Validation(
                "JSON_QUERY first argument must be JSONB".to_string(),
            ));
        };

        let Literal::Text(path_str) = path_val else {
            return Err(Error::Validation(
                "JSON_QUERY second argument must be TEXT (JSONPath)".to_string(),
            ));
        };

        // Compile or retrieve cached JSONPath
        // Wrap in error handling that respects ON ERROR behavior
        let path = if matches!(&args[1].expr, Expr::Literal(_)) {
            // Path is a constant literal - use cache
            let cache_lock = get_jsonpath_cache();
            {
                let cache = cache_lock.read().unwrap();
                if let Some(cached_path) = cache.get(&path_str) {
                    // Cache hit - clone the compiled path
                    cached_path.clone()
                } else {
                    // Cache miss - compile and store
                    drop(cache); // Release read lock before acquiring write lock

                    let compiled = match JsonPath::parse(&path_str) {
                        Ok(p) => p,
                        Err(e) => {
                            // JSONPath parsing error - handle based on ON ERROR
                            return on_error
                                .apply(format!("Invalid JSONPath '{}': {}", path_str, e));
                        }
                    };

                    cache_lock
                        .write()
                        .unwrap()
                        .insert(path_str.clone(), compiled.clone());

                    compiled
                }
            }
        } else {
            // Path is dynamic (rare case) - parse each time
            match JsonPath::parse(&path_str) {
                Ok(p) => p,
                Err(e) => {
                    return on_error.apply(format!("Invalid JSONPath '{}': {}", path_str, e));
                }
            }
        };

        // Query the JSON
        let result = path.query(&json);
        let matches = result.all();

        match matches.len() {
            0 => {
                // No matches - handle based on ON EMPTY behavior
                on_empty.apply()
            }
            1 => {
                // Single match
                let value = matches[0];

                // Filter out scalars (JSON_QUERY only returns objects/arrays)
                let is_scalar = !matches!(
                    value,
                    serde_json::Value::Object(_) | serde_json::Value::Array(_)
                );
                if is_scalar && !matches!(value, serde_json::Value::Null) {
                    return Ok(Literal::Null);
                }

                // Handle wrapper clause for single match
                match wrapper_clause {
                    WrapperClause::With => {
                        // WITH WRAPPER: Always wrap in array, even single match
                        Ok(Literal::JsonB(serde_json::Value::Array(
                            vec![value.clone()],
                        )))
                    }
                    WrapperClause::Without | WrapperClause::Conditional => {
                        // WITHOUT WRAPPER / CONDITIONAL: Return value as-is for single match
                        if matches!(value, serde_json::Value::Null) {
                            Ok(Literal::Null)
                        } else {
                            Ok(Literal::JsonB(value.clone()))
                        }
                    }
                }
            }
            _ => {
                // Multiple matches - behavior depends on wrapper clause
                match wrapper_clause {
                    WrapperClause::Without => {
                        // WITHOUT WRAPPER: Return NULL for multiple matches (SQL:2016 default)
                        Ok(Literal::Null)
                    }
                    WrapperClause::With | WrapperClause::Conditional => {
                        // WITH WRAPPER / CONDITIONAL: Wrap all matches in an array
                        // Filter out scalar values (JSON_QUERY only returns objects/arrays)
                        let filtered_matches: Vec<serde_json::Value> = matches
                            .iter()
                            .filter(|v| {
                                matches!(
                                    v,
                                    serde_json::Value::Object(_)
                                        | serde_json::Value::Array(_)
                                        | serde_json::Value::Null
                                )
                            })
                            .map(|v| (*v).clone())
                            .collect();

                        if filtered_matches.is_empty() {
                            // All matches were scalars
                            Ok(Literal::Null)
                        } else {
                            Ok(Literal::JsonB(serde_json::Value::Array(filtered_matches)))
                        }
                    }
                }
            }
        }
    }
}
