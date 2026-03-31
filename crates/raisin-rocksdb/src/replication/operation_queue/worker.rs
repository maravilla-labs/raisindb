//! Background worker for processing queued operations in batches
//!
//! The worker collects operations into batches and writes them to
//! RocksDB via OperationCapture, amortizing write costs.

use super::{QueueStats, QueuedOperation};
use crate::replication::OperationCapture;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// Background worker that processes operations in batches
///
/// This function runs in a separate task and:
/// 1. Collects operations into batches (up to `batch_size` or `batch_timeout`)
/// 2. Processes each batch by calling `operation_capture.capture_operation()`
/// 3. Updates statistics
/// 4. Continues until channel is closed and all operations are processed
pub(super) async fn process_operations(
    mut receiver: mpsc::Receiver<QueuedOperation>,
    operation_capture: Arc<OperationCapture>,
    batch_size: usize,
    batch_timeout: Duration,
    stats: Arc<QueueStats>,
) {
    debug!("Operation queue worker started");

    let mut batch = Vec::with_capacity(batch_size);

    loop {
        // Collect operations into batch with timeout
        let timeout_result = tokio::time::timeout(
            batch_timeout,
            collect_batch(&mut receiver, &mut batch, batch_size),
        )
        .await;

        // Process batch if we have operations
        if !batch.is_empty() {
            let batch_size_actual = batch.len();
            info!(
                batch_size = batch_size_actual,
                "Processing {} operation(s) from queue", batch_size_actual
            );

            process_batch(&batch, &operation_capture, &stats).await;

            // Update queue size after processing
            stats
                .current_queue_size
                .fetch_sub(batch_size_actual, Ordering::Relaxed);

            info!(
                batch_size = batch_size_actual,
                "Completed processing {} operation(s)", batch_size_actual
            );

            batch.clear();
        }

        // Exit if channel closed and no more operations
        match timeout_result {
            Ok(should_continue) => {
                if !should_continue {
                    // Channel closed
                    break;
                }
            }
            Err(_) => {
                // Timeout - continue to process next batch
                // This is normal and allows us to process partial batches
            }
        }
    }

    debug!("Operation queue worker stopped");
}

/// Collect operations into a batch
///
/// Fills the batch up to `max_size` or until the channel is closed.
///
/// # Returns
///
/// - `true` if the channel is still open
/// - `false` if the channel has been closed
async fn collect_batch(
    receiver: &mut mpsc::Receiver<QueuedOperation>,
    batch: &mut Vec<QueuedOperation>,
    max_size: usize,
) -> bool {
    while batch.len() < max_size {
        match receiver.recv().await {
            Some(op) => batch.push(op),
            None => {
                // Channel closed
                return false;
            }
        }
    }

    // Channel still open
    true
}

/// Process a batch of operations
///
/// Attempts to capture each operation, updating statistics for successes and failures.
/// Failures are logged but do not stop processing of the batch.
async fn process_batch(
    batch: &[QueuedOperation],
    operation_capture: &Arc<OperationCapture>,
    stats: &Arc<QueueStats>,
) {
    for op in batch {
        info!(
            tenant_id = %op.tenant_id,
            repo_id = %op.repo_id,
            op_type = ?op.op_type,
            "Calling capture_operation() for operation"
        );

        // Log operation BEFORE capturing (to trace revision through the queue)
        info!(
            revision = ?op.revision,
            "Processing operation with revision={:?}",
            op.revision
        );

        match operation_capture
            .capture_operation_with_revision(
                op.tenant_id.clone(),
                op.repo_id.clone(),
                op.branch.clone(),
                op.op_type.clone(),
                op.actor.clone(),
                op.message.clone(),
                op.is_system,
                op.revision,
            )
            .await
        {
            Ok(_) => {
                stats.processed_count.fetch_add(1, Ordering::Relaxed);
                info!(
                    tenant_id = %op.tenant_id,
                    repo_id = %op.repo_id,
                    "Operation captured successfully"
                );
            }
            Err(e) => {
                stats.failed_count.fetch_add(1, Ordering::Relaxed);
                warn!(
                    tenant_id = %op.tenant_id,
                    repo_id = %op.repo_id,
                    error = %e,
                    "Failed to capture operation from queue"
                );
            }
        }
    }
}
