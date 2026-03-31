//! Replication performance and tail latency tests
//!
//! These tests measure replication performance metrics including:
//! - Latency distribution (P50, P90, P95, P99)
//! - Tail latency identification
//! - Throughput under load
//!
//! Run with: cargo test --package raisin-rocksdb --test replication_performance_test

use once_cell::sync::Lazy;
use raisin_replication::{ClusterConfig, ConnectionConfig, PeerConfig, SyncConfig};
use raisin_rocksdb::replication::start_replication;
use raisin_rocksdb::{OpLogRepository, RocksDBConfig, RocksDBStorage};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tracing_subscriber::{fmt, EnvFilter};

static TRACING_INIT: Lazy<()> = Lazy::new(|| {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,raisin_replication=info,raisin_rocksdb=info"));

    fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .try_init()
        .ok();
});

fn init_tracing() {
    Lazy::force(&TRACING_INIT);
}

/// Helper to create a test storage instance with replication enabled
fn create_replicated_storage(node_id: &str) -> (TempDir, Arc<RocksDBStorage>) {
    let temp_dir = TempDir::new().unwrap();
    let mut config = RocksDBConfig::default();
    config.path = temp_dir.path().to_path_buf();
    config.replication_enabled = true;
    config.cluster_node_id = Some(node_id.to_string());
    let storage = Arc::new(RocksDBStorage::with_config(config).unwrap());
    (temp_dir, storage)
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("failed to bind to ask OS for free port")
        .local_addr()
        .unwrap()
        .port()
}

fn unique_ports() -> (u16, u16) {
    loop {
        let p1 = free_port();
        let p2 = free_port();
        if p1 != p2 {
            return (p1, p2);
        }
    }
}

/// Helper to start replication coordinator for a node
async fn start_node_replication(
    storage: Arc<RocksDBStorage>,
    node_id: &str,
    port: u16,
    peer_configs: Vec<PeerConfig>,
) -> Arc<raisin_replication::ReplicationCoordinator> {
    let cluster_config = ClusterConfig {
        node_id: node_id.to_string(),
        replication_port: port,
        bind_address: "127.0.0.1".to_string(),
        peers: peer_configs,
        sync: SyncConfig {
            interval_seconds: 1,
            batch_size: 100,
            realtime_push: true,
            ..Default::default()
        },
        connection: ConnectionConfig {
            heartbeat_interval_seconds: 300,
            connect_timeout_seconds: 5,
            read_timeout_seconds: 10,
            write_timeout_seconds: 10,
            max_connections_per_peer: 4,
            keepalive_seconds: 60,
        },
        sync_tenants: vec![("tenant1".to_string(), "repo1".to_string())],
    };

    start_replication(storage, cluster_config).await.unwrap()
}

/// Wait for a specific number of operations from a node
async fn wait_for_operations(
    storage: &Arc<RocksDBStorage>,
    tenant_id: &str,
    repo_id: &str,
    node_id: &str,
    expected_count: usize,
    timeout: Duration,
) -> Result<Duration, String> {
    let start = Instant::now();

    loop {
        let oplog = OpLogRepository::new(storage.db().clone());
        match oplog.get_operations_from_node(tenant_id, repo_id, node_id) {
            Ok(ops) if ops.len() >= expected_count => {
                return Ok(start.elapsed());
            }
            Ok(_) => {
                if start.elapsed() > timeout {
                    return Err(format!(
                        "Timeout after {:?}: expected {} ops from {}, got less",
                        timeout, expected_count, node_id
                    ));
                }
            }
            Err(e) => {
                if start.elapsed() > timeout {
                    return Err(format!("Error reading operations: {}", e));
                }
            }
        }

        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Calculate percentiles from sorted samples
fn calculate_percentiles(mut samples: Vec<Duration>) -> PercentileStats {
    samples.sort();
    let len = samples.len();

    let p50_idx = (len as f64 * 0.50) as usize;
    let p90_idx = (len as f64 * 0.90) as usize;
    let p95_idx = (len as f64 * 0.95) as usize;
    let p99_idx = (len as f64 * 0.99) as usize;

    PercentileStats {
        count: len,
        min: samples[0],
        p50: samples[p50_idx.min(len - 1)],
        p90: samples[p90_idx.min(len - 1)],
        p95: samples[p95_idx.min(len - 1)],
        p99: samples[p99_idx.min(len - 1)],
        max: samples[len - 1],
        mean: Duration::from_nanos(
            (samples.iter().map(|d| d.as_nanos()).sum::<u128>() / len as u128) as u64,
        ),
    }
}

#[derive(Debug)]
struct PercentileStats {
    count: usize,
    min: Duration,
    p50: Duration,
    p90: Duration,
    p95: Duration,
    p99: Duration,
    max: Duration,
    mean: Duration,
}

impl PercentileStats {
    fn print_report(&self, label: &str) {
        println!(
            "\n📊 {} Latency Distribution ({} samples):",
            label, self.count
        );
        println!("  Min:  {:?}", self.min);
        println!("  P50:  {:?}", self.p50);
        println!("  P90:  {:?}", self.p90);
        println!("  P95:  {:?}", self.p95);
        println!("  P99:  {:?}", self.p99);
        println!("  Max:  {:?}", self.max);
        println!("  Mean: {:?}", self.mean);

        // Identify tail latency spikes (>10x P50)
        let spike_threshold = self.p50 * 10;
        if self.max > spike_threshold {
            println!(
                "\n⚠️  TAIL LATENCY SPIKE DETECTED: Max ({:?}) is >10x P50 ({:?})",
                self.max, self.p50
            );
        }

        // Identify significant P99 issues (>5x P50)
        if self.p99 > self.p50 * 5 {
            println!(
                "\n⚠️  P99 LATENCY ISSUE: P99 ({:?}) is >5x P50 ({:?})",
                self.p99, self.p50
            );
        }
    }
}

#[tokio::test]
#[ignore] // Exclude from normal test runs - run explicitly with --include-ignored
async fn test_replication_latency_distribution() {
    init_tracing();
    println!("\n🧪 Starting Replication Latency Distribution Test");
    println!("   Measuring P50, P90, P95, P99 latencies over 100 operations\n");

    // Create two nodes
    let (_dir1, storage1) = create_replicated_storage("node1");
    let (_dir2, storage2) = create_replicated_storage("node2");
    let (port1, port2) = unique_ports();

    let peer2 = PeerConfig::new("node2", "127.0.0.1").with_port(port2);
    let peer1 = PeerConfig::new("node1", "127.0.0.1").with_port(port1);

    println!("🚀 Starting coordinators...");
    let _coordinator1 = start_node_replication(storage1.clone(), "node1", port1, vec![peer2]).await;
    let _coordinator2 = start_node_replication(storage2.clone(), "node2", port2, vec![peer1]).await;

    // Wait for TCP servers and connections
    tokio::time::sleep(Duration::from_millis(200)).await;

    println!("📊 Creating 100 operations and measuring replication latency...\n");

    let mut latencies = Vec::with_capacity(100);
    let sample_count = 100;

    for i in 1..=sample_count {
        let start = Instant::now();

        // Create operation on node1
        storage1
            .operation_capture()
            .capture_create_node(
                "tenant1".to_string(),
                "repo1".to_string(),
                "main".to_string(),
                format!("perf-test-{}", i),
                format!("Performance Test Node {}", i),
                "Document".to_string(),
                None,
                None,
                "a".to_string(),
                serde_json::json!({"index": i, "test": "latency"}),
                None,
                None,
                format!("/Performance Test Node {}", i),
                "perftest".to_string(),
            )
            .await
            .unwrap();

        // Wait for replication to node2
        let replication_latency = wait_for_operations(
            &storage2,
            "tenant1",
            "repo1",
            "node1",
            i,
            Duration::from_secs(5),
        )
        .await
        .expect("Operation should replicate");

        latencies.push(replication_latency);

        // Print progress every 10 operations
        if i % 10 == 0 {
            println!(
                "  Progress: {}/100 - Last latency: {:?}",
                i, replication_latency
            );
        }

        // Track anomalies in real-time
        if replication_latency > Duration::from_millis(100) {
            println!(
                "  ⚠️  Spike detected at operation #{}: {:?}",
                i, replication_latency
            );
        }
    }

    // Calculate and print percentiles
    let stats = calculate_percentiles(latencies.clone());
    stats.print_report("Replication");

    // Find and report all spikes > 100ms
    println!("\n🔍 Analyzing tail latency spikes (>100ms):");
    let mut spike_count = 0;
    for (i, latency) in latencies.iter().enumerate() {
        if *latency > Duration::from_millis(100) {
            spike_count += 1;
            println!("  - Operation #{}: {:?}", i + 1, latency);
        }
    }

    if spike_count == 0 {
        println!("  ✅ No spikes detected - all operations < 100ms");
    } else {
        println!("\n  Total spikes: {}/{}", spike_count, sample_count);
        println!(
            "  Spike rate: {:.1}%",
            (spike_count as f64 / sample_count as f64) * 100.0
        );
    }

    // Performance assertions
    assert!(
        stats.p50 < Duration::from_millis(50),
        "P50 latency should be < 50ms, got {:?}",
        stats.p50
    );
    assert!(
        stats.p99 < Duration::from_millis(200),
        "P99 latency should be < 200ms, got {:?}",
        stats.p99
    );

    println!("\n✅ Latency distribution test complete!");
}

#[tokio::test]
#[ignore] // Run with --include-ignored
async fn test_cold_start_vs_warm_latency() {
    init_tracing();
    println!("\n🧪 Testing Cold Start vs Warm Replication Latency");

    let (_dir1, storage1) = create_replicated_storage("node1");
    let (_dir2, storage2) = create_replicated_storage("node2");
    let (port1, port2) = unique_ports();

    let peer2 = PeerConfig::new("node2", "127.0.0.1").with_port(port2);
    let peer1 = PeerConfig::new("node1", "127.0.0.1").with_port(port1);

    let _coordinator1 = start_node_replication(storage1.clone(), "node1", port1, vec![peer2]).await;
    let _coordinator2 = start_node_replication(storage2.clone(), "node2", port2, vec![peer1]).await;

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Measure cold start (first operation)
    println!("\n❄️  Measuring COLD START latency (first operation)...");
    let cold_start = Instant::now();
    storage1
        .operation_capture()
        .capture_create_node(
            "tenant1".to_string(),
            "repo1".to_string(),
            "main".to_string(),
            "cold-start".to_string(),
            "Cold Start Test".to_string(),
            "Document".to_string(),
            None,
            None,
            "a".to_string(),
            serde_json::json!({"test": "cold"}),
            None,
            None,
            "/Cold Start Test".to_string(),
            "perftest".to_string(),
        )
        .await
        .unwrap();

    let cold_latency = wait_for_operations(
        &storage2,
        "tenant1",
        "repo1",
        "node1",
        1,
        Duration::from_secs(5),
    )
    .await
    .unwrap();

    println!("  Cold start latency: {:?}", cold_latency);

    // Measure warm operations (next 10)
    println!("\n🔥 Measuring WARM latency (operations 2-11)...");
    let mut warm_latencies = Vec::new();

    for i in 2..=11 {
        let start = Instant::now();
        storage1
            .operation_capture()
            .capture_create_node(
                "tenant1".to_string(),
                "repo1".to_string(),
                "main".to_string(),
                format!("warm-{}", i),
                format!("Warm Test {}", i),
                "Document".to_string(),
                None,
                None,
                "a".to_string(),
                serde_json::json!({"test": "warm", "index": i}),
                None,
                None,
                format!("/Warm Test {}", i),
                "perftest".to_string(),
            )
            .await
            .unwrap();

        let latency = wait_for_operations(
            &storage2,
            "tenant1",
            "repo1",
            "node1",
            i,
            Duration::from_secs(5),
        )
        .await
        .unwrap();

        warm_latencies.push(latency);
    }

    let warm_stats = calculate_percentiles(warm_latencies);

    println!("\n📊 Cold vs Warm Comparison:");
    println!("  Cold start: {:?}", cold_latency);
    println!("  Warm P50:   {:?}", warm_stats.p50);
    println!("  Warm mean:  {:?}", warm_stats.mean);
    println!("  Warm max:   {:?}", warm_stats.max);

    let cold_vs_warm_ratio = cold_latency.as_nanos() as f64 / warm_stats.p50.as_nanos() as f64;
    println!("\n  Cold/Warm ratio: {:.1}x", cold_vs_warm_ratio);

    if cold_vs_warm_ratio > 10.0 {
        println!("  ⚠️  Cold start is >10x slower than warm operations");
        println!("  Likely cause: RocksDB cache warming, initial fsync, or compaction");
    } else {
        println!("  ✅ Cold start latency is reasonable");
    }

    println!("\n✅ Cold vs Warm test complete!");
}

#[tokio::test]
#[ignore] // Run with --include-ignored
async fn test_concurrent_bidirectional_writes() {
    init_tracing();
    println!("\n🧪 Testing Concurrent Bidirectional Writes");
    println!("   Both nodes writing simultaneously, measuring contention effects\n");

    // Create two nodes
    let (_dir1, storage1) = create_replicated_storage("node1");
    let (_dir2, storage2) = create_replicated_storage("node2");
    let (port1, port2) = unique_ports();

    let peer2 = PeerConfig::new("node2", "127.0.0.1").with_port(port2);
    let peer1 = PeerConfig::new("node1", "127.0.0.1").with_port(port1);

    println!("🚀 Starting coordinators...");
    let _coordinator1 = start_node_replication(storage1.clone(), "node1", port1, vec![peer2]).await;
    let _coordinator2 = start_node_replication(storage2.clone(), "node2", port2, vec![peer1]).await;

    tokio::time::sleep(Duration::from_millis(200)).await;

    println!("📊 Starting concurrent writes (50 ops per node)...\n");

    let storage1_task1 = storage1.clone();
    let storage2_task1 = storage2.clone();
    let storage1_task2 = storage1.clone();
    let storage2_task2 = storage2.clone();

    // Spawn two concurrent tasks
    let task1 = tokio::spawn(async move {
        let mut latencies = Vec::new();
        for i in 1..=50 {
            storage1_task1
                .operation_capture()
                .capture_create_node(
                    "tenant1".to_string(),
                    "repo1".to_string(),
                    "main".to_string(),
                    format!("node1-concurrent-{}", i),
                    format!("Node1 Concurrent {}", i),
                    "Document".to_string(),
                    None,
                    None,
                    format!("a{}", i),
                    serde_json::json!({"source": "node1", "index": i}),
                    None,
                    None,
                    format!("/Node1 Concurrent {}", i),
                    "concurrent-test".to_string(),
                )
                .await
                .unwrap();

            // Wait for replication to node2
            let replication_latency = wait_for_operations(
                &storage2_task1,
                "tenant1",
                "repo1",
                "node1",
                i,
                Duration::from_secs(5),
            )
            .await
            .unwrap();

            latencies.push(replication_latency);

            if i % 10 == 0 {
                println!("  Node1 progress: {}/50", i);
            }
        }
        latencies
    });

    let task2 = tokio::spawn(async move {
        let mut latencies = Vec::new();
        for i in 1..=50 {
            storage2_task2
                .operation_capture()
                .capture_create_node(
                    "tenant1".to_string(),
                    "repo1".to_string(),
                    "main".to_string(),
                    format!("node2-concurrent-{}", i),
                    format!("Node2 Concurrent {}", i),
                    "Document".to_string(),
                    None,
                    None,
                    format!("b{}", i),
                    serde_json::json!({"source": "node2", "index": i}),
                    None,
                    None,
                    format!("/Node2 Concurrent {}", i),
                    "concurrent-test".to_string(),
                )
                .await
                .unwrap();

            // Wait for replication to node1
            let replication_latency = wait_for_operations(
                &storage1_task2,
                "tenant1",
                "repo1",
                "node2",
                i,
                Duration::from_secs(5),
            )
            .await
            .unwrap();

            latencies.push(replication_latency);

            if i % 10 == 0 {
                println!("  Node2 progress: {}/50", i);
            }
        }
        latencies
    });

    // Wait for both tasks to complete
    let (latencies1, latencies2) = tokio::join!(task1, task2);
    let latencies1 = latencies1.unwrap();
    let latencies2 = latencies2.unwrap();

    // Calculate statistics for each direction
    let stats1 = calculate_percentiles(latencies1);
    let stats2 = calculate_percentiles(latencies2);

    println!("\n📊 Node1 → Node2 Replication (under concurrent load):");
    stats1.print_report("Node1→Node2");

    println!("\n📊 Node2 → Node1 Replication (under concurrent load):");
    stats2.print_report("Node2→Node1");

    // Compare to baseline (from sequential test)
    println!("\n🔍 Analysis:");
    println!("  Node1→2 P50: {:?}", stats1.p50);
    println!("  Node2→1 P50: {:?}", stats2.p50);

    let asymmetry_ratio = stats1.p50.as_nanos() as f64 / stats2.p50.as_nanos() as f64;
    println!("  Asymmetry ratio: {:.2}x", asymmetry_ratio);

    if asymmetry_ratio > 1.5 || asymmetry_ratio < 0.67 {
        println!("  ⚠️  Significant asymmetry detected (>1.5x difference)");
    } else {
        println!("  ✅ Symmetric performance (within 1.5x)");
    }

    // Verify all operations replicated
    let oplog1 = OpLogRepository::new(storage1.db().clone());
    let oplog2 = OpLogRepository::new(storage2.db().clone());

    let ops_from_node1 = oplog2
        .get_operations_from_node("tenant1", "repo1", "node1")
        .unwrap();
    let ops_from_node2 = oplog1
        .get_operations_from_node("tenant1", "repo1", "node2")
        .unwrap();

    println!("\n✅ Replication verification:");
    println!("  Node2 received {} ops from Node1", ops_from_node1.len());
    println!("  Node1 received {} ops from Node2", ops_from_node2.len());

    assert_eq!(
        ops_from_node1.len(),
        50,
        "Node2 should have all 50 ops from node1"
    );
    assert_eq!(
        ops_from_node2.len(),
        50,
        "Node1 should have all 50 ops from node2"
    );

    println!("\n✅ Concurrent bidirectional writes test complete!");
}

#[tokio::test]
#[ignore] // Run with --include-ignored
async fn test_burst_load() {
    init_tracing();
    println!("\n🧪 Testing Burst Load (Rapid Sequential Writes)");
    println!("   Sending 20 operations as fast as possible\n");

    let (_dir1, storage1) = create_replicated_storage("node1");
    let (_dir2, storage2) = create_replicated_storage("node2");
    let (port1, port2) = unique_ports();

    let peer2 = PeerConfig::new("node2", "127.0.0.1").with_port(port2);
    let peer1 = PeerConfig::new("node1", "127.0.0.1").with_port(port1);

    let _coordinator1 = start_node_replication(storage1.clone(), "node1", port1, vec![peer2]).await;
    let _coordinator2 = start_node_replication(storage2.clone(), "node2", port2, vec![peer1]).await;

    tokio::time::sleep(Duration::from_millis(200)).await;

    println!("💥 Sending burst of 20 operations with no delays...");

    let burst_start = Instant::now();

    // Send all operations as fast as possible (no await between captures)
    let mut futures = Vec::new();
    for i in 1..=20 {
        let future = storage1.operation_capture().capture_create_node(
            "tenant1".to_string(),
            "repo1".to_string(),
            "main".to_string(),
            format!("burst-{}", i),
            format!("Burst Test {}", i),
            "Document".to_string(),
            None,
            None,
            format!("a{}", i),
            serde_json::json!({"test": "burst", "index": i}),
            None,
            None,
            format!("/Burst {}", i),
            "burst-test".to_string(),
        );
        futures.push(future);
    }

    // Execute all captures concurrently
    futures::future::join_all(futures).await;

    let capture_duration = burst_start.elapsed();
    println!("  All 20 operations captured in: {:?}", capture_duration);
    println!(
        "  Throughput: {:.0} ops/sec",
        20.0 / capture_duration.as_secs_f64()
    );

    // Now measure how long it takes for all to replicate
    let replication_start = Instant::now();
    wait_for_operations(
        &storage2,
        "tenant1",
        "repo1",
        "node1",
        20,
        Duration::from_secs(10),
    )
    .await
    .expect("All 20 operations should replicate");

    let replication_duration = replication_start.elapsed();

    println!("\n📊 Burst Results:");
    println!("  Capture time: {:?}", capture_duration);
    println!("  Replication time: {:?}", replication_duration);
    println!("  Total time: {:?}", burst_start.elapsed());
    println!("  Average per-op latency: {:?}", replication_duration / 20);

    if replication_duration > capture_duration * 2 {
        println!("\n  ⚠️  Replication is slower than capture (possible queuing)");
    } else {
        println!("\n  ✅ Replication keeps up with burst writes");
    }

    // Verify all operations arrived
    let oplog2 = OpLogRepository::new(storage2.db().clone());
    let ops = oplog2
        .get_operations_from_node("tenant1", "repo1", "node1")
        .unwrap();

    assert_eq!(
        ops.len(),
        20,
        "All 20 burst operations should be replicated"
    );

    // Check sequence numbers are monotonic
    for i in 0..20 {
        assert_eq!(
            ops[i].op_seq,
            (i + 1) as u64,
            "Sequence numbers should be monotonic"
        );
    }

    println!("  ✅ All operations replicated in order");
    println!("\n✅ Burst load test complete!");
}
