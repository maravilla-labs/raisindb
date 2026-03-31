//! Integration Tests for Batch Execution
//!
//! Tests batch-aware execution components.

use super::*;
use crate::physical_plan::batch::BatchConfig;
use crate::physical_plan::executor::Row;
use futures::StreamExt;
use indexmap::indexmap;
use raisin_models::nodes::properties::PropertyValue;
use std::collections::HashMap;

/// Helper to create a test row
fn create_test_row(id: i32) -> Row {
    Row::from_map(indexmap! {
        "id".to_string() => PropertyValue::Integer(id as i64),
        "name".to_string() => PropertyValue::String(format!("name_{}", id)),
    })
}

#[tokio::test]
async fn test_accumulator_basic() {
    let mut acc = RowAccumulator::new(3);

    // Add rows until batch is full
    assert!(acc.add_row(create_test_row(1)).is_none());
    assert!(acc.add_row(create_test_row(2)).is_none());

    // Third row completes the batch
    let batch = acc.add_row(create_test_row(3)).unwrap();
    assert_eq!(batch.num_rows(), 3);
}

#[tokio::test]
async fn test_accumulator_flush() {
    let mut acc = RowAccumulator::new(10);

    // Add fewer rows than batch size
    acc.add_row(create_test_row(1));
    acc.add_row(create_test_row(2));
    acc.add_row(create_test_row(3));

    // Flush should return partial batch
    let batch = acc.flush().unwrap();
    assert_eq!(batch.num_rows(), 3);
}

#[tokio::test]
async fn test_conversion_functions() {
    use async_stream::try_stream;

    // Create a test row stream
    let row_stream = Box::pin(try_stream! {
        for i in 0..5 {
            yield create_test_row(i);
        }
    });

    let batch_config = BatchConfig::new(2);
    let mut batch_stream = convert_row_stream_to_batch_stream(row_stream, &batch_config);

    // Should get 3 batches: [2, 2, 1]
    let mut batch_count = 0;
    let mut total_rows = 0;

    while let Some(result) = batch_stream.next().await {
        let batch = result.unwrap();
        total_rows += batch.num_rows();
        batch_count += 1;
    }

    assert_eq!(batch_count, 3);
    assert_eq!(total_rows, 5);
}

#[tokio::test]
async fn test_batch_to_row_conversion() {
    use async_stream::try_stream;

    // Create test row stream
    let row_stream = Box::pin(try_stream! {
        for i in 0..10 {
            yield create_test_row(i);
        }
    });

    let batch_config = BatchConfig::new(4);

    // Convert to batches
    let batch_stream = convert_row_stream_to_batch_stream(row_stream, &batch_config);

    // Convert back to rows
    let mut row_stream_back = convert_batch_stream_to_row_stream(batch_stream);

    // Verify all rows are preserved
    let mut row_count = 0;
    while let Some(result) = row_stream_back.next().await {
        result.unwrap();
        row_count += 1;
    }

    assert_eq!(row_count, 10);
}

#[tokio::test]
async fn test_batch_execution_config() {
    // Test default config
    let config = BatchExecutionConfig::default();
    assert!(config.should_use_batch_for_table_scan(Some(1000)));
    assert!(!config.should_use_batch_for_table_scan(Some(50)));

    // Test always batch
    let config = BatchExecutionConfig::always_batch();
    assert!(config.should_use_batch_for_table_scan(Some(1)));

    // Test never batch
    let config = BatchExecutionConfig::never_batch();
    assert!(!config.should_use_batch_for_table_scan(Some(1_000_000)));

    // Test low latency
    let config = BatchExecutionConfig::low_latency();
    assert!(config.should_use_batch_for_table_scan(Some(2000)));
    assert!(!config.should_use_batch_for_index_scan(Some(2000)));

    // Test high throughput
    let config = BatchExecutionConfig::high_throughput();
    assert!(config.should_use_batch_for_table_scan(Some(20)));
    assert!(config.should_use_batch_for_index_scan(Some(20)));
}

// ============================================================================
// Batch Projection Tests
// ============================================================================

#[tokio::test]
async fn test_project_basic_column_passthrough() {
    use crate::physical_plan::batch::Batch;
    use raisin_sql::analyzer::{DataType, TypedExpr};
    use raisin_sql::logical_plan::ProjectionExpr;

    // Create test batch with simple columns
    let rows = vec![
        Row::from_map(indexmap! {
            "table1.id".to_string() => PropertyValue::Integer(1),
            "table1.name".to_string() => PropertyValue::String("Alice".to_string()),
        }),
        Row::from_map(indexmap! {
            "table1.id".to_string() => PropertyValue::Integer(2),
            "table1.name".to_string() => PropertyValue::String("Bob".to_string()),
        }),
    ];
    let batch = Batch::from(rows);

    // Test batch structure
    assert_eq!(batch.num_rows(), 2);
    assert_eq!(batch.num_columns(), 2);
    assert!(batch.contains_column("table1.id"));
    assert!(batch.contains_column("table1.name"));

    // Verify column access works correctly
    use crate::physical_plan::batch::ColumnArray;
    if let Some(ColumnArray::Integer(ids)) = batch.column("table1.id") {
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], Some(1));
        assert_eq!(ids[1], Some(2));
    } else {
        panic!("Expected Integer column for id");
    }

    if let Some(ColumnArray::String(names)) = batch.column("table1.name") {
        assert_eq!(names.len(), 2);
        assert_eq!(names[0], Some("Alice".to_string()));
        assert_eq!(names[1], Some("Bob".to_string()));
    } else {
        panic!("Expected String column for name");
    }
}

#[tokio::test]
async fn test_project_json_extraction_columnar() {
    use crate::physical_plan::batch::{Batch, ColumnArray};
    use raisin_sql::analyzer::{DataType, Expr, Literal, TypedExpr};
    use raisin_sql::logical_plan::ProjectionExpr;
    use std::collections::HashMap;

    // Create test batch with JSON objects
    let mut properties1 = HashMap::new();
    properties1.insert(
        "username".to_string(),
        PropertyValue::String("alice".to_string()),
    );
    properties1.insert("age".to_string(), PropertyValue::Float(30.0));

    let mut properties2 = HashMap::new();
    properties2.insert(
        "username".to_string(),
        PropertyValue::String("bob".to_string()),
    );
    properties2.insert("age".to_string(), PropertyValue::Float(25.0));

    let rows = vec![
        Row::from_map(indexmap! {
            "user.properties".to_string() => PropertyValue::Object(properties1),
        }),
        Row::from_map(indexmap! {
            "user.properties".to_string() => PropertyValue::Object(properties2),
        }),
    ];
    let batch = Batch::from(rows);

    // Test that batch has correct structure
    assert_eq!(batch.num_rows(), 2);
    assert_eq!(batch.num_columns(), 1);
    assert!(batch.contains_column("user.properties"));

    // Verify we can extract the Object column
    if let Some(ColumnArray::Object(objects)) = batch.column("user.properties") {
        assert_eq!(objects.len(), 2);
        assert!(objects[0].is_some());
        assert!(objects[1].is_some());
    } else {
        panic!("Expected Object column");
    }
}

#[tokio::test]
async fn test_broadcast_literal() {
    use crate::physical_plan::batch::ColumnArray;
    use crate::physical_plan::batch_execution::project::broadcast_literal;
    use raisin_sql::analyzer::Literal;

    // Test string literal
    let lit = Literal::Text("hello".to_string());
    let col = broadcast_literal(&lit, 3);
    if let ColumnArray::String(values) = col {
        assert_eq!(values.len(), 3);
        assert_eq!(values[0], Some("hello".to_string()));
        assert_eq!(values[1], Some("hello".to_string()));
        assert_eq!(values[2], Some("hello".to_string()));
    } else {
        panic!("Expected String column");
    }

    // Test number literal
    let lit = Literal::Double(42.5);
    let col = broadcast_literal(&lit, 5);
    if let ColumnArray::Float(values) = col {
        assert_eq!(values.len(), 5);
        assert!(values.iter().all(|v| *v == Some(42.5)));
    } else {
        panic!("Expected Float column");
    }

    // Test boolean literal
    let lit = Literal::Boolean(true);
    let col = broadcast_literal(&lit, 2);
    if let ColumnArray::Boolean(values) = col {
        assert_eq!(values.len(), 2);
        assert!(values.iter().all(|v| *v == Some(true)));
    } else {
        panic!("Expected Boolean column");
    }
}

#[tokio::test]
async fn test_extract_text_from_property_value() {
    use crate::physical_plan::batch_execution::project::extract_text_from_property_value;

    // String extraction
    assert_eq!(
        extract_text_from_property_value(&PropertyValue::String("test".to_string())),
        Some("test".to_string())
    );

    // Integer extraction
    assert_eq!(
        extract_text_from_property_value(&PropertyValue::Integer(42)),
        Some("42".to_string())
    );

    // Float extraction
    assert_eq!(
        extract_text_from_property_value(&PropertyValue::Float(42.5)),
        Some("42.5".to_string())
    );

    // Boolean extraction
    assert_eq!(
        extract_text_from_property_value(&PropertyValue::Boolean(true)),
        Some("true".to_string())
    );

    // Complex types return JSON
    let mut obj = HashMap::new();
    obj.insert(
        "key".to_string(),
        PropertyValue::String("value".to_string()),
    );
    let result = extract_text_from_property_value(&PropertyValue::Object(obj));
    assert!(result.is_some());
    assert!(result.unwrap().contains("key"));
}

#[tokio::test]
async fn test_batch_from_columns() {
    use crate::physical_plan::batch::{Batch, ColumnArray};
    use indexmap::IndexMap;

    let mut columns = IndexMap::new();
    columns.insert(
        "name".to_string(),
        ColumnArray::String(vec![
            Some("Alice".to_string()),
            Some("Bob".to_string()),
            Some("Charlie".to_string()),
        ]),
    );
    columns.insert(
        "age".to_string(),
        ColumnArray::Float(vec![Some(30.0), Some(25.0), Some(35.0)]),
    );

    let batch = Batch::from_columns(columns);

    assert_eq!(batch.num_rows(), 3);
    assert_eq!(batch.num_columns(), 2);
    assert!(batch.contains_column("name"));
    assert!(batch.contains_column("age"));

    // Verify we can iterate over rows
    let rows: Vec<_> = batch.iter().collect();
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows[0].get("name"),
        Some(&PropertyValue::String("Alice".to_string()))
    );
    assert_eq!(rows[0].get("age"), Some(&PropertyValue::Float(30.0)));
}

#[test]
fn test_property_values_to_column_array() {
    use crate::physical_plan::batch::ColumnArray;
    use crate::physical_plan::batch_execution::project::property_values_to_column_array;

    // Test string column
    let values = vec![
        Some(PropertyValue::String("a".to_string())),
        Some(PropertyValue::String("b".to_string())),
        None,
    ];
    let result = property_values_to_column_array(values).unwrap();
    if let ColumnArray::String(strings) = result {
        assert_eq!(strings.len(), 3);
        assert_eq!(strings[0], Some("a".to_string()));
        assert_eq!(strings[1], Some("b".to_string()));
        assert_eq!(strings[2], None);
    } else {
        panic!("Expected String column");
    }

    // Test integer column
    let values = vec![
        Some(PropertyValue::Integer(1)),
        Some(PropertyValue::Integer(2)),
        None,
    ];
    let result = property_values_to_column_array(values).unwrap();
    if let ColumnArray::Integer(integers) = result {
        assert_eq!(integers.len(), 3);
        assert_eq!(integers[0], Some(1));
        assert_eq!(integers[1], Some(2));
        assert_eq!(integers[2], None);
    } else {
        panic!("Expected Integer column");
    }

    // Test float column
    let values = vec![
        Some(PropertyValue::Float(1.5)),
        Some(PropertyValue::Float(2.5)),
        None,
    ];
    let result = property_values_to_column_array(values).unwrap();
    if let ColumnArray::Float(floats) = result {
        assert_eq!(floats.len(), 3);
        assert_eq!(floats[0], Some(1.5));
        assert_eq!(floats[1], Some(2.5));
        assert_eq!(floats[2], None);
    } else {
        panic!("Expected Float column");
    }
}
