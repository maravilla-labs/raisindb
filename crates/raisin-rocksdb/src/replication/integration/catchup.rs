//! Catch-up assessment logic for replication startup.
//!
//! Determines whether a node needs to perform an initial catch-up from peers
//! by inspecting its local operation log and comparing vector clocks with
//! connected peers.

use std::sync::Arc;

use raisin_replication::{CoordinatorError, OperationLogStorage, ReplicationCoordinator};
use raisin_storage::Storage;

use crate::RocksDBStorage;

use super::oplog_storage::RocksDbOperationLogStorage;

/// Rich assessment of whether the node needs an initial catch-up
#[derive(Debug)]
pub(super) struct CatchUpAssessment {
    /// Whether automatic catch-up should run
    pub requires_catch_up: bool,
    /// Total tenant/repo pairs encountered (including `_registry`)
    pub total_pairs: usize,
    /// Number of `_registry` repositories skipped
    pub skipped_registry: usize,
    /// Repositories without operations (excluding `_registry`)
    pub empty_pairs: Vec<String>,
    /// Repositories that already contain operations
    pub non_empty_pairs: Vec<String>,
}

/// Largest observed divergence between local state and any peer
pub(super) struct PeerLagFinding {
    pub peer_node_id: String,
    pub configured_peer_id: String,
    pub lag_distance: u64,
}

/// Inspect the local operation log and decide if a catch-up run is required
pub(super) async fn assess_catchup_need(
    db: &Arc<RocksDBStorage>,
    cluster_config: &raisin_replication::ClusterConfig,
) -> CatchUpAssessment {
    // Enumerate ALL tenant/repo pairs from the database, not just configured ones
    let tenant_repos = match super::enumerate_all_tenant_repos(db).await {
        Ok(pairs) => pairs,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "Failed to enumerate tenant/repo pairs, falling back to configured pairs"
            );
            // Fallback to configured pairs if enumeration fails
            cluster_config.sync_tenants.clone()
        }
    };

    let mut assessment = CatchUpAssessment {
        requires_catch_up: true,
        total_pairs: tenant_repos.len(),
        skipped_registry: 0,
        empty_pairs: Vec::new(),
        non_empty_pairs: Vec::new(),
    };

    // If no tenant/repo pairs exist at all, this is definitely a fresh node
    if tenant_repos.is_empty() {
        tracing::info!("No tenant/repo pairs found in database - fresh node detected");
        return assessment;
    }

    let oplog_repo = crate::repositories::OpLogRepository::new(db.db().clone());

    tracing::debug!(
        num_pairs = tenant_repos.len(),
        "Checking vector clocks for all tenant/repo pairs to determine if catch-up needed"
    );

    for (tenant_id, repo_id) in &tenant_repos {
        // CRITICAL: Skip _registry pseudo-repository for fresh node detection
        // _registry operations are created during every node startup,
        // so they can't indicate whether a node has real data
        if repo_id == "_registry" {
            assessment.skipped_registry += 1;
            tracing::debug!(
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                "Skipping _registry for fresh node detection (created during startup)"
            );
            continue;
        }

        // Get local vector clock for this tenant/repo
        match oplog_repo.get_vector_clock_snapshot(tenant_id, repo_id) {
            Ok(vc) => {
                if !vc.is_empty() {
                    // Have operations in a REAL repository
                    assessment
                        .non_empty_pairs
                        .push(format!("({}, {})", tenant_id, repo_id));
                    tracing::info!(
                        tenant_id = %tenant_id,
                        repo_id = %repo_id,
                        vc_entries = vc.len(),
                        "Found existing operations in real repository"
                    );
                } else {
                    assessment
                        .empty_pairs
                        .push(format!("({}, {})", tenant_id, repo_id));
                }
            }
            Err(e) => {
                tracing::debug!(
                    tenant_id = %tenant_id,
                    repo_id = %repo_id,
                    error = %e,
                    "Failed to get vector clock (treating as empty)"
                );
                assessment
                    .empty_pairs
                    .push(format!("({}, {}) [error]", tenant_id, repo_id));
            }
        }
    }

    assessment.requires_catch_up = assessment.non_empty_pairs.is_empty();

    if assessment.requires_catch_up {
        tracing::info!(
            total_pairs = assessment.total_pairs,
            skipped_registry = assessment.skipped_registry,
            checked_pairs = assessment.empty_pairs.len(),
            empty_pairs = ?assessment.empty_pairs,
            "All real repositories have no operations - fresh node detected (excluding _registry)"
        );
    } else {
        tracing::info!(
            total_pairs = assessment.total_pairs,
            skipped_registry = assessment.skipped_registry,
            repositories_with_data = assessment.non_empty_pairs.len(),
            example_repo = %assessment
                .non_empty_pairs
                .first()
                .cloned()
                .unwrap_or_default(),
            "Existing operations found - skipping automatic catch-up"
        );
    }

    assessment
}

/// Determine if any connected peer's vector clock is ahead of the local node
pub(super) async fn assess_peer_divergence(
    storage: &Arc<RocksDbOperationLogStorage>,
    coordinator: &Arc<ReplicationCoordinator>,
) -> Result<Option<PeerLagFinding>, CoordinatorError> {
    let local_stats = storage
        .get_cluster_stats()
        .await
        .map_err(|e| CoordinatorError::Storage(e.to_string()))?;

    let peer_snapshots = coordinator.get_peer_cluster_snapshots().await;

    if peer_snapshots.is_empty() {
        tracing::debug!("No peer cluster snapshots available for divergence assessment");
        return Ok(None);
    }

    let mut lagging_peer: Option<PeerLagFinding> = None;

    for snapshot in peer_snapshots {
        let ordering = local_stats.max_vector_clock.compare(&snapshot.vector_clock);
        let lag_distance = local_stats
            .max_vector_clock
            .distance(&snapshot.vector_clock);

        tracing::debug!(
            configured_peer_id = %snapshot.configured_peer_id,
            peer_node_id = %snapshot.node_id,
            peer_log_index = snapshot.log_index,
            ordering = ?ordering,
            lag_distance = lag_distance,
            "Peer cluster snapshot compared against local vector clock"
        );

        if lag_distance > 0 {
            let entry = PeerLagFinding {
                peer_node_id: snapshot.node_id.clone(),
                configured_peer_id: snapshot.configured_peer_id.clone(),
                lag_distance: lag_distance.max(1),
            };

            match lagging_peer {
                Some(ref current) if current.lag_distance >= entry.lag_distance => {}
                _ => lagging_peer = Some(entry),
            }
        }
    }

    Ok(lagging_peer)
}
