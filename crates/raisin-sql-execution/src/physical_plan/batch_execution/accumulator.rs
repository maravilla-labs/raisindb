//! Row Accumulator for Batch Construction
//!
//! Provides utilities for collecting individual rows into batches.
//! This is the core helper used by all batch-aware scan operators.

use super::super::batch::Batch;
use super::super::executor::Row;

/// Accumulates rows into batches of a configured size
///
/// This helper manages the buffering of rows and automatic batch emission
/// when the batch size is reached. It handles partial batches at the end
/// of a stream via the `flush()` method.
///
/// # Example
///
/// ```rust,ignore
/// let mut accumulator = RowAccumulator::new(1000);
///
/// for row in rows {
///     if let Some(batch) = accumulator.add_row(row) {
///         // Full batch ready, yield it
///         yield batch;
///     }
/// }
///
/// // Flush remaining rows
/// if let Some(batch) = accumulator.flush() {
///     yield batch;
/// }
/// ```
#[derive(Debug)]
pub struct RowAccumulator {
    /// Target batch size (number of rows per batch)
    batch_size: usize,

    /// Buffer of accumulated rows
    buffer: Vec<Row>,
}

impl RowAccumulator {
    /// Create a new row accumulator with the specified batch size
    ///
    /// # Arguments
    ///
    /// * `batch_size` - Number of rows to accumulate before emitting a batch
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let accumulator = RowAccumulator::new(1000);
    /// ```
    pub fn new(batch_size: usize) -> Self {
        Self {
            batch_size,
            // Pre-allocate buffer to avoid reallocation during accumulation
            buffer: Vec::with_capacity(batch_size),
        }
    }

    /// Add a row to the accumulator
    ///
    /// If adding this row fills the batch, returns Some(Batch) with the completed batch.
    /// Otherwise returns None and the row is buffered.
    ///
    /// # Arguments
    ///
    /// * `row` - The row to add to the current batch
    ///
    /// # Returns
    ///
    /// * `Some(Batch)` - A complete batch ready to be yielded
    /// * `None` - Row was buffered, batch not yet full
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// for row in rows {
    ///     if let Some(batch) = accumulator.add_row(row) {
    ///         yield batch; // Full batch ready
    ///     }
    /// }
    /// ```
    pub fn add_row(&mut self, row: Row) -> Option<Batch> {
        self.buffer.push(row);

        if self.buffer.len() >= self.batch_size {
            // Batch is full, emit it
            Some(self.create_batch())
        } else {
            // Still accumulating
            None
        }
    }

    /// Flush any remaining rows as a partial batch
    ///
    /// Call this at the end of a stream to emit any buffered rows that didn't
    /// fill a complete batch. Returns None if the buffer is empty.
    ///
    /// # Returns
    ///
    /// * `Some(Batch)` - A partial batch with the remaining rows
    /// * `None` - No rows were buffered
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // After processing all rows
    /// if let Some(batch) = accumulator.flush() {
    ///     yield batch; // Partial batch with remaining rows
    /// }
    /// ```
    pub fn flush(&mut self) -> Option<Batch> {
        if self.buffer.is_empty() {
            None
        } else {
            Some(self.create_batch())
        }
    }

    /// Get the current number of buffered rows
    ///
    /// Useful for monitoring and debugging.
    pub fn buffered_rows(&self) -> usize {
        self.buffer.len()
    }

    /// Check if the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Get the configured batch size
    pub fn batch_size(&self) -> usize {
        self.batch_size
    }

    /// Create a batch from the current buffer and reset the buffer
    ///
    /// This is called internally by `add_row` when the batch is full
    /// and by `flush` to emit partial batches.
    fn create_batch(&mut self) -> Batch {
        // Take all buffered rows
        let rows = std::mem::take(&mut self.buffer);

        // Reset buffer for next batch (reuse the allocated capacity)
        self.buffer = Vec::with_capacity(self.batch_size);

        // Convert rows to batch
        Batch::from(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::indexmap;
    use raisin_models::nodes::properties::PropertyValue;

    fn create_test_row(id: i32) -> Row {
        Row::from_map(indexmap! {
            "id".to_string() => PropertyValue::Integer(id as i64),
            "name".to_string() => PropertyValue::String(format!("row_{}", id)),
        })
    }

    #[test]
    fn test_accumulator_basic() {
        let mut acc = RowAccumulator::new(3);

        // Add first row - not full
        assert!(acc.add_row(create_test_row(1)).is_none());
        assert_eq!(acc.buffered_rows(), 1);

        // Add second row - not full
        assert!(acc.add_row(create_test_row(2)).is_none());
        assert_eq!(acc.buffered_rows(), 2);

        // Add third row - full, returns batch
        let batch = acc.add_row(create_test_row(3)).unwrap();
        assert_eq!(batch.num_rows(), 3);
        assert_eq!(acc.buffered_rows(), 0); // Buffer cleared
    }

    #[test]
    fn test_accumulator_flush() {
        let mut acc = RowAccumulator::new(5);

        // Add 3 rows (less than batch size)
        acc.add_row(create_test_row(1));
        acc.add_row(create_test_row(2));
        acc.add_row(create_test_row(3));

        assert_eq!(acc.buffered_rows(), 3);

        // Flush should return partial batch
        let batch = acc.flush().unwrap();
        assert_eq!(batch.num_rows(), 3);
        assert_eq!(acc.buffered_rows(), 0);

        // Second flush should return None
        assert!(acc.flush().is_none());
    }

    #[test]
    fn test_accumulator_multiple_batches() {
        let mut acc = RowAccumulator::new(2);

        // First batch
        assert!(acc.add_row(create_test_row(1)).is_none());
        let batch1 = acc.add_row(create_test_row(2)).unwrap();
        assert_eq!(batch1.num_rows(), 2);

        // Second batch
        assert!(acc.add_row(create_test_row(3)).is_none());
        let batch2 = acc.add_row(create_test_row(4)).unwrap();
        assert_eq!(batch2.num_rows(), 2);

        // Partial third batch
        acc.add_row(create_test_row(5));
        let batch3 = acc.flush().unwrap();
        assert_eq!(batch3.num_rows(), 1);
    }

    #[test]
    fn test_accumulator_empty() {
        let mut acc = RowAccumulator::new(10);
        assert!(acc.is_empty());
        assert_eq!(acc.buffered_rows(), 0);
        assert!(acc.flush().is_none());
    }

    #[test]
    fn test_accumulator_exact_batch_size() {
        let mut acc = RowAccumulator::new(3);

        // Fill exactly to batch size
        acc.add_row(create_test_row(1));
        acc.add_row(create_test_row(2));
        let batch = acc.add_row(create_test_row(3)).unwrap();

        assert_eq!(batch.num_rows(), 3);
        assert!(acc.is_empty());
        assert!(acc.flush().is_none()); // Nothing left to flush
    }

    #[test]
    fn test_accumulator_single_row_batch() {
        let mut acc = RowAccumulator::new(1);

        // Every row should produce a batch
        let batch1 = acc.add_row(create_test_row(1)).unwrap();
        assert_eq!(batch1.num_rows(), 1);

        let batch2 = acc.add_row(create_test_row(2)).unwrap();
        assert_eq!(batch2.num_rows(), 1);

        assert!(acc.flush().is_none());
    }

    #[test]
    fn test_accumulator_batch_size() {
        let acc = RowAccumulator::new(100);
        assert_eq!(acc.batch_size(), 100);
    }

    #[test]
    fn test_accumulator_buffer_reuse() {
        let mut acc = RowAccumulator::new(2);

        // First batch
        acc.add_row(create_test_row(1));
        acc.add_row(create_test_row(2));

        // Second batch - buffer should be reused (capacity preserved)
        acc.add_row(create_test_row(3));
        assert_eq!(acc.buffer.capacity(), 2); // Capacity preserved
    }

    #[test]
    fn test_accumulator_with_different_row_schemas() {
        let mut acc = RowAccumulator::new(2);

        // Row with different columns
        let row1 = Row::from_map(indexmap! {
            "a".to_string() => PropertyValue::Float(1.0),
        });

        let row2 = Row::from_map(indexmap! {
            "b".to_string() => PropertyValue::String("test".to_string()),
        });

        acc.add_row(row1);
        let batch = acc.add_row(row2).unwrap();

        // Batch should handle different schemas
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 2); // Union of columns
    }
}
