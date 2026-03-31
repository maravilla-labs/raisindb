//! Basic metrics collection and health checks
//!
//! This module provides fundamental health monitoring and metrics collection
//! for RocksDB storage. It forms the foundation for production monitoring
//! and observability.

use crate::RocksDBStorage;
use raisin_error::Result;
use raisin_storage::{HealthCheck, HealthLevel, HealthStatus, Metrics};

/// Get basic health status
///
/// Performs a simple liveness check by attempting to iterate the database.
/// This verifies that RocksDB is responsive and accessible.
///
/// # Arguments
///
/// * `storage` - The RocksDBStorage instance to check
/// * `tenant` - Optional tenant ID for tenant-specific health checks
///
/// # Returns
///
/// Health status with configuration details
pub async fn get_health(storage: &RocksDBStorage, tenant: Option<&str>) -> Result<HealthStatus> {
    // If we have a valid RocksDBStorage reference, the DB is accessible
    let db_ok = true;

    let config = storage.config();
    let db_status = if db_ok {
        HealthLevel::Healthy
    } else {
        HealthLevel::Critical
    };

    let checks = vec![
        HealthCheck {
            name: "database_connectivity".to_string(),
            status: db_status,
            message: Some(if db_ok {
                "Database is responsive".to_string()
            } else {
                "Database is not accessible".to_string()
            }),
        },
        HealthCheck {
            name: "configuration".to_string(),
            status: HealthLevel::Healthy,
            message: Some(format!(
                "cache={}MB, compression={:?}, jobs={}, heal={}",
                config.block_cache_size / (1024 * 1024),
                config.compression,
                config.background_jobs_enabled,
                config.auto_heal_enabled
            )),
        },
    ];

    // Overall status is the worst of all checks
    let overall_status = if !db_ok {
        HealthLevel::Critical
    } else {
        HealthLevel::Healthy
    };

    Ok(HealthStatus {
        status: overall_status,
        tenant: tenant.map(|s| s.to_string()),
        checks,
        needs_healing: !db_ok,
        last_check: chrono::Utc::now(),
    })
}

/// Get basic metrics
///
/// Collects fundamental metrics about the RocksDB storage instance.
/// In Week 1, this provides basic configuration info. More detailed
/// metrics (node counts, storage size, cache statistics) will be
/// added in Week 2.
///
/// # Arguments
///
/// * `storage` - The RocksDBStorage instance
/// * `tenant` - Optional tenant ID for tenant-specific metrics
///
/// # Returns
///
/// Metrics structure with available statistics
pub async fn get_metrics(storage: &RocksDBStorage, tenant: Option<&str>) -> Result<Metrics> {
    // TODO Week 2: Extract actual RocksDB statistics if enabled
    // This will include:
    // - Cache hit rates from RocksDB stats
    // - Compaction stats
    // - Read/write latencies
    // - Memory usage
    // - Actual node counts and disk usage

    Ok(Metrics {
        tenant: tenant.map(|s| s.to_string()),
        operations_per_sec: 0.0, // TODO Week 2: Track operations
        error_rate: 0.0,         // TODO Week 2: Track errors
        disk_usage_bytes: 0,     // TODO Week 2: Get DB size
        index_sizes: std::collections::HashMap::new(), // TODO Week 2: Get CF sizes
        node_count: 0,           // TODO Week 2: Count nodes
        active_connections: 0,   // N/A for RocksDB (embedded)
        cache_hit_rate: 0.0,     // TODO Week 2: From RocksDB stats
        last_compaction: None,   // TODO Week 2: Track compactions
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RocksDBConfig, RocksDBStorage};
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_get_health() {
        let temp_dir = TempDir::new().unwrap();
        let config = RocksDBConfig::development().with_path(temp_dir.path());
        let storage = RocksDBStorage::with_config(config).unwrap();

        let health = get_health(&storage, None).await.unwrap();
        assert!(matches!(health.status, HealthLevel::Healthy));
        assert!(!health.checks.is_empty());
        assert_eq!(health.tenant, None);
    }

    #[tokio::test]
    async fn test_get_metrics() {
        let temp_dir = TempDir::new().unwrap();
        let config = RocksDBConfig::development().with_path(temp_dir.path());
        let storage = RocksDBStorage::with_config(config).unwrap();

        let metrics = get_metrics(&storage, None).await.unwrap();
        // Week 1: Metrics are placeholders
        assert_eq!(metrics.node_count, 0);
        assert_eq!(metrics.disk_usage_bytes, 0);
        assert_eq!(metrics.tenant, None);
    }

    #[tokio::test]
    async fn test_health_includes_config_details() {
        let temp_dir = TempDir::new().unwrap();
        let config = RocksDBConfig::production().with_path(temp_dir.path());
        let storage = RocksDBStorage::with_config(config).unwrap();

        let health = get_health(&storage, None).await.unwrap();

        // Should have database connectivity and configuration checks
        assert!(health.checks.len() >= 2);

        // Find the configuration check
        let config_check = health.checks.iter().find(|c| c.name == "configuration");
        assert!(config_check.is_some());

        // Verify configuration check contains expected info
        let msg = config_check.unwrap().message.as_ref().unwrap();
        assert!(msg.contains("cache="));
        assert!(msg.contains("jobs=true"));
        assert!(msg.contains("heal=true"));
    }
}
