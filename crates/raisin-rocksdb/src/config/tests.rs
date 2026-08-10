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

/// The column-family options must actually reach the column families.
///
/// This is the regression test for the defect that made this whole file's
/// tuning inert: `to_rocksdb_options()` was passed to `DB::open_cf_descriptors`
/// as the DB options, and RocksDB takes CF-scoped settings *exclusively* from
/// the descriptors — so the block cache, `cache_index_and_filter_blocks`, the
/// write buffers and periodic compaction were all silently discarded for every
/// one of the ~49 column families. Nothing failed; the settings simply did
/// nothing, for years.
///
/// The observable proof is where the index and filter blocks live. With
/// `cache_index_and_filter_blocks` enabled they are charged to the block cache
/// and `rocksdb.estimate-table-readers-mem` stays near zero. With it disabled
/// — the state this database was in — they are pinned in the table reader
/// instead: outside any cache, evicted by nothing, growing with SST count for
/// the life of the process. That is precisely the slow RSS creep this was
/// chasing, so the assertion below is on table-readers memory staying small.
#[test]
fn index_and_filter_blocks_are_charged_to_the_block_cache() {
    use crate::cf;

    let dir = tempfile::tempdir().unwrap();
    let config = RocksDBConfig::development().with_path(dir.path());
    let db = crate::open_db_with_config(&config).unwrap();

    // Write and flush so there is a real SST with index and filter blocks.
    let handle = db.cf_handle(cf::NODES).unwrap();
    for i in 0..2_000u32 {
        db.put_cf(&handle, format!("key-{i:06}"), vec![b'v'; 256])
            .unwrap();
    }
    db.flush_cf(&handle).unwrap();

    // Read them back, forcing the index/filter blocks to be loaded.
    for i in 0..2_000u32 {
        assert!(db.get_cf(&handle, format!("key-{i:06}")).unwrap().is_some());
    }

    let prop = |name: &str| {
        db.property_int_value_cf(&handle, name)
            .unwrap()
            .unwrap_or(0)
    };

    let cache_usage = prop("rocksdb.block-cache-usage");
    let table_readers = prop("rocksdb.estimate-table-readers-mem");

    assert!(
        cache_usage > 0,
        "block cache usage was 0; the column family is not using the \
         configured cache, which means the CF options were dropped again"
    );
    assert!(
        cache_usage <= config.block_cache_size as u64,
        "block cache usage {cache_usage} exceeds the configured {} — the CF is \
         on a different cache than the one we sized",
        config.block_cache_size
    );
    // The discriminator. Measured on this exact fixture (2000 keys, one SST):
    //   cache_index_and_filter_blocks OFF → table_readers ≈ 7_248 bytes
    //   cache_index_and_filter_blocks ON  → table_readers ≈ 1_925 bytes
    // The delta is the index + filter, moved into the (bounded, evictable)
    // block cache instead of being pinned in the reader. The threshold sits
    // between the two; do not "fix" a failure here by raising it.
    //
    // Note the absolute numbers are small because this fixture has ONE SST.
    // In production it is multiplied by every SST across ~49 column families
    // and never evicted, which is what made it a multi-hundred-MB creep.
    const PINNED_METADATA_CEILING: u64 = 4_096;
    assert!(
        table_readers < PINNED_METADATA_CEILING,
        "estimate-table-readers-mem is {table_readers} (cache usage \
         {cache_usage}); above {PINNED_METADATA_CEILING} means index and filter \
         blocks are pinned outside the cache again, i.e. \
         cache_index_and_filter_blocks is off"
    );
}

/// Every column family must share ONE cache, not get a private default.
///
/// Three CFs (`PROPERTY_INDEX`, `SPATIAL_INDEX`, `UNIQUE_INDEX`) install their
/// own `BlockBasedOptions` for bloom filters. Building those from
/// `BlockBasedOptions::default()` instead of `block_table_options` silently
/// gives each one RocksDB's private 32MB cache — bounded, but invisible and
/// unaccounted. Sharing means they report the same usage figure.
#[test]
fn bloom_filter_column_families_share_the_same_cache() {
    use crate::cf;

    let dir = tempfile::tempdir().unwrap();
    let config = RocksDBConfig::development().with_path(dir.path());
    let db = crate::open_db_with_config(&config).unwrap();

    let nodes = db.cf_handle(cf::NODES).unwrap();
    for i in 0..1_000u32 {
        db.put_cf(&nodes, format!("key-{i:06}"), vec![b'v'; 256])
            .unwrap();
    }
    db.flush_cf(&nodes).unwrap();
    for i in 0..1_000u32 {
        db.get_cf(&nodes, format!("key-{i:06}")).unwrap();
    }

    let usage_of = |name: &str| {
        let handle = db.cf_handle(name).unwrap();
        db.property_int_value_cf(&handle, "rocksdb.block-cache-usage")
            .unwrap()
            .unwrap_or(0)
    };

    // A shared cache reports one global usage number through every handle. A
    // per-CF cache would report 0 for the CFs we never touched.
    let baseline = usage_of(cf::NODES);
    assert!(
        baseline > 0,
        "expected the write/read above to populate a cache"
    );

    for name in [cf::PROPERTY_INDEX, cf::SPATIAL_INDEX, cf::UNIQUE_INDEX] {
        assert_eq!(
            usage_of(name),
            baseline,
            "{name} reports different block cache usage than {}, so it is on a \
             private cache instead of the shared one",
            cf::NODES
        );
    }
}
