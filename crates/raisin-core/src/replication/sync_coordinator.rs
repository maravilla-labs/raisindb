//! Synchronization coordinator for managing periodic peer synchronization
//!
//! The SyncCoordinator is responsible for scheduling and executing periodic
//! synchronization jobs for all configured peers.

use crate::replication::PeerRegistry;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// Tracks the synchronization state for a peer
#[derive(Debug, Clone)]
pub struct PeerSyncState {
    /// Last successful synchronization timestamp
    pub last_sync_at: Option<Instant>,

    /// Last attempted synchronization timestamp
    pub last_attempt_at: Option<Instant>,

    /// Number of consecutive failures
    pub consecutive_failures: usize,

    /// Last error message (if any)
    pub last_error: Option<String>,

    /// Current backoff delay in seconds (for exponential backoff)
    pub current_backoff_secs: u64,
}

impl PeerSyncState {
    /// Create a new sync state
    pub fn new() -> Self {
        Self {
            last_sync_at: None,
            last_attempt_at: None,
            consecutive_failures: 0,
            last_error: None,
            current_backoff_secs: 0,
        }
    }

    /// Record a successful synchronization
    pub fn record_success(&mut self) {
        self.last_sync_at = Some(Instant::now());
        self.last_attempt_at = Some(Instant::now());
        self.consecutive_failures = 0;
        self.last_error = None;
        self.current_backoff_secs = 0;
    }

    /// Record a failed synchronization
    pub fn record_failure(&mut self, error: String, initial_backoff: u64, max_backoff: u64) {
        self.last_attempt_at = Some(Instant::now());
        self.consecutive_failures += 1;
        self.last_error = Some(error);

        // Exponential backoff: 2^failures * initial_backoff, capped at max_backoff
        self.current_backoff_secs = if self.consecutive_failures == 1 {
            initial_backoff
        } else {
            (self.current_backoff_secs * 2).min(max_backoff)
        };
    }

    /// Check if enough time has passed since last attempt to try again
    pub fn should_retry(&self, sync_interval_secs: u64) -> bool {
        let Some(last_attempt) = self.last_attempt_at else {
            // Never attempted, should try
            return true;
        };

        let elapsed = last_attempt.elapsed().as_secs();

        // Use backoff delay if we have failures, otherwise use sync interval
        let required_delay = if self.consecutive_failures > 0 {
            self.current_backoff_secs
        } else {
            sync_interval_secs
        };

        elapsed >= required_delay
    }
}

impl Default for PeerSyncState {
    fn default() -> Self {
        Self::new()
    }
}

/// Coordinator for managing periodic synchronization with peers
pub struct SyncCoordinator {
    /// Peer registry
    peer_registry: Arc<PeerRegistry>,

    /// Per-peer synchronization state
    sync_states: Arc<RwLock<HashMap<String, PeerSyncState>>>,
}

impl SyncCoordinator {
    /// Create a new synchronization coordinator
    pub fn new(peer_registry: Arc<PeerRegistry>) -> Self {
        Self {
            peer_registry,
            sync_states: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get the synchronization state for a peer
    pub fn get_sync_state(&self, peer_id: &str) -> Option<PeerSyncState> {
        let states = self.sync_states.read().expect("sync state lock poisoned");
        states.get(peer_id).cloned()
    }

    /// Get synchronization states for all peers
    pub fn get_all_sync_states(&self) -> HashMap<String, PeerSyncState> {
        let states = self.sync_states.read().expect("sync state lock poisoned");
        states.clone()
    }

    /// Record a successful synchronization for a peer
    pub fn record_success(&self, peer_id: &str) {
        let mut states = self.sync_states.write().expect("sync state lock poisoned");
        states
            .entry(peer_id.to_string())
            .or_default()
            .record_success();

        info!(peer_id = %peer_id, "Peer synchronization successful");
    }

    /// Record a failed synchronization for a peer
    pub fn record_failure(&self, peer_id: &str, error: String) {
        let peer_config = self.peer_registry.get_peer(peer_id);
        let (initial_backoff, max_backoff) = peer_config
            .as_ref()
            .map(|c| {
                (
                    c.retry_config.initial_backoff_secs,
                    c.retry_config.max_backoff_secs,
                )
            })
            .unwrap_or((5, 300));

        let mut states = self.sync_states.write().expect("sync state lock poisoned");
        let state = states.entry(peer_id.to_string()).or_default();

        state.record_failure(error.clone(), initial_backoff, max_backoff);

        warn!(
            peer_id = %peer_id,
            consecutive_failures = state.consecutive_failures,
            backoff_secs = state.current_backoff_secs,
            error = %error,
            "Peer synchronization failed"
        );
    }

    /// Get list of peers that are ready for synchronization
    ///
    /// A peer is ready if:
    /// - It is enabled
    /// - Enough time has passed since last attempt (considering backoff)
    pub fn get_peers_ready_for_sync(&self) -> Vec<String> {
        let enabled_peers = self.peer_registry.list_enabled_peers();
        let states = self.sync_states.read().expect("sync state lock poisoned");

        enabled_peers
            .iter()
            .filter_map(|peer| {
                let state = states.get(&peer.peer_id).cloned().unwrap_or_default();

                if state.should_retry(peer.sync_interval_secs) {
                    Some(peer.peer_id.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get health status for all peers
    pub fn get_health_status(&self) -> HashMap<String, PeerHealthStatus> {
        let states = self.sync_states.read().expect("sync state lock poisoned");
        let peers = self.peer_registry.list_peers();

        let mut status_map = HashMap::new();

        for peer in peers {
            let state = states.get(&peer.peer_id).cloned().unwrap_or_default();

            let status = if !peer.enabled {
                PeerHealthStatus::Disabled
            } else if state.consecutive_failures == 0 {
                PeerHealthStatus::Healthy {
                    last_sync_elapsed_secs: state.last_sync_at.map(|t| t.elapsed().as_secs()),
                }
            } else if state.consecutive_failures < 3 {
                PeerHealthStatus::Degraded {
                    consecutive_failures: state.consecutive_failures,
                    last_error: state.last_error.clone(),
                }
            } else {
                PeerHealthStatus::Unhealthy {
                    consecutive_failures: state.consecutive_failures,
                    last_error: state.last_error.clone(),
                    backoff_secs: state.current_backoff_secs,
                }
            };

            status_map.insert(peer.peer_id.clone(), status);
        }

        status_map
    }

    /// Start a background task to periodically check for sync opportunities
    ///
    /// Returns a cancellation token that can be used to stop the background task
    pub async fn start_background_sync<F, Fut>(
        self: Arc<Self>,
        check_interval_secs: u64,
        sync_callback: F,
    ) -> tokio::task::JoinHandle<()>
    where
        F: Fn(String, String, String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), String>> + Send + 'static,
    {
        let sync_callback = Arc::new(sync_callback);

        tokio::spawn(async move {
            info!(
                check_interval_secs = check_interval_secs,
                "Starting background sync coordinator"
            );

            let mut interval = tokio::time::interval(Duration::from_secs(check_interval_secs));

            loop {
                interval.tick().await;

                let ready_peers = self.get_peers_ready_for_sync();

                if ready_peers.is_empty() {
                    debug!("No peers ready for synchronization");
                    continue;
                }

                info!(
                    ready_peer_count = ready_peers.len(),
                    "Found peers ready for synchronization"
                );

                for peer_id in ready_peers {
                    let peer_config = match self.peer_registry.get_peer(&peer_id) {
                        Some(config) => config,
                        None => {
                            warn!(peer_id = %peer_id, "Peer not found in registry");
                            continue;
                        }
                    };

                    // TODO: Make tenant_id and repo_id configurable per peer
                    // For now, use placeholder values
                    let tenant_id = "default".to_string();
                    let repo_id = "main".to_string();

                    let callback = Arc::clone(&sync_callback);
                    let coordinator = Arc::clone(&self);

                    // Spawn a separate task for each peer sync
                    tokio::spawn(async move {
                        debug!(peer_id = %peer_id, "Starting synchronization");

                        match callback(peer_id.clone(), tenant_id, repo_id).await {
                            Ok(()) => {
                                coordinator.record_success(&peer_id);
                            }
                            Err(error) => {
                                coordinator.record_failure(&peer_id, error);
                            }
                        }
                    });
                }
            }
        })
    }
}

/// Health status for a peer
#[derive(Debug, Clone)]
pub enum PeerHealthStatus {
    /// Peer is disabled
    Disabled,

    /// Peer is healthy (no failures)
    Healthy {
        /// Seconds since last successful sync
        last_sync_elapsed_secs: Option<u64>,
    },

    /// Peer is degraded (1-2 failures)
    Degraded {
        /// Number of consecutive failures
        consecutive_failures: usize,
        /// Last error message
        last_error: Option<String>,
    },

    /// Peer is unhealthy (3+ failures)
    Unhealthy {
        /// Number of consecutive failures
        consecutive_failures: usize,
        /// Last error message
        last_error: Option<String>,
        /// Current backoff delay in seconds
        backoff_secs: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replication::PeerConfig;

    #[test]
    fn test_sync_state_success() {
        let mut state = PeerSyncState::new();

        state.record_success();

        assert!(state.last_sync_at.is_some());
        assert_eq!(state.consecutive_failures, 0);
        assert_eq!(state.current_backoff_secs, 0);
        assert!(state.last_error.is_none());
    }

    #[test]
    fn test_sync_state_failure_backoff() {
        let mut state = PeerSyncState::new();

        state.record_failure("error 1".to_string(), 5, 300);
        assert_eq!(state.consecutive_failures, 1);
        assert_eq!(state.current_backoff_secs, 5);

        state.record_failure("error 2".to_string(), 5, 300);
        assert_eq!(state.consecutive_failures, 2);
        assert_eq!(state.current_backoff_secs, 10);

        state.record_failure("error 3".to_string(), 5, 300);
        assert_eq!(state.consecutive_failures, 3);
        assert_eq!(state.current_backoff_secs, 20);

        // Reset on success
        state.record_success();
        assert_eq!(state.consecutive_failures, 0);
        assert_eq!(state.current_backoff_secs, 0);
    }

    #[test]
    fn test_coordinator_basic() {
        let registry = Arc::new(PeerRegistry::new());
        registry.add_peer(PeerConfig::new("peer1", "http://peer1:8080"));

        let coordinator = SyncCoordinator::new(registry.clone());

        // Initially, peer should be ready
        let ready = coordinator.get_peers_ready_for_sync();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0], "peer1");

        // Record success
        coordinator.record_success("peer1");

        // Get health status
        let health = coordinator.get_health_status();
        assert!(matches!(
            health.get("peer1"),
            Some(PeerHealthStatus::Healthy { .. })
        ));
    }

    #[test]
    fn test_coordinator_failure_handling() {
        let registry = Arc::new(PeerRegistry::new());
        registry.add_peer(PeerConfig::new("peer1", "http://peer1:8080"));

        let coordinator = SyncCoordinator::new(registry);

        // Record multiple failures
        coordinator.record_failure("peer1", "error 1".to_string());
        coordinator.record_failure("peer1", "error 2".to_string());

        let health = coordinator.get_health_status();
        assert!(matches!(
            health.get("peer1"),
            Some(PeerHealthStatus::Degraded {
                consecutive_failures: 2,
                ..
            })
        ));

        // Record more failures
        coordinator.record_failure("peer1", "error 3".to_string());

        let health = coordinator.get_health_status();
        assert!(matches!(
            health.get("peer1"),
            Some(PeerHealthStatus::Unhealthy {
                consecutive_failures: 3,
                ..
            })
        ));
    }
}
