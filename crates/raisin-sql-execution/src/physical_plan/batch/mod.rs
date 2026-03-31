//! Columnar Batch Data Structure for Vectorized Query Execution
//!
//! This module provides a columnar batch data structure that stores rows in a column-oriented
//! format for improved CPU cache locality and vectorization opportunities.
//!
//! # Architecture
//!
//! Instead of storing data row-by-row:
//! ```text
//! [(username1, age1), (username2, age2), (username3, age3)]
//! ```
//!
//! We store it column-by-column:
//! ```text
//! {
//!   username: [username1, username2, username3],
//!   age: [age1, age2, age3]
//! }
//! ```
//!
//! # Performance Benefits
//!
//! 1. **Cache Locality**: Accessing a column sequentially is cache-friendly
//! 2. **Vectorization**: SIMD operations can process entire columns efficiently
//! 3. **Compression**: Columnar data compresses better (future optimization)
//! 4. **Column Pruning**: Only load columns needed for the query
//!
//! # Example
//!
//! ```rust,ignore
//! use raisin_sql::physical_plan::batch::{Batch, BatchConfig};
//! use raisin_sql::physical_plan::Row;
//! use raisin_models::nodes::properties::PropertyValue;
//!
//! // Create batch from rows
//! let rows = vec![
//!     Row::from_map(indexmap! {
//!         "id".to_string() => PropertyValue::String("1".to_string()),
//!         "age".to_string() => PropertyValue::Float(25.0),
//!     }),
//!     Row::from_map(indexmap! {
//!         "id".to_string() => PropertyValue::String("2".to_string()),
//!         "age".to_string() => PropertyValue::Float(30.0),
//!     }),
//! ];
//!
//! let batch = Batch::from(rows);
//! assert_eq!(batch.num_rows(), 2);
//! assert_eq!(batch.num_columns(), 2);
//!
//! // Access column data
//! if let Some(column) = batch.column("age") {
//!     // Process entire column at once
//! }
//! ```

mod column_array;
mod conversions;

#[cfg(test)]
mod tests;

pub use column_array::ColumnArray;

use crate::physical_plan::executor::Row;
use indexmap::IndexMap;

/// Configuration for batch processing
#[derive(Debug, Clone)]
pub struct BatchConfig {
    /// Default batch size (number of rows per batch)
    /// Trade-off: Larger batches = better vectorization but more memory
    pub default_batch_size: usize,
}

impl Default for BatchConfig {
    fn default() -> Self {
        Self {
            // 1000 rows is a good balance between memory and vectorization
            default_batch_size: 1000,
        }
    }
}

impl BatchConfig {
    /// Create a new batch configuration with custom batch size
    pub fn new(batch_size: usize) -> Self {
        Self {
            default_batch_size: batch_size,
        }
    }

    /// Create configuration optimized for small batches (better for low latency)
    pub fn small_batches() -> Self {
        Self {
            default_batch_size: 100,
        }
    }

    /// Create configuration optimized for large batches (better for throughput)
    pub fn large_batches() -> Self {
        Self {
            default_batch_size: 5000,
        }
    }
}

/// A batch of rows stored in columnar format
///
/// Each column is stored as a separate array, allowing for efficient vectorized
/// processing. Columns are stored in an IndexMap to preserve insertion order
/// and enable efficient column access by name.
#[derive(Debug, Clone, PartialEq)]
pub struct Batch {
    /// Columnar storage: column name -> array of values
    columns: IndexMap<String, ColumnArray>,
    /// Number of rows in this batch
    num_rows: usize,
}

impl Batch {
    /// Create a new empty batch
    pub fn new() -> Self {
        Self {
            columns: IndexMap::new(),
            num_rows: 0,
        }
    }

    /// Create a new empty batch with pre-allocated column capacity
    pub fn with_column_capacity(capacity: usize) -> Self {
        Self {
            columns: IndexMap::with_capacity(capacity),
            num_rows: 0,
        }
    }

    /// Get the number of rows in this batch
    pub fn num_rows(&self) -> usize {
        self.num_rows
    }

    /// Get the number of columns in this batch
    pub fn num_columns(&self) -> usize {
        self.columns.len()
    }

    /// Check if this batch is empty
    pub fn is_empty(&self) -> bool {
        self.num_rows == 0
    }

    /// Get a column by name
    pub fn column(&self, name: &str) -> Option<&ColumnArray> {
        self.columns.get(name)
    }

    /// Get all column names in insertion order
    pub fn schema(&self) -> Vec<&str> {
        self.columns.keys().map(|s| s.as_str()).collect()
    }

    /// Get a row at the specified index
    ///
    /// This reconstructs a row from the columnar format. For performance,
    /// prefer working with entire columns when possible.
    pub fn row(&self, index: usize) -> Option<Row> {
        if index >= self.num_rows {
            return None;
        }

        let mut columns = IndexMap::with_capacity(self.columns.len());

        for (col_name, col_array) in &self.columns {
            if let Some(value) = col_array.get(index) {
                columns.insert(col_name.clone(), value);
            }
        }

        Some(Row::from_map(columns))
    }

    /// Check if this batch contains a column with the given name
    pub fn contains_column(&self, name: &str) -> bool {
        self.columns.contains_key(name)
    }

    /// Get an iterator over all rows in this batch
    ///
    /// Note: This reconstructs rows from columnar format and allocates.
    /// For performance-critical code, prefer working with columns directly.
    pub fn iter(&self) -> BatchIterator<'_> {
        BatchIterator {
            batch: self,
            current_index: 0,
        }
    }

    /// Create a batch directly from columnar data
    ///
    /// This is the inverse of converting rows to batches. It's useful for
    /// operators that produce columnar output directly (like batch-aware projection).
    ///
    /// # Arguments
    ///
    /// * `columns` - Map of column names to column arrays
    ///
    /// # Panics
    ///
    /// Panics if columns have different lengths
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let mut columns = IndexMap::new();
    /// columns.insert("name".to_string(), ColumnArray::String(vec![Some("Alice".to_string())]));
    /// columns.insert("age".to_string(), ColumnArray::Number(vec![Some(30.0)]));
    /// let batch = Batch::from_columns(columns);
    /// ```
    pub fn from_columns(columns: IndexMap<String, ColumnArray>) -> Self {
        // Determine number of rows from first column
        let num_rows = columns.values().next().map(|col| col.len()).unwrap_or(0);

        // Validate all columns have same length
        #[cfg(debug_assertions)]
        {
            for (name, col) in &columns {
                assert_eq!(
                    col.len(),
                    num_rows,
                    "Column '{}' has {} rows, expected {}",
                    name,
                    col.len(),
                    num_rows
                );
            }
        }

        Self { columns, num_rows }
    }
}

impl Default for Batch {
    fn default() -> Self {
        Self::new()
    }
}

/// Iterator over rows in a batch
pub struct BatchIterator<'a> {
    batch: &'a Batch,
    current_index: usize,
}

impl<'a> Iterator for BatchIterator<'a> {
    type Item = Row;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current_index >= self.batch.num_rows {
            return None;
        }

        let row = self.batch.row(self.current_index);
        self.current_index += 1;
        row
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.batch.num_rows.saturating_sub(self.current_index);
        (remaining, Some(remaining))
    }
}

impl<'a> ExactSizeIterator for BatchIterator<'a> {
    fn len(&self) -> usize {
        self.batch.num_rows.saturating_sub(self.current_index)
    }
}
