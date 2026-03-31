//! Tests for RocksDB configuration.

use super::*;

#[test]
fn test_development_preset() {
    let config = RocksDBConfig::development();
    assert_eq!(config.block_cache_size, 128 * 1024 * 1024);
    assert!(!config.background_jobs_enabled);
    assert!(!config.auto_heal_enabled);
}

#[test]
fn test_production_preset() {
    let config = RocksDBConfig::production();
    assert_eq!(config.block_cache_size, 512 * 1024 * 1024);
    assert!(config.background_jobs_enabled);
    assert!(config.auto_heal_enabled);
    assert!(config.enable_statistics);
}

#[test]
fn test_high_performance_preset() {
    let config = RocksDBConfig::high_performance();
    assert_eq!(config.block_cache_size, 2048 * 1024 * 1024);
    assert_eq!(config.max_background_compactions, 8);
    assert!(matches!(config.compression, CompressionType::Lz4));
}

#[test]
fn test_with_path() {
    let config = RocksDBConfig::development().with_path("/custom/path");
    assert_eq!(config.path, PathBuf::from("/custom/path"));
}

#[test]
fn test_tenant_limits() {
    let mut config = RocksDBConfig::development();
    let limits = TenantLimits {
        max_storage_bytes: Some(1_000_000),
        max_nodes: Some(10_000),
        max_ops_per_second: Some(100),
        max_memory_bytes: Some(500_000),
    };

    config.set_tenant_limit("tenant1".to_string(), limits.clone());

    let retrieved = config.get_tenant_limit("tenant1").unwrap();
    assert_eq!(retrieved.max_storage_bytes, Some(1_000_000));
    assert_eq!(retrieved.max_nodes, Some(10_000));
}

// Note: merge_uint64_add is tested indirectly through RocksDB's merge operator
// integration tests. Direct unit testing is not possible because MergeOperands::new
// is a private API in the rocksdb crate.

#[test]
fn test_compression_type_conversion() {
    assert_eq!(
        CompressionType::Snappy.to_rocksdb(),
        rocksdb::DBCompressionType::Snappy
    );
    assert_eq!(
        CompressionType::Lz4.to_rocksdb(),
        rocksdb::DBCompressionType::Lz4
    );
}
