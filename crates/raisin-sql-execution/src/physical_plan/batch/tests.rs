//! Tests for the batch module.

use super::*;
use crate::physical_plan::executor::Row;
use indexmap::IndexMap;
use raisin_models::nodes::properties::PropertyValue;

fn create_test_row(id: &str, age: f64) -> Row {
    let mut columns = IndexMap::new();
    columns.insert("id".to_string(), PropertyValue::String(id.to_string()));
    columns.insert("age".to_string(), PropertyValue::Float(age));
    Row::from_map(columns)
}

#[test]
fn test_empty_batch() {
    let batch = Batch::new();
    assert_eq!(batch.num_rows(), 0);
    assert_eq!(batch.num_columns(), 0);
    assert!(batch.is_empty());
    assert_eq!(batch.schema(), Vec::<&str>::new());
}

#[test]
fn test_batch_from_empty_rows() {
    let rows: Vec<Row> = vec![];
    let batch = Batch::from(rows);
    assert_eq!(batch.num_rows(), 0);
    assert_eq!(batch.num_columns(), 0);
}

#[test]
fn test_single_column_batch() {
    let mut columns = IndexMap::new();
    columns.insert("id".to_string(), PropertyValue::String("1".to_string()));

    let rows = vec![
        Row::from_map(columns.clone()),
        Row::from_map({
            let mut c = IndexMap::new();
            c.insert("id".to_string(), PropertyValue::String("2".to_string()));
            c
        }),
    ];

    let batch = Batch::from(rows);
    assert_eq!(batch.num_rows(), 2);
    assert_eq!(batch.num_columns(), 1);
    assert_eq!(batch.schema(), vec!["id"]);

    // Check column data
    let id_column = batch.column("id").unwrap();
    assert_eq!(id_column.len(), 2);
    assert_eq!(
        id_column.get(0),
        Some(PropertyValue::String("1".to_string()))
    );
    assert_eq!(
        id_column.get(1),
        Some(PropertyValue::String("2".to_string()))
    );
}

#[test]
fn test_multi_column_batch() {
    let rows = vec![
        create_test_row("1", 25.0),
        create_test_row("2", 30.0),
        create_test_row("3", 35.0),
    ];

    let batch = Batch::from(rows);
    assert_eq!(batch.num_rows(), 3);
    assert_eq!(batch.num_columns(), 2);

    let schema = batch.schema();
    assert!(schema.contains(&"id"));
    assert!(schema.contains(&"age"));

    // Verify column data
    let id_column = batch.column("id").unwrap();
    assert_eq!(id_column.len(), 3);

    let age_column = batch.column("age").unwrap();
    assert_eq!(age_column.len(), 3);
    assert_eq!(age_column.get(1), Some(PropertyValue::Float(30.0)));
}

#[test]
fn test_row_batch_row_round_trip() {
    let original_rows = vec![
        create_test_row("1", 25.0),
        create_test_row("2", 30.0),
        create_test_row("3", 35.0),
    ];

    let batch = Batch::from(original_rows.clone());
    let reconstructed_rows = Vec::<Row>::from(batch);

    assert_eq!(reconstructed_rows.len(), original_rows.len());

    for (original, reconstructed) in original_rows.iter().zip(reconstructed_rows.iter()) {
        assert_eq!(original.columns, reconstructed.columns);
    }
}

#[test]
fn test_mixed_null_values() {
    let mut row1_cols = IndexMap::new();
    row1_cols.insert("id".to_string(), PropertyValue::String("1".to_string()));
    row1_cols.insert("age".to_string(), PropertyValue::Float(25.0));

    let mut row2_cols = IndexMap::new();
    row2_cols.insert("id".to_string(), PropertyValue::String("2".to_string()));
    // age is missing (NULL)

    let rows = vec![Row::from_map(row1_cols), Row::from_map(row2_cols)];

    let batch = Batch::from(rows);
    assert_eq!(batch.num_rows(), 2);
    assert_eq!(batch.num_columns(), 2);

    // Check that missing value is represented as None
    let age_column = batch.column("age").unwrap();
    assert_eq!(age_column.get(0), Some(PropertyValue::Float(25.0)));
    assert_eq!(age_column.get(1), None); // NULL value
}

#[test]
fn test_different_column_types() {
    let mut columns = IndexMap::new();
    columns.insert("str".to_string(), PropertyValue::String("test".to_string()));
    columns.insert("num".to_string(), PropertyValue::Float(42.0));
    columns.insert("bool".to_string(), PropertyValue::Boolean(true));
    columns.insert(
        "vec".to_string(),
        PropertyValue::Vector(vec![1.0, 2.0, 3.0]),
    );

    let rows = vec![Row::from_map(columns)];
    let batch = Batch::from(rows);

    assert_eq!(batch.num_columns(), 4);
    assert!(batch.contains_column("str"));
    assert!(batch.contains_column("num"));
    assert!(batch.contains_column("bool"));
    assert!(batch.contains_column("vec"));

    // Verify each column type
    let str_col = batch.column("str").unwrap();
    assert!(matches!(str_col, ColumnArray::String(_)));

    let num_col = batch.column("num").unwrap();
    assert!(matches!(num_col, ColumnArray::Float(_)));

    let bool_col = batch.column("bool").unwrap();
    assert!(matches!(bool_col, ColumnArray::Boolean(_)));

    let vec_col = batch.column("vec").unwrap();
    assert!(matches!(vec_col, ColumnArray::Vector(_)));
}

#[test]
fn test_large_batch() {
    // Create a batch with 1000 rows to test performance characteristics
    let rows: Vec<Row> = (0..1000)
        .map(|i| create_test_row(&format!("id_{}", i), i as f64))
        .collect();

    let batch = Batch::from(rows);
    assert_eq!(batch.num_rows(), 1000);
    assert_eq!(batch.num_columns(), 2);

    // Spot check some values
    let id_column = batch.column("id").unwrap();
    assert_eq!(
        id_column.get(0),
        Some(PropertyValue::String("id_0".to_string()))
    );
    assert_eq!(
        id_column.get(999),
        Some(PropertyValue::String("id_999".to_string()))
    );

    let age_column = batch.column("age").unwrap();
    assert_eq!(age_column.get(500), Some(PropertyValue::Float(500.0)));
}

#[test]
fn test_batch_row_access() {
    let rows = vec![
        create_test_row("1", 25.0),
        create_test_row("2", 30.0),
        create_test_row("3", 35.0),
    ];

    let batch = Batch::from(rows.clone());

    // Test row access
    let row0 = batch.row(0).unwrap();
    assert_eq!(row0.columns, rows[0].columns);

    let row2 = batch.row(2).unwrap();
    assert_eq!(row2.columns, rows[2].columns);

    // Out of bounds
    assert!(batch.row(3).is_none());
    assert!(batch.row(100).is_none());
}

#[test]
fn test_batch_iterator() {
    let rows = vec![
        create_test_row("1", 25.0),
        create_test_row("2", 30.0),
        create_test_row("3", 35.0),
    ];

    let batch = Batch::from(rows.clone());
    let collected: Vec<Row> = batch.iter().collect();

    assert_eq!(collected.len(), 3);
    for (original, iterated) in rows.iter().zip(collected.iter()) {
        assert_eq!(original.columns, iterated.columns);
    }
}

#[test]
fn test_batch_iterator_size_hint() {
    let rows = vec![
        create_test_row("1", 25.0),
        create_test_row("2", 30.0),
        create_test_row("3", 35.0),
    ];

    let batch = Batch::from(rows);
    let mut iter = batch.iter();

    assert_eq!(iter.size_hint(), (3, Some(3)));
    assert_eq!(iter.len(), 3);

    iter.next();
    assert_eq!(iter.size_hint(), (2, Some(2)));
    assert_eq!(iter.len(), 2);

    iter.next();
    iter.next();
    assert_eq!(iter.size_hint(), (0, Some(0)));
    assert_eq!(iter.len(), 0);

    assert!(iter.next().is_none());
}

#[test]
fn test_batch_config() {
    let default_config = BatchConfig::default();
    assert_eq!(default_config.default_batch_size, 1000);

    let small_config = BatchConfig::small_batches();
    assert_eq!(small_config.default_batch_size, 100);

    let large_config = BatchConfig::large_batches();
    assert_eq!(large_config.default_batch_size, 5000);

    let custom_config = BatchConfig::new(250);
    assert_eq!(custom_config.default_batch_size, 250);
}

#[test]
fn test_schema_evolution_across_rows() {
    // First row has columns: id, age
    let mut row1 = IndexMap::new();
    row1.insert("id".to_string(), PropertyValue::String("1".to_string()));
    row1.insert("age".to_string(), PropertyValue::Float(25.0));

    // Second row adds a new column: name
    let mut row2 = IndexMap::new();
    row2.insert("id".to_string(), PropertyValue::String("2".to_string()));
    row2.insert("age".to_string(), PropertyValue::Float(30.0));
    row2.insert(
        "name".to_string(),
        PropertyValue::String("Alice".to_string()),
    );

    let rows = vec![Row::from_map(row1), Row::from_map(row2)];
    let batch = Batch::from(rows);

    // Batch should have all 3 columns
    assert_eq!(batch.num_columns(), 3);
    assert!(batch.contains_column("id"));
    assert!(batch.contains_column("age"));
    assert!(batch.contains_column("name"));

    // First row should have NULL for name
    let name_column = batch.column("name").unwrap();
    assert_eq!(name_column.get(0), None);
    assert_eq!(
        name_column.get(1),
        Some(PropertyValue::String("Alice".to_string()))
    );
}

#[test]
fn test_column_array_get_out_of_bounds() {
    let rows = vec![create_test_row("1", 25.0)];
    let batch = Batch::from(rows);

    let id_column = batch.column("id").unwrap();
    assert!(id_column.get(0).is_some());
    assert!(id_column.get(1).is_none());
    assert!(id_column.get(100).is_none());
}
