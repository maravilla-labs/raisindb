//! Row-to-batch and batch-to-row conversion implementations.
//!
//! Provides `From<Vec<Row>>` for `Batch` (row-to-columnar conversion with schema evolution)
//! and `From<Batch>` for `Vec<Row>` (columnar-to-row reconstruction).

use super::column_array::ColumnArray;
use super::Batch;
use crate::physical_plan::executor::Row;
use indexmap::IndexMap;
use raisin_models::nodes::properties::PropertyValue;

/// Create a [`ColumnArray`] with pre-allocated capacity from a [`PropertyValue`] type hint.
///
/// Used during row-to-batch conversion to initialize typed columns based on the first
/// observed value for each column name.
fn column_array_for_value(value: &PropertyValue, capacity: usize) -> ColumnArray {
    match value {
        PropertyValue::Null => ColumnArray::String(Vec::with_capacity(capacity)),
        PropertyValue::Date(_) => ColumnArray::Date(Vec::with_capacity(capacity)),
        PropertyValue::Boolean(_) => ColumnArray::Boolean(Vec::with_capacity(capacity)),
        PropertyValue::Integer(_) => ColumnArray::Integer(Vec::with_capacity(capacity)),
        PropertyValue::Float(_) => ColumnArray::Float(Vec::with_capacity(capacity)),
        PropertyValue::Decimal(_) => ColumnArray::Float(Vec::with_capacity(capacity)),
        PropertyValue::String(_) => ColumnArray::String(Vec::with_capacity(capacity)),
        PropertyValue::Url(_) => ColumnArray::Url(Vec::with_capacity(capacity)),
        PropertyValue::Reference(_) => ColumnArray::Reference(Vec::with_capacity(capacity)),
        PropertyValue::Resource(_) => ColumnArray::Resource(Vec::with_capacity(capacity)),
        PropertyValue::Composite(_) => ColumnArray::Composite(Vec::with_capacity(capacity)),
        PropertyValue::Element(_) => ColumnArray::Element(Vec::with_capacity(capacity)),
        PropertyValue::Vector(_) => ColumnArray::Vector(Vec::with_capacity(capacity)),
        PropertyValue::Geometry(_) => ColumnArray::String(Vec::with_capacity(capacity)),
        PropertyValue::Array(_) => ColumnArray::Array(Vec::with_capacity(capacity)),
        PropertyValue::Object(_) => ColumnArray::Object(Vec::with_capacity(capacity)),
    }
}

/// Convert a vector of rows to a columnar batch
///
/// This conversion:
/// 1. Analyzes the first row to determine column types
/// 2. Pre-allocates column arrays based on row count
/// 3. Iterates through rows, appending values to appropriate columns
/// 4. Handles missing columns by inserting NULL values
impl From<Vec<Row>> for Batch {
    fn from(rows: Vec<Row>) -> Self {
        if rows.is_empty() {
            return Batch::new();
        }

        let num_rows = rows.len();

        // Determine schema from the first row
        let first_row = &rows[0];
        let mut columns = IndexMap::with_capacity(first_row.columns.len());

        // Initialize column arrays based on first row's schema
        for (col_name, value) in &first_row.columns {
            let column_array = column_array_for_value(value, num_rows);
            columns.insert(col_name.clone(), column_array);
        }

        // Collect all unique column names from all rows (handle schema evolution)
        let mut all_column_names = first_row.columns.keys().cloned().collect::<Vec<_>>();
        for row in rows.iter().skip(1) {
            for col_name in row.columns.keys() {
                if !columns.contains_key(col_name) {
                    all_column_names.push(col_name.clone());
                }
            }
        }

        // Initialize any new columns discovered after the first row
        for col_name in &all_column_names {
            if !columns.contains_key(col_name) {
                // Find the first row with this column to determine type
                if let Some(row) = rows.iter().find(|r| r.columns.contains_key(col_name)) {
                    if let Some(value) = row.columns.get(col_name) {
                        let column_array = column_array_for_value(value, num_rows);
                        columns.insert(col_name.clone(), column_array);
                    }
                }
            }
        }

        // Populate columns from rows
        for row in &rows {
            for (col_name, col_array) in columns.iter_mut() {
                let value = row.columns.get(col_name).cloned();
                col_array.push(value);
            }
        }

        Batch { columns, num_rows }
    }
}

/// Convert a batch back to a vector of rows
///
/// This is useful for interoperating with row-based operators.
/// Note: This allocates a new vector and reconstructs all rows.
impl From<Batch> for Vec<Row> {
    fn from(batch: Batch) -> Self {
        let mut rows = Vec::with_capacity(batch.num_rows);

        for i in 0..batch.num_rows {
            if let Some(row) = batch.row(i) {
                rows.push(row);
            }
        }

        rows
    }
}
