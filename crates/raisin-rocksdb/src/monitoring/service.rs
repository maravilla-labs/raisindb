//! Production monitoring service implementation

use super::*;
use crate::RocksDBStorage;
use raisin_replication::ReplicationCoordinator;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::interval;

/// Production monitoring service
///
/// Aggregates metrics from all replication components and provides:
/// - Periodic logging
/// - JSON export for external monitoring
/// - Health checks
/// - HTTP endpoint (optional)
pub struct MonitoringService {
    storage: Arc<RocksDBStorage>,
    coordinator: Option<Arc<ReplicationCoordinator>>,
    start_time: SystemTime,
    state: Arc<RwLock<MonitoringState>>,
}

#[derive(Debug)]
struct MonitoringState {
    enabled: bool,
    last_snapshot: Option<ReplicationMetricsSnapshot>,
}

impl MonitoringService {
    /// Create a new monitoring service
    ///
    /// # Arguments
    /// * `storage` - RocksDB storage instance
    /// * `coordinator` - Optional replication coordinator
    pub fn new(
        storage: Arc<RocksDBStorage>,
        coordinator: Option<Arc<ReplicationCoordinator>>,
    ) -> Self {
        Self {
            storage,
            coordinator,
            start_time: SystemTime::now(),
            state: Arc::new(RwLock::new(MonitoringState {
                enabled: false,
                last_snapshot: None,
            })),
        }
    }

    /// Start periodic metrics logging
    ///
    /// # Arguments
    /// * `interval_duration` - How often to log metrics (e.g., 30 seconds)
    ///
    /// # Returns
    /// Task handle that can be used to cancel logging
    pub fn start_periodic_logging(
        &self,
        interval_duration: Duration,
    ) -> tokio::task::JoinHandle<()> {
        let service = self.clone_for_task();

        tokio::spawn(async move {
            let mut ticker = interval(interval_duration);

            loop {
                ticker.tick().await;

                match service.collect_metrics().await {
                    Ok(snapshot) => {
                        Self::log_metrics(&snapshot);

                        // Update state with latest snapshot
                        let mut state = service.state.write().await;
                        state.last_snapshot = Some(snapshot);
                    }
                    Err(e) => {
                        warn!("Failed to collect metrics: {}", e);
                    }
                }
            }
        })
    }

    /// Export metrics as JSON string
    ///
    /// # Returns
    /// JSON string containing current metrics snapshot
    pub async fn export_json(&self) -> Result<String> {
        let snapshot = self.collect_metrics().await?;
        serde_json::to_string_pretty(&snapshot).map_err(|e| {
            raisin_error::Error::encoding(format!("Failed to serialize metrics: {}", e))
        })
    }

    /// Export metrics as JSON value (for direct API responses)
    ///
    /// # Returns
    /// JSON value containing current metrics snapshot
    pub async fn export_json_value(&self) -> Result<serde_json::Value> {
        let snapshot = self.collect_metrics().await?;
        serde_json::to_value(&snapshot).map_err(|e| {
            raisin_error::Error::encoding(format!("Failed to serialize metrics: {}", e))
        })
    }

    /// Check if the replication system is healthy
    ///
    /// # Returns
    /// `true` if all health checks pass
    pub async fn is_healthy(&self) -> bool {
        match self.collect_metrics().await {
            Ok(snapshot) => snapshot.check_health() == HealthStatus::Healthy,
            Err(_) => false,
        }
    }

    /// Get current metrics snapshot
    pub async fn get_snapshot(&self) -> Result<ReplicationMetricsSnapshot> {
        self.collect_metrics().await
    }

    /// Collect metrics from all components
    async fn collect_metrics(&self) -> Result<ReplicationMetricsSnapshot> {
        let now = SystemTime::now();
        let timestamp_ms = now
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let uptime_seconds = self.start_time.elapsed().map(|d| d.as_secs()).unwrap_or(0);

        // Collect causal buffer metrics (placeholder - would integrate with actual buffer)
        let causal_buffer = CausalBufferMetrics {
            current_size: 0,
            max_size: 1000,
            utilization_percent: 0.0,
            operations_delivered: 0,
            direct_deliveries: 0,
            operations_buffered: 0,
            avg_delivery_lag_ms: 0.0,
            buffer_full_events: 0,
            missing_dependencies: 0,
        };

        // Collect idempotency metrics (placeholder - would integrate with actual tracker)
        let idempotency = IdempotencyMetrics {
            operations_tracked: 0,
            hits: 0,
            misses: 0,
            hit_rate_percent: 0.0,
            duplicates_prevented: 0,
            storage_bytes: None,
            avg_lookup_latency_us: 0.0,
        };

        let snapshot = ReplicationMetricsSnapshot {
            timestamp_ms,
            uptime_seconds,
            causal_buffer,
            idempotency,
            health: HealthStatus::Healthy,
        };

        let health = snapshot.check_health();
        Ok(ReplicationMetricsSnapshot { health, ..snapshot })
    }

    /// Log metrics in a structured format
    fn log_metrics(snapshot: &ReplicationMetricsSnapshot) {
        info!(
            "=== Replication Metrics (uptime: {}s, health: {:?}) ===",
            snapshot.uptime_seconds, snapshot.health
        );

        Self::log_causal_buffer(&snapshot.causal_buffer);
        Self::log_idempotency(&snapshot.idempotency);

        // Health warnings
        match snapshot.health {
            HealthStatus::Degraded => warn!("System health: DEGRADED - review metrics"),
            HealthStatus::Unhealthy => {
                warn!("System health: UNHEALTHY - immediate action required")
            }
            _ => {}
        }
    }

    fn log_causal_buffer(metrics: &CausalBufferMetrics) {
        info!("--- Causal Delivery Buffer ---");
        info!(
            "  Size: {}/{} ({:.1}% utilization)",
            metrics.current_size, metrics.max_size, metrics.utilization_percent
        );
        info!(
            "  Operations: {} delivered ({} direct), {} buffered",
            metrics.operations_delivered, metrics.direct_deliveries, metrics.operations_buffered
        );
        info!("  Avg delivery lag: {:.1}ms", metrics.avg_delivery_lag_ms);

        if metrics.buffer_full_events > 0 {
            warn!(
                "  Buffer full events: {} (operations may have been dropped)",
                metrics.buffer_full_events
            );
        }
        if metrics.missing_dependencies > 0 {
            warn!(
                "  Missing dependencies: {} operations waiting",
                metrics.missing_dependencies
            );
        }
    }

    fn log_idempotency(metrics: &IdempotencyMetrics) {
        info!("--- Idempotency Tracker ---");
        info!(
            "  Operations tracked: {} (hit rate: {:.1}%)",
            metrics.operations_tracked, metrics.hit_rate_percent
        );
        info!(
            "  Hits: {}, Misses: {}, Duplicates prevented: {}",
            metrics.hits, metrics.misses, metrics.duplicates_prevented
        );
        info!(
            "  Avg lookup latency: {:.1}μs",
            metrics.avg_lookup_latency_us
        );

        if let Some(storage_bytes) = metrics.storage_bytes {
            info!(
                "  Storage size: {:.2} MB",
                storage_bytes as f64 / 1_048_576.0
            );
        }
    }

    fn clone_for_task(&self) -> Self {
        Self {
            storage: self.storage.clone(),
            coordinator: self.coordinator.clone(),
            start_time: self.start_time,
            state: self.state.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_monitoring_service_creation() {
        // This would require actual storage instance, so we just test the API
        // In production, you'd create MonitoringService with real storage
    }
}
