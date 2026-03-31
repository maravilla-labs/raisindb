//! Hash Join Executor
//!
//! Implements the hash join algorithm for equality joins with support for all join types.
//!
//! Algorithm:
//! 1. Build Phase: Materialize right side and build hash table
//!    - For each right row: hash(right_keys) -> row
//! 2. Probe Phase: For each left row:
//!    - Compute hash(left_keys)
//!    - Look up matching right rows in hash table
//!    - Output merged rows for matches
//! 3. For LEFT/FULL joins: output unmatched left rows
//! 4. For RIGHT/FULL joins: output unmatched right rows
//!
//! Complexity: O(n + m) where n = left rows, m = right rows
//! Memory: O(m) - right side is materialized in hash table
//!
//! Performance: 10-100x faster than NestedLoopJoin for large datasets

use super::eval::eval_expr;
use super::executor::{ExecutionContext, ExecutionError, Row, RowStream};
use super::operators::PhysicalPlan;
use futures::stream::{self, StreamExt};
use indexmap::IndexMap;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::analyzer::{JoinType, Literal, TypedExpr};
use raisin_storage::Storage;
use std::collections::HashMap;

/// Execute a hash join
pub async fn execute_hash_join<
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let (left, right, join_type, left_keys, right_keys) = match plan {
        PhysicalPlan::HashJoin {
            left,
            right,
            join_type,
            left_keys,
            right_keys,
        } => (left, right, join_type, left_keys, right_keys),
        _ => {
            return Err(ExecutionError::Backend(
                "Invalid plan passed to execute_hash_join".to_string(),
            ))
        }
    };

    // Execute both inputs
    let left_stream = super::executor::execute_plan(left.as_ref(), ctx).await?;
    let right_stream = super::executor::execute_plan(right.as_ref(), ctx).await?;

    // Build Phase: Materialize right side and build hash table
    let mut right_rows = Vec::new();
    let right_vec: Vec<_> = right_stream.collect().await;
    for row_result in right_vec {
        right_rows.push(row_result?);
    }

    tracing::debug!(
        "HashJoin: Built hash table with {} right rows, {} join key(s)",
        right_rows.len(),
        right_keys.len()
    );

    // Build hash table: hash(right_keys) -> Vec<row_index>
    // Use Vec<usize> to support multiple rows with same key (duplicate join keys)
    // We use String representation as the hash key for simplicity
    let mut hash_table: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, row) in right_rows.iter().enumerate() {
        let key_values = extract_key_values(row, right_keys)?;
        let key_string = values_to_hash_key(&key_values);
        hash_table.entry(key_string).or_default().push(idx);
    }

    // Probe Phase: Create output rows based on join type
    let output_rows = match join_type {
        JoinType::Inner => {
            execute_hash_inner_join(left_stream, &right_rows, &hash_table, left_keys).await?
        }
        JoinType::Left => {
            execute_hash_left_join(left_stream, &right_rows, &hash_table, left_keys).await?
        }
        JoinType::Right => {
            execute_hash_right_join(left_stream, &right_rows, &hash_table, left_keys).await?
        }
        JoinType::Full => {
            execute_hash_full_join(left_stream, &right_rows, &hash_table, left_keys).await?
        }
        JoinType::Cross => {
            // CROSS JOIN should use NestedLoopJoin, not HashJoin
            return Err(ExecutionError::Validation(
                "CROSS JOIN should use NestedLoopJoin, not HashJoin".to_string(),
            ));
        }
    };

    tracing::debug!("HashJoin: Produced {} output rows", output_rows.len());

    // Convert Vec<Row> to stream of Result<Row, ExecutionError>
    Ok(Box::pin(stream::iter(output_rows.into_iter().map(Ok))))
}

/// Execute INNER hash join
async fn execute_hash_inner_join(
    mut left_stream: RowStream,
    right_rows: &[Row],
    hash_table: &HashMap<String, Vec<usize>>,
    left_keys: &[TypedExpr],
) -> Result<Vec<Row>, ExecutionError> {
    let mut output = Vec::new();

    // Probe Phase: For each left row
    while let Some(row_result) = left_stream.next().await {
        let left_row = row_result?;

        // Extract join key values from left row
        let key_values = extract_key_values(&left_row, left_keys)?;
        let key_string = values_to_hash_key(&key_values);

        // Look up matching right rows in hash table
        if let Some(right_indices) = hash_table.get(&key_string) {
            // For each matching right row
            for &idx in right_indices {
                let right_row = &right_rows[idx];
                // Merge and output
                output.push(merge_rows(&left_row, right_row));
            }
        }
    }

    Ok(output)
}

/// Execute LEFT OUTER hash join
async fn execute_hash_left_join(
    mut left_stream: RowStream,
    right_rows: &[Row],
    hash_table: &HashMap<String, Vec<usize>>,
    left_keys: &[TypedExpr],
) -> Result<Vec<Row>, ExecutionError> {
    let mut output = Vec::new();

    // Probe Phase: For each left row
    while let Some(row_result) = left_stream.next().await {
        let left_row = row_result?;

        // Extract join key values from left row
        let key_values = extract_key_values(&left_row, left_keys)?;
        let key_string = values_to_hash_key(&key_values);

        // Look up matching right rows in hash table
        if let Some(right_indices) = hash_table.get(&key_string) {
            // For each matching right row
            for &idx in right_indices {
                let right_row = &right_rows[idx];
                // Merge and output
                output.push(merge_rows(&left_row, right_row));
            }
        } else {
            // No match: output left row only (no right columns)
            output.push(left_row);
        }
    }

    Ok(output)
}

/// Execute RIGHT OUTER hash join
async fn execute_hash_right_join(
    mut left_stream: RowStream,
    right_rows: &[Row],
    hash_table: &HashMap<String, Vec<usize>>,
    left_keys: &[TypedExpr],
) -> Result<Vec<Row>, ExecutionError> {
    let mut output = Vec::new();
    let mut matched_right = vec![false; right_rows.len()];

    // Probe Phase: For each left row
    while let Some(row_result) = left_stream.next().await {
        let left_row = row_result?;

        // Extract join key values from left row
        let key_values = extract_key_values(&left_row, left_keys)?;
        let key_string = values_to_hash_key(&key_values);

        // Look up matching right rows in hash table
        if let Some(right_indices) = hash_table.get(&key_string) {
            // For each matching right row
            for &idx in right_indices {
                let right_row = &right_rows[idx];
                // Merge and output
                output.push(merge_rows(&left_row, right_row));
                matched_right[idx] = true;
            }
        }
    }

    // Output unmatched right rows (no left columns)
    for (idx, right_row) in right_rows.iter().enumerate() {
        if !matched_right[idx] {
            output.push(right_row.clone());
        }
    }

    Ok(output)
}

/// Execute FULL OUTER hash join
async fn execute_hash_full_join(
    mut left_stream: RowStream,
    right_rows: &[Row],
    hash_table: &HashMap<String, Vec<usize>>,
    left_keys: &[TypedExpr],
) -> Result<Vec<Row>, ExecutionError> {
    let mut output = Vec::new();
    let mut matched_right = vec![false; right_rows.len()];

    // Probe Phase: For each left row
    while let Some(row_result) = left_stream.next().await {
        let left_row = row_result?;

        // Extract join key values from left row
        let key_values = extract_key_values(&left_row, left_keys)?;
        let key_string = values_to_hash_key(&key_values);

        // Look up matching right rows in hash table
        if let Some(right_indices) = hash_table.get(&key_string) {
            // For each matching right row
            for &idx in right_indices {
                let right_row = &right_rows[idx];
                // Merge and output
                output.push(merge_rows(&left_row, right_row));
                matched_right[idx] = true;
            }
        } else {
            // No match: output left row only
            output.push(left_row);
        }
    }

    // Output unmatched right rows (no left columns)
    for (idx, right_row) in right_rows.iter().enumerate() {
        if !matched_right[idx] {
            output.push(right_row.clone());
        }
    }

    Ok(output)
}

/// Extract key values from a row for hashing
///
/// Evaluates the join key expressions on the row and returns their values.
/// These values are used as the hash table key.
fn extract_key_values(
    row: &Row,
    key_exprs: &[TypedExpr],
) -> Result<Vec<PropertyValue>, ExecutionError> {
    let mut values = Vec::with_capacity(key_exprs.len());

    for expr in key_exprs {
        let value = eval_expr(expr, row)?;
        // Convert Literal to PropertyValue for hashing
        let prop_value = literal_to_property_value(&value);
        values.push(prop_value);
    }

    Ok(values)
}

/// Convert values to a hash key string
///
/// Creates a string representation of the join key values for hashing.
/// This allows us to use a simple HashMap<String, ...> without requiring
/// PropertyValue to implement Hash and Eq.
fn values_to_hash_key(values: &[PropertyValue]) -> String {
    values
        .iter()
        .map(|v| format!("{:?}", v))
        .collect::<Vec<_>>()
        .join("\x00") // Use null byte as separator
}

/// Convert a Literal to PropertyValue
fn literal_to_property_value(lit: &Literal) -> PropertyValue {
    match lit {
        Literal::Text(s) => PropertyValue::String(s.clone()),
        Literal::Int(i) => PropertyValue::Integer(*i as i64),
        Literal::BigInt(i) => PropertyValue::Integer(*i),
        Literal::Double(d) => PropertyValue::Float(*d),
        Literal::Boolean(b) => PropertyValue::Boolean(*b),
        Literal::Null => PropertyValue::Null,
        Literal::JsonB(j) => PropertyValue::Object(
            j.as_object()
                .map(|obj| {
                    obj.iter()
                        .map(|(k, v)| (k.clone(), json_to_property_value(v)))
                        .collect()
                })
                .unwrap_or_default(),
        ),
        _ => PropertyValue::String(format!("{:?}", lit)),
    }
}

/// Convert JSON value to PropertyValue
fn json_to_property_value(value: &serde_json::Value) -> PropertyValue {
    match value {
        serde_json::Value::Null => PropertyValue::Null,
        serde_json::Value::Bool(b) => PropertyValue::Boolean(*b),
        serde_json::Value::Number(n) => {
            // Prefer integer if the number is a valid i64
            if let Some(i) = n.as_i64() {
                PropertyValue::Integer(i)
            } else {
                PropertyValue::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::String(s) => PropertyValue::String(s.clone()),
        serde_json::Value::Array(arr) => {
            PropertyValue::Array(arr.iter().map(json_to_property_value).collect())
        }
        serde_json::Value::Object(obj) => PropertyValue::Object(
            obj.iter()
                .map(|(k, v)| (k.clone(), json_to_property_value(v)))
                .collect(),
        ),
    }
}

/// Merge two rows into one
fn merge_rows(left: &Row, right: &Row) -> Row {
    let mut merged = IndexMap::new();

    // Add all left columns
    for (k, v) in &left.columns {
        merged.insert(k.clone(), v.clone());
    }

    // Add all right columns (may overwrite if same column name)
    for (k, v) in &right.columns {
        merged.insert(k.clone(), v.clone());
    }

    Row::from_map(merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_literal_to_property_value() {
        assert_eq!(
            literal_to_property_value(&Literal::Text("hello".to_string())),
            PropertyValue::String("hello".to_string())
        );
        assert_eq!(
            literal_to_property_value(&Literal::Int(42)),
            PropertyValue::Integer(42)
        );
        assert_eq!(
            literal_to_property_value(&Literal::Boolean(true)),
            PropertyValue::Boolean(true)
        );
        assert_eq!(
            literal_to_property_value(&Literal::Null),
            PropertyValue::Null
        );
    }

    #[test]
    fn test_values_to_hash_key() {
        let values = vec![
            PropertyValue::String("test".to_string()),
            PropertyValue::Integer(42),
        ];
        let key = values_to_hash_key(&values);
        // Should create a deterministic string representation
        assert!(!key.is_empty());
        assert!(key.contains("test"));
    }

    #[test]
    fn test_merge_rows() {
        let mut left_cols = IndexMap::new();
        left_cols.insert("id".to_string(), PropertyValue::Integer(1));
        left_cols.insert(
            "name".to_string(),
            PropertyValue::String("Alice".to_string()),
        );
        let left = Row::from_map(left_cols);

        let mut right_cols = IndexMap::new();
        right_cols.insert("user_id".to_string(), PropertyValue::Integer(1));
        right_cols.insert("city".to_string(), PropertyValue::String("NYC".to_string()));
        let right = Row::from_map(right_cols);

        let merged = merge_rows(&left, &right);

        assert_eq!(merged.columns.len(), 4);
        assert_eq!(merged.get("id"), Some(&PropertyValue::Integer(1)));
        assert_eq!(
            merged.get("name"),
            Some(&PropertyValue::String("Alice".to_string()))
        );
        assert_eq!(merged.get("user_id"), Some(&PropertyValue::Integer(1)));
        assert_eq!(
            merged.get("city"),
            Some(&PropertyValue::String("NYC".to_string()))
        );
    }
}
