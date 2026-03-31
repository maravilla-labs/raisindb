//! Replication startup and coordinator initialization.
//!
//! Contains the start_replication entry point for initializing
//! the replication coordinator, connecting to peers, and running catch-up
//! if the local node is behind.

use std::sync::Arc;

use raisin_replication::{
    ClusterConfig, CoordinatorError, OperationLogStorage, ReplicationCoordinator,
};
use raisin_storage::Storage;

use crate::RocksDBStorage;

use super::catchup::{assess_catchup_need, assess_peer_divergence};
use super::checkpoint_ingestor::RocksDbCheckpointIngestor;
use super::oplog_storage::RocksDbOperationLogStorage;

/// Start replication coordinator for RocksDB
///
/// This function initializes and starts the replication coordinator,
/// connecting to configured peers and beginning synchronization.
///
/// # Example
///
/// ```rust,ignore
/// use raisin_rocksdb::replication::integration::start_replication;
/// use raisin_replication::ClusterConfig;
///
/// let db = Arc::new(RocksDBStorage::new(config)?);
/// let cluster_config = ClusterConfig::from_toml_file("config/cluster.toml")?;
/// let coordinator = start_replication(db, cluster_config).await?;
/// ```
pub async fn start_replication(
    db: Arc<RocksDBStorage>,
    cluster_config: ClusterConfig,
) -> Result<Arc<ReplicationCoordinator>, CoordinatorError> {
    // Create storage adapter
    let storage = Arc::new(RocksDbOperationLogStorage::new(db.clone()));
    let storage_adapter: Arc<dyn OperationLogStorage> = storage.clone();

    // Create persistent idempotency tracker
    // Use APPLIED_OPS column family for tracking which operations have been applied
    let idempotency_tracker = Box::new(crate::replication::PersistentIdempotencyTracker::new(
        db.db().clone(),
        crate::cf::APPLIED_OPS.to_string(),
    )) as Box<dyn raisin_replication::IdempotencyTracker>;

    // Create coordinator with persistent idempotency tracking
    let mut coordinator = ReplicationCoordinator::new_with_tracker(
        cluster_config.clone(),
        storage_adapter,
        idempotency_tracker,
    )?;

    // Create and configure CheckpointServer for serving snapshots during catch-up
    let checkpoint_dir = db.config().path.join("checkpoints");

    // CRITICAL FIX: Create index managers for transferring Tantivy and HNSW indexes during catch-up
    // Without these, new nodes won't have fulltext or vector search capabilities
    let tantivy_base_dir = db.config().path.join("tantivy_indexes");
    let hnsw_base_dir = db.config().path.join("hnsw_indexes");

    let tantivy_manager = Some(Arc::new(crate::TantivyIndexManager::new(
        tantivy_base_dir.clone(),
    )));
    let hnsw_manager = Some(Arc::new(crate::HnswIndexManager::new(
        hnsw_base_dir.clone(),
    )));

    let checkpoint_server = Arc::new(crate::replication::CheckpointServer::new(
        db.db().clone(),
        checkpoint_dir,
        cluster_config.node_id.clone(),
        tantivy_manager, // Enable Tantivy index transfer
        hnsw_manager,    // Enable HNSW index transfer
    ));

    // Set checkpoint provider on coordinator before starting
    coordinator.set_checkpoint_provider(checkpoint_server);

    let coordinator = Arc::new(coordinator);

    // CRITICAL: Set coordinator on storage BEFORE starting coordinator or catch-up
    // This ensures push_operations_to_peers() can access coordinator when callback is invoked
    db.set_replication_coordinator(coordinator.clone()).await;

    // CRITICAL: Set up real-time push callback BEFORE starting coordinator
    // This ensures callback is ready before ANY operations can be captured (startup, catch-up, etc.)
    let db_clone = db.clone();
    db.operation_capture()
        .set_push_callback(move |op| {
            tracing::info!(
                op_id = %op.op_id,
                op_seq = op.op_seq,
                cluster_node_id = %op.cluster_node_id,
                tenant_id = %op.tenant_id,
                repo_id = %op.repo_id,
                op_type = ?op.op_type,
                revision = ?op.revision,
                "📞 PUSH CALLBACK INVOKED with revision={:?}",
                op.revision
            );
            // Spawn async task to push (callback must be sync)
            let db_ref = db_clone.clone();
            let op_id = op.op_id;
            let ops = vec![op];
            tokio::spawn(async move {
                tracing::info!(op_id = %op_id, "📤 PUSHING operation to peers via coordinator");
                if let Err(e) = db_ref.push_operations_to_peers(ops).await {
                    tracing::error!(op_id = %op_id, error = %e, "❌ REAL-TIME PUSH FAILED");
                } else {
                    tracing::info!(op_id = %op_id, "✅ REAL-TIME PUSH SUCCEEDED");
                }
            });
            Ok(())
        })
        .await;

    // Start coordinator (connect to peers, start sync tasks)
    coordinator.clone().start(cluster_config.clone()).await?;

    // IMPORTANT: Wait for initial peer connections to establish before checking catch-up need
    // This gives time for at least one peer to connect successfully
    tracing::info!("Waiting for peer connections to establish...");
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Check if catch-up is needed by comparing local vs peer vector clocks
    let assessment = assess_catchup_need(&db, &cluster_config).await;
    let mut catch_up_needed = assessment.requires_catch_up;

    if !catch_up_needed {
        match assess_peer_divergence(&storage, &coordinator).await {
            Ok(Some(lag)) => {
                tracing::info!(
                    peer_node_id = %lag.peer_node_id,
                    configured_peer_id = %lag.configured_peer_id,
                    lag_distance = lag.lag_distance,
                    "Peer state is ahead of local node - triggering catch-up protocol"
                );
                catch_up_needed = true;
            }
            Ok(None) => {
                tracing::debug!(
                    "Peer divergence check indicates local node is caught up with connected peers"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "Failed to compare vector clocks with peers; skipping divergence-based catch-up trigger"
                );
            }
        }
    }

    tracing::info!(
        requires_catch_up = catch_up_needed,
        total_pairs = assessment.total_pairs,
        skipped_registry = assessment.skipped_registry,
        repositories_with_data = assessment.non_empty_pairs.len(),
        repositories_without_data = assessment.empty_pairs.len(),
        "Catch-up assessment completed"
    );

    if catch_up_needed {
        tracing::info!("Fresh node or significantly behind peers - triggering automatic catch-up");

        // Create temporary directories for catch-up
        let temp_data = tempfile::TempDir::new()
            .map_err(|e| CoordinatorError::Storage(format!("Failed to create temp dir: {}", e)))?;
        let temp_staging = tempfile::TempDir::new()
            .map_err(|e| CoordinatorError::Storage(format!("Failed to create temp dir: {}", e)))?;

        // Collect peer addresses
        let peer_addresses: Vec<String> = cluster_config
            .peers
            .iter()
            .map(|p| format!("{}:{}", p.host, p.port))
            .collect();

        // CRITICAL FIX: Create index receivers for ingesting Tantivy and HNSW indexes during catch-up
        // Without these, fulltext and vector search won't work on new nodes
        let tantivy_staging = temp_staging.path().join("tantivy_indexes");
        let hnsw_staging = temp_staging.path().join("hnsw_indexes");

        let tantivy_receiver = Some(Arc::new(crate::TantivyIndexReceiver::new(
            tantivy_base_dir.clone(),
            tantivy_staging,
        ))
            as Arc<dyn raisin_replication::TantivyIndexReceiver>);

        let hnsw_receiver = Some(Arc::new(crate::HnswIndexReceiver::new(
            hnsw_base_dir.clone(),
            hnsw_staging,
        )) as Arc<dyn raisin_replication::HnswIndexReceiver>);

        // Create catch-up coordinator with checkpoint ingestor and index receivers
        let catch_up = raisin_replication::CatchUpCoordinator::new(
            cluster_config.node_id.clone(),
            peer_addresses,
            temp_data.path().to_path_buf(),
            temp_staging.path().to_path_buf(),
            Some(Arc::new(RocksDbOperationLogStorage::new(db.clone()))),
            Some(Arc::new(RocksDbCheckpointIngestor::new(db.clone()))),
            tantivy_receiver, // Enable Tantivy index ingestion
            hnsw_receiver,    // Enable HNSW index ingestion
            None,             // checkpoint_threshold - use default value
        );

        // Execute catch-up protocol
        match catch_up.execute_full_catch_up().await {
            Ok(result) => {
                tracing::info!(
                    "Catch-up completed successfully: {} files transferred ({} bytes), {} operations applied",
                    result.checkpoint_result.num_files,
                    result.checkpoint_result.total_bytes,
                    result.verification_result.operations_applied
                );
            }
            Err(e) => {
                tracing::warn!("Catch-up failed (will rely on steady-state sync): {}", e);
                // Don't fail startup - steady-state sync will eventually catch up
            }
        }
    } else {
        tracing::info!("Node is up-to-date, skipping catch-up protocol");
    }

    Ok(coordinator)
}
