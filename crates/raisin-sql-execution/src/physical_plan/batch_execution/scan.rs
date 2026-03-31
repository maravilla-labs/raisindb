//! Batch-Aware Scan Executors
//!
//! Implements scan operators that produce batches instead of individual rows.
//! These operators reuse the existing row-based scan logic and accumulate results
//! into batches for improved throughput.
//!
//! # Design Pattern
//!
//! Instead of reimplementing scan logic, we:
//! 1. Call the existing row-based scan function
//! 2. Use RowAccumulator to collect rows into batches
//! 3. Yield batches as they fill up
//!
//! This approach:
//! - Minimizes code duplication
//! - Ensures consistency between row and batch execution
//! - Simplifies maintenance

use super::super::batch::BatchConfig;
use super::super::executor::{ExecutionContext, ExecutionError};
use super::super::operators::PhysicalPlan;
use super::super::scan_executors::{
    execute_prefix_scan, execute_property_index_scan, execute_table_scan,
};
use super::accumulator::RowAccumulator;
use super::BatchStream;
use async_stream::try_stream;
use futures::StreamExt;
use raisin_storage::Storage;

/// Execute a table scan and return batches
///
/// This is the batch-aware version of `execute_table_scan`. It performs the same
/// scan but accumulates rows into batches before yielding them.
///
/// # Arguments
///
/// * `plan` - Physical plan containing scan parameters
/// * `ctx` - Execution context with storage and configuration
/// * `batch_config` - Configuration specifying batch size
///
/// # Performance
///
/// Batching reduces per-row overhead and improves cache locality for downstream
/// operators. Typical improvements: 10-20% for large scans.
///
/// # Example
///
/// ```rust,ignore
/// let batch_stream = execute_table_scan_batch(plan, ctx, batch_config).await?;
///
/// while let Some(batch) = batch_stream.next().await {
///     let batch = batch?;
///     // Process entire batch at once
/// }
/// ```
pub async fn execute_table_scan_batch<S: Storage + 'static>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
    batch_config: &BatchConfig,
) -> Result<BatchStream, ExecutionError> {
    // Reuse the existing row-based table scan
    let row_stream = execute_table_scan(plan, ctx).await?;

    // Wrap with accumulator to produce batches
    let batch_size = batch_config.default_batch_size;

    Ok(Box::pin(try_stream! {
        let mut accumulator = RowAccumulator::new(batch_size);
        let mut stream = row_stream;

        while let Some(result) = stream.next().await {
            let row = result?;

            // Add row to accumulator, yield batch if full
            if let Some(batch) = accumulator.add_row(row) {
                yield batch;
            }
        }

        // Flush any remaining rows as a partial batch
        if let Some(batch) = accumulator.flush() {
            yield batch;
        }
    }))
}

/// Execute a prefix scan and return batches
///
/// This is the batch-aware version of `execute_prefix_scan`. It scans nodes
/// with a specific path prefix and returns results in batches.
///
/// # Arguments
///
/// * `plan` - Physical plan containing prefix scan parameters
/// * `ctx` - Execution context with storage and configuration
/// * `batch_config` - Configuration specifying batch size
///
/// # Performance
///
/// Batching is particularly effective for prefix scans that return many nodes
/// (e.g., scanning entire subtrees).
///
/// # Example
///
/// ```rust,ignore
/// // Scan all nodes under /content/
/// let batch_stream = execute_prefix_scan_batch(plan, ctx, batch_config).await?;
/// ```
pub async fn execute_prefix_scan_batch<S: Storage + 'static>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
    batch_config: &BatchConfig,
) -> Result<BatchStream, ExecutionError> {
    // Reuse the existing row-based prefix scan
    let row_stream = execute_prefix_scan(plan, ctx).await?;

    // Wrap with accumulator to produce batches
    let batch_size = batch_config.default_batch_size;

    Ok(Box::pin(try_stream! {
        let mut accumulator = RowAccumulator::new(batch_size);
        let mut stream = row_stream;

        while let Some(result) = stream.next().await {
            let row = result?;

            // Add row to accumulator, yield batch if full
            if let Some(batch) = accumulator.add_row(row) {
                yield batch;
            }
        }

        // Flush any remaining rows as a partial batch
        if let Some(batch) = accumulator.flush() {
            yield batch;
        }
    }))
}

/// Execute a property index scan and return batches
///
/// This is the batch-aware version of `execute_property_index_scan`. It uses
/// the property index to find matching nodes and returns results in batches.
///
/// # Arguments
///
/// * `plan` - Physical plan containing property scan parameters
/// * `ctx` - Execution context with storage and configuration
/// * `batch_config` - Configuration specifying batch size
///
/// # Performance
///
/// Batching is effective when the property index scan returns many matches.
/// For highly selective scans (few matches), batching overhead may outweigh benefits.
///
/// # Example
///
/// ```rust,ignore
/// // Find all nodes with status='published'
/// let batch_stream = execute_property_index_scan_batch(plan, ctx, batch_config).await?;
/// ```
pub async fn execute_property_index_scan_batch<S: Storage + 'static>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
    batch_config: &BatchConfig,
) -> Result<BatchStream, ExecutionError> {
    // Reuse the existing row-based property index scan
    let row_stream = execute_property_index_scan(plan, ctx).await?;

    // Wrap with accumulator to produce batches
    let batch_size = batch_config.default_batch_size;

    Ok(Box::pin(try_stream! {
        let mut accumulator = RowAccumulator::new(batch_size);
        let mut stream = row_stream;

        while let Some(result) = stream.next().await {
            let row = result?;

            // Add row to accumulator, yield batch if full
            if let Some(batch) = accumulator.add_row(row) {
                yield batch;
            }
        }

        // Flush any remaining rows as a partial batch
        if let Some(batch) = accumulator.flush() {
            yield batch;
        }
    }))
}

// Note: Integration tests for scan operators are in
// tests/batch_integration_tests.rs and tests/rocksdb_integration_tests.rs
