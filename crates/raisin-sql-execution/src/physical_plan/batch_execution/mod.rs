//! Batch Execution Engine
//!
//! This module provides batch-aware execution capabilities for vectorized query processing.
//! Instead of processing rows one at a time, operators can process data in batches for
//! better CPU cache utilization and vectorization opportunities.
//!
//! # Architecture
//!
//! The batch execution layer mirrors the row-based execution but operates on batches:
//! - `BatchStream` - Stream of batches (vs `RowStream`)
//! - Batch-aware scan operators - Collect rows into batches before yielding
//! - Stream converters - Bridge between row and batch execution modes
//!
//! # Usage
//!
//! ```rust,ignore
//! use raisin_sql::physical_plan::batch::{execute_plan_batch, BatchStream};
//!
//! // Execute plan in batch mode
//! let batch_stream = execute_plan_batch(&plan, &ctx).await?;
//!
//! // Process batches
//! while let Some(batch) = batch_stream.next().await {
//!     let batch = batch?;
//!     // Process entire batch at once
//!     for row in batch.iter() {
//!         // ...
//!     }
//! }
//! ```
//!
//! # Performance
//!
//! Batch execution provides:
//! - Better CPU cache locality (columnar access patterns)
//! - Reduced per-row overhead (amortized across batch)
//! - Opportunities for SIMD vectorization (future enhancement)
//! - Improved throughput for OLAP-style queries
//!
//! # Module Structure
//!
//! - `mod.rs` (this file) - Public API and stream type definitions
//! - `config.rs` - Batch execution configuration
//! - `accumulator.rs` - Helper for collecting rows into batches
//! - `scan.rs` - Batch-aware scan operators
//! - `project.rs` - Batch-aware projection with columnar evaluation

use super::batch as data;
use super::executor::{ExecutionError, RowStream};
use futures::stream::Stream;
use std::pin::Pin;

// Module declarations
mod accumulator;
mod config;
mod project;
mod scan;

#[cfg(test)]
mod tests;

// Re-exports
pub use accumulator::RowAccumulator;
pub use config::BatchExecutionConfig;
pub use project::execute_project_batch;
pub use scan::{
    execute_prefix_scan_batch, execute_property_index_scan_batch, execute_table_scan_batch,
};

/// Stream of batches produced by batch-aware query execution
///
/// This is the batch equivalent of `RowStream`. Each item is a `Batch` containing
/// multiple rows in columnar format.
///
/// # Example
///
/// ```rust,ignore
/// let batch_stream: BatchStream = execute_plan_batch(&plan, &ctx).await?;
///
/// while let Some(result) = batch_stream.next().await {
///     let batch = result?;
///     println!("Processing batch with {} rows", batch.num_rows());
/// }
/// ```
pub type BatchStream = Pin<Box<dyn Stream<Item = Result<data::Batch, ExecutionError>> + Send>>;

/// Convert a row stream into a batch stream
///
/// This accumulates rows from a row-based stream into batches of the configured size.
/// Useful for adapting row-based operators to work in batch execution pipelines.
///
/// # Arguments
///
/// * `row_stream` - The input row stream to convert
/// * `batch_config` - Configuration specifying batch size
///
/// # Example
///
/// ```rust,ignore
/// let row_stream = execute_table_scan(&plan, &ctx).await?;
/// let batch_stream = convert_row_stream_to_batch_stream(row_stream, &config);
/// ```
pub fn convert_row_stream_to_batch_stream(
    row_stream: RowStream,
    batch_config: &data::BatchConfig,
) -> BatchStream {
    use async_stream::try_stream;
    use futures::StreamExt;

    let batch_size = batch_config.default_batch_size;

    Box::pin(try_stream! {
        let mut accumulator = RowAccumulator::new(batch_size);
        let mut stream = row_stream;

        while let Some(result) = stream.next().await {
            let row = result?;

            if let Some(batch) = accumulator.add_row(row) {
                yield batch;
            }
        }

        // Flush any remaining rows as a partial batch
        if let Some(batch) = accumulator.flush() {
            yield batch;
        }
    })
}

/// Convert a batch stream into a row stream
///
/// This flattens batches back into individual rows. Useful for integrating batch-aware
/// operators with row-based downstream operators.
///
/// # Arguments
///
/// * `batch_stream` - The input batch stream to convert
///
/// # Example
///
/// ```rust,ignore
/// let batch_stream = execute_table_scan_batch(&plan, &ctx).await?;
/// let row_stream = convert_batch_stream_to_row_stream(batch_stream);
/// ```
pub fn convert_batch_stream_to_row_stream(batch_stream: BatchStream) -> RowStream {
    use async_stream::try_stream;
    use futures::StreamExt;

    Box::pin(try_stream! {
        let mut stream = batch_stream;

        while let Some(result) = stream.next().await {
            let batch = result?;

            // Yield each row from the batch
            for row in batch.iter() {
                yield row;
            }
        }
    })
}

#[cfg(test)]
mod conversion_tests {
    use super::*;
    use crate::physical_plan::executor::Row;
    use async_stream::try_stream;
    use futures::StreamExt;
    use indexmap::indexmap;
    use raisin_models::nodes::properties::PropertyValue;

    fn create_test_row(id: i32) -> Row {
        Row::from_map(indexmap! {
            "id".to_string() => PropertyValue::Integer(id as i64),
            "name".to_string() => PropertyValue::String(format!("name_{}", id)),
        })
    }

    fn create_test_row_stream(num_rows: usize) -> RowStream {
        Box::pin(try_stream! {
            for i in 0..num_rows {
                yield create_test_row(i as i32);
            }
        })
    }

    #[tokio::test]
    async fn test_row_to_batch_conversion() {
        let row_stream = create_test_row_stream(5);
        let batch_config = data::BatchConfig::new(3);

        let mut batch_stream = convert_row_stream_to_batch_stream(row_stream, &batch_config);

        // First batch should have 3 rows
        let batch1 = batch_stream.next().await.unwrap().unwrap();
        assert_eq!(batch1.num_rows(), 3);

        // Second batch should have 2 rows (partial)
        let batch2 = batch_stream.next().await.unwrap().unwrap();
        assert_eq!(batch2.num_rows(), 2);

        // No more batches
        assert!(batch_stream.next().await.is_none());
    }

    #[tokio::test]
    async fn test_batch_to_row_conversion() {
        let row_stream = create_test_row_stream(5);
        let batch_config = data::BatchConfig::new(3);

        let batch_stream = convert_row_stream_to_batch_stream(row_stream, &batch_config);
        let mut row_stream_back = convert_batch_stream_to_row_stream(batch_stream);

        // Should get all 5 rows back
        let mut count = 0;
        while let Some(result) = row_stream_back.next().await {
            result.unwrap();
            count += 1;
        }
        assert_eq!(count, 5);
    }

    #[tokio::test]
    async fn test_round_trip_preserves_data() {
        let row_stream = create_test_row_stream(10);
        let batch_config = data::BatchConfig::new(4);

        // Convert to batch and back to row
        let batch_stream = convert_row_stream_to_batch_stream(row_stream, &batch_config);
        let mut row_stream_back = convert_batch_stream_to_row_stream(batch_stream);

        // Verify all data is preserved
        for i in 0..10 {
            let row = row_stream_back.next().await.unwrap().unwrap();
            assert_eq!(row.get("id").unwrap(), &PropertyValue::Integer(i as i64));
        }

        assert!(row_stream_back.next().await.is_none());
    }

    #[tokio::test]
    async fn test_empty_stream() {
        let row_stream = create_test_row_stream(0);
        let batch_config = data::BatchConfig::new(10);

        let mut batch_stream = convert_row_stream_to_batch_stream(row_stream, &batch_config);

        // Should get no batches
        assert!(batch_stream.next().await.is_none());
    }

    #[tokio::test]
    async fn test_single_row() {
        let row_stream = create_test_row_stream(1);
        let batch_config = data::BatchConfig::new(10);

        let mut batch_stream = convert_row_stream_to_batch_stream(row_stream, &batch_config);

        // Should get one batch with one row
        let batch = batch_stream.next().await.unwrap().unwrap();
        assert_eq!(batch.num_rows(), 1);

        assert!(batch_stream.next().await.is_none());
    }

    #[tokio::test]
    async fn test_exact_batch_size() {
        let row_stream = create_test_row_stream(10);
        let batch_config = data::BatchConfig::new(10);

        let mut batch_stream = convert_row_stream_to_batch_stream(row_stream, &batch_config);

        // Should get exactly one full batch
        let batch = batch_stream.next().await.unwrap().unwrap();
        assert_eq!(batch.num_rows(), 10);

        assert!(batch_stream.next().await.is_none());
    }
}
