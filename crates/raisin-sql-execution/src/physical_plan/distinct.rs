//! Distinct Executor
//!
//! Implements hash-based deduplication for SELECT DISTINCT and DISTINCT ON.

use super::executor::{ExecutionContext, ExecutionError, Row, RowStream};
use super::operators::PhysicalPlan;
use futures::stream::{self, StreamExt};
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::Storage;
use std::collections::HashSet;

/// Convert a PropertyValue to a hashable string key for DISTINCT deduplication.
///
/// This function creates a canonical string representation that:
/// - Treats all NULL values as equal (SQL DISTINCT semantics)
/// - Performs deep comparison for nested objects/arrays
/// - Is deterministic (same value always produces same key)
fn property_value_to_hash_key(value: &PropertyValue) -> String {
    match value {
        PropertyValue::Null => "NULL".to_string(),
        PropertyValue::Boolean(b) => format!("BOOL:{}", b),
        PropertyValue::Integer(i) => format!("INT:{}", i),
        PropertyValue::Float(f) => {
            if f.is_nan() {
                "FLOAT:NaN".to_string()
            } else {
                format!("FLOAT:{}", f.to_bits())
            }
        }
        PropertyValue::String(s) => format!("STR:{}", s),
        PropertyValue::Date(d) => format!("DATE:{:?}", d),
        PropertyValue::Decimal(d) => format!("DEC:{}", d),
        PropertyValue::Reference(r) => format!("REF:{}:{}", r.id, r.path),
        PropertyValue::Url(u) => format!("URL:{}", u.url),
        PropertyValue::Resource(r) => {
            // Hash resource by its metadata storage key if available
            if let Some(meta) = &r.metadata {
                if let Some(key) = meta.get("storage_key") {
                    return format!("RES:{}", property_value_to_hash_key(key));
                }
            }
            format!("RES:{:?}", r)
        }
        PropertyValue::Composite(c) => format!("COMP:{:?}", c),
        PropertyValue::Element(e) => format!("ELEM:{:?}", e),
        PropertyValue::Vector(v) => format!("VEC:{:?}", v),
        PropertyValue::Geometry(g) => format!("GEOM:{:?}", g),
        PropertyValue::Array(arr) => {
            let elements: Vec<String> = arr.iter().map(property_value_to_hash_key).collect();
            format!("ARR:[{}]", elements.join(","))
        }
        PropertyValue::Object(obj) => {
            let mut entries: Vec<_> = obj
                .iter()
                .map(|(k, v)| format!("{}:{}", k, property_value_to_hash_key(v)))
                .collect();
            entries.sort();
            format!("OBJ:{{{}}}", entries.join(","))
        }
    }
}

/// Generate a hash key for an entire row (for basic DISTINCT)
fn row_to_hash_key(row: &Row) -> String {
    let column_hashes: Vec<String> = row
        .columns
        .iter()
        .map(|(col_name, value)| format!("{}={}", col_name, property_value_to_hash_key(value)))
        .collect();
    column_hashes.join("|")
}

/// Generate a hash key for specific columns (for DISTINCT ON)
fn row_columns_to_hash_key(row: &Row, columns: &[String]) -> String {
    let column_hashes: Vec<String> = columns
        .iter()
        .map(|col_name| {
            let value = row.columns.get(col_name).unwrap_or(&PropertyValue::Null);
            format!("{}={}", col_name, property_value_to_hash_key(value))
        })
        .collect();
    column_hashes.join("|")
}

/// Execute a distinct operation
pub async fn execute_distinct<
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let (input, on_columns) = match plan {
        PhysicalPlan::Distinct { input, on_columns } => (input, on_columns),
        _ => {
            return Err(ExecutionError::Backend(
                "Invalid plan passed to execute_distinct".to_string(),
            ))
        }
    };

    // Execute input plan
    let mut input_stream = super::executor::execute_plan(input.as_ref(), ctx).await?;

    // Track seen keys for deduplication
    let mut seen_keys: HashSet<String> = HashSet::new();
    let mut output_rows = Vec::new();

    let is_distinct_on = !on_columns.is_empty();

    while let Some(row_result) = input_stream.next().await {
        let row = row_result?;

        let hash_key = if is_distinct_on {
            row_columns_to_hash_key(&row, on_columns)
        } else {
            row_to_hash_key(&row)
        };

        if seen_keys.insert(hash_key) {
            output_rows.push(row);
        }
    }

    tracing::debug!(
        "Distinct: {} unique rows from input (on_columns={:?})",
        output_rows.len(),
        if is_distinct_on {
            on_columns.as_slice()
        } else {
            &[]
        }
    );

    Ok(Box::pin(stream::iter(output_rows.into_iter().map(Ok))))
}
