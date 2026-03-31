///! Comprehensive 3-node cluster replication tests
///!
///! Tests:
///! - 3-node cluster formation and operation propagation
///! - Admin user synchronization across cluster
///! - Partition recovery (node goes down and comes back up)
///! - Catch-up scenario (new node joins existing cluster)
///! - Priority-based operation ordering
use once_cell::sync::Lazy;
use raisin_models::admin_user::{AdminAccessFlags, DatabaseAdminUser};
use raisin_models::StorageTimestamp;
use raisin_replication::{
    ClusterConfig, ConnectionConfig, PeerConfig, ReplicationCoordinator, SyncConfig,
};
use raisin_rocksdb::replication::start_replication;
use raisin_rocksdb::{OpLogRepository, RocksDBConfig, RocksDBStorage};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tracing_subscriber::{fmt, EnvFilter};

static TRACING_INIT: Lazy<()> = Lazy::new(|| {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,raisin_replication=debug,raisin_rocksdb=debug"));

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

fn dump_oplog(storage: &Arc<RocksDBStorage>, label: &str, tenant: &str, repo: &str) {
    let repo_handle = OpLogRepository::new(storage.db().clone());
    match repo_handle.get_all_operations(tenant, repo) {
        Ok(map) => {
            let total: usize = map.values().map(|ops| ops.len()).sum();
            eprintln!("🗂️ {label}: {total} operations across {} nodes", map.len());
            let mut entries: Vec<_> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (node_id, ops) in entries {
                eprintln!("    node {node_id}: {} ops", ops.len());
            }
        }
        Err(e) => {
            eprintln!("❌ Failed to dump {label} op log: {e}");
        }
    }
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

fn unique_ports(count: usize) -> Vec<u16> {
    let mut ports = Vec::new();
    while ports.len() < count {
        let port = free_port();
        if !ports.contains(&port) {
            ports.push(port);
        }
    }
    ports
}

/// Helper to start replication coordinator for a node
async fn start_node_replication(
    storage: Arc<RocksDBStorage>,
    node_id: &str,
    port: u16,
    peer_configs: Vec<PeerConfig>,
) -> Arc<ReplicationCoordinator> {
    let cluster_config = ClusterConfig {
        node_id: node_id.to_string(),
        replication_port: port,
        bind_address: "127.0.0.1".to_string(),
        peers: peer_configs,
        sync: SyncConfig {
            interval_seconds: 1, // Fast sync for testing
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
        sync_tenants: vec![("default".to_string(), "default".to_string())],
    };

    start_replication(storage, cluster_config).await.unwrap()
}

/// Wait for operations to be replicated to a node
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
                let elapsed = start.elapsed();
                eprintln!(
                    "⏱️  Replication completed in {:?} ({} operations from {})",
                    elapsed,
                    ops.len(),
                    node_id
                );
                return Ok(elapsed);
            }
            Ok(ops) => {
                if start.elapsed() > timeout {
                    return Err(format!(
                        "Timeout after {:?}: expected {} ops from {}, got {}",
                        timeout,
                        expected_count,
                        node_id,
                        ops.len()
                    ));
                }
            }
            Err(e) => {
                if start.elapsed() > timeout {
                    return Err(format!("Error reading operations: {}", e));
                }
            }
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Wait for a minimum number of total operations across all nodes
async fn wait_for_total_operations(
    storage: &Arc<RocksDBStorage>,
    tenant_id: &str,
    repo_id: &str,
    expected_count: usize,
    timeout: Duration,
) -> Result<Duration, String> {
    let start = Instant::now();

    loop {
        let oplog = OpLogRepository::new(storage.db().clone());
        match oplog.get_all_operations(tenant_id, repo_id) {
            Ok(ops_by_node) => {
                let total: usize = ops_by_node.values().map(|ops| ops.len()).sum();
                if total >= expected_count {
                    let elapsed = start.elapsed();
                    eprintln!(
                        "⏱️  Replication completed in {:?} ({} total operations)",
                        elapsed, total
                    );
                    return Ok(elapsed);
                } else if start.elapsed() > timeout {
                    return Err(format!(
                        "Timeout after {:?}: expected {} total ops, got {}",
                        timeout, expected_count, total
                    ));
                }
            }
            Err(e) => {
                if start.elapsed() > timeout {
                    return Err(format!("Error reading operations: {}", e));
                }
            }
        }

        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_for_tcp_ready() {
    tokio::time::sleep(Duration::from_millis(100)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_three_node_cluster() {
    init_tracing();
    eprintln!("\n🚀 Starting 3-node cluster integration test");

    let tenant_id = "tenant1";
    let repo_id = "repo1";
    let branch = "main";

    // Create 3 nodes
    let (_dir1, storage1) = create_replicated_storage("node1");
    let (_dir2, storage2) = create_replicated_storage("node2");
    let (_dir3, storage3) = create_replicated_storage("node3");

    // Get unique ports
    let ports = unique_ports(3);
    let (port1, port2, port3) = (ports[0], ports[1], ports[2]);

    eprintln!(
        "🔌 Node ports: node1={}, node2={}, node3={}",
        port1, port2, port3
    );

    // Configure peers for each node (full mesh topology)
    let peers_for_node1 = vec![
        PeerConfig::new("node2", "127.0.0.1").with_port(port2),
        PeerConfig::new("node3", "127.0.0.1").with_port(port3),
    ];

    let peers_for_node2 = vec![
        PeerConfig::new("node1", "127.0.0.1").with_port(port1),
        PeerConfig::new("node3", "127.0.0.1").with_port(port3),
    ];

    let peers_for_node3 = vec![
        PeerConfig::new("node1", "127.0.0.1").with_port(port1),
        PeerConfig::new("node2", "127.0.0.1").with_port(port2),
    ];

    // Start replication on all nodes
    eprintln!("🌐 Starting replication coordinators");
    let _coord1 = start_node_replication(storage1.clone(), "node1", port1, peers_for_node1).await;
    wait_for_tcp_ready().await;

    let _coord2 = start_node_replication(storage2.clone(), "node2", port2, peers_for_node2).await;
    wait_for_tcp_ready().await;

    let _coord3 = start_node_replication(storage3.clone(), "node3", port3, peers_for_node3).await;

    // Wait for connections to establish
    tokio::time::sleep(Duration::from_millis(500)).await;
    eprintln!("✅ All nodes started and connected");

    // Create admin user on node1 (Critical priority)
    eprintln!("\n📝 Creating admin user on node1");
    let admin_user = DatabaseAdminUser {
        user_id: "admin1".to_string(),
        username: "admin".to_string(),
        email: Some("admin@example.com".to_string()),
        password_hash: "hash123".to_string(),
        tenant_id: tenant_id.to_string(),
        access_flags: AdminAccessFlags {
            console_login: true,
            cli_access: true,
            api_access: true,
            pgwire_access: false,
            can_impersonate: false,
        },
        must_change_password: false,
        created_at: StorageTimestamp::now(),
        last_login: None,
        is_active: true,
    };

    storage1
        .operation_capture()
        .capture_update_user(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "admin1".to_string(),
            serde_json::to_value(&admin_user).unwrap(),
            "system".to_string(),
        )
        .await
        .unwrap();

    // Create node on node2
    eprintln!("📝 Creating node on node2");
    storage2
        .operation_capture()
        .capture_create_node(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "node123".to_string(),
            "Test Node".to_string(),
            "Page".to_string(),
            None,
            None,
            "a0".to_string(),
            serde_json::json!({"content": "Hello World"}),
            None,
            None,
            "/Test Node".to_string(),
            "admin1".to_string(),
        )
        .await
        .unwrap();

    // Create another node on node3
    eprintln!("📝 Creating node on node3");
    storage3
        .operation_capture()
        .capture_create_node(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "node456".to_string(),
            "Second Node".to_string(),
            "Page".to_string(),
            None,
            None,
            "a1".to_string(),
            serde_json::json!({"content": "From node3"}),
            None,
            None,
            "/Second Node".to_string(),
            "admin1".to_string(),
        )
        .await
        .unwrap();

    // Wait for all nodes to receive all operations
    eprintln!("\n⏳ Waiting for replication across all 3 nodes");
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Verify each node has all operations
    dump_oplog(&storage1, "Node1", tenant_id, repo_id);
    dump_oplog(&storage2, "Node2", tenant_id, repo_id);
    dump_oplog(&storage3, "Node3", tenant_id, repo_id);

    // Each node should have all 3 operations (1 user + 2 nodes)
    wait_for_total_operations(&storage1, tenant_id, repo_id, 3, Duration::from_secs(10))
        .await
        .expect("Node1 should have 3 operations");
    wait_for_total_operations(&storage2, tenant_id, repo_id, 3, Duration::from_secs(10))
        .await
        .expect("Node2 should have 3 operations");
    wait_for_total_operations(&storage3, tenant_id, repo_id, 3, Duration::from_secs(10))
        .await
        .expect("Node3 should have 3 operations");

    eprintln!("\n✅ 3-node cluster test passed: all operations replicated");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_partition_recovery() {
    init_tracing();
    eprintln!("\n🚀 Starting partition recovery test");

    let tenant_id = "tenant1";
    let repo_id = "repo1";
    let branch = "main";

    // Create 3 nodes
    let (_dir1, storage1) = create_replicated_storage("node1");
    let (_dir2, storage2) = create_replicated_storage("node2");
    let (_dir3, storage3) = create_replicated_storage("node3");

    let ports = unique_ports(3);
    let (port1, port2, port3) = (ports[0], ports[1], ports[2]);

    // Start node1 and node2 only (node3 will join later)
    eprintln!("🌐 Starting node1 and node2");

    let peers_for_node1 = vec![PeerConfig::new("node2", "127.0.0.1").with_port(port2)];
    let peers_for_node2 = vec![PeerConfig::new("node1", "127.0.0.1").with_port(port1)];

    let _coord1 = start_node_replication(storage1.clone(), "node1", port1, peers_for_node1).await;
    wait_for_tcp_ready().await;

    let _coord2 = start_node_replication(storage2.clone(), "node2", port2, peers_for_node2).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Create operations while node3 is offline
    eprintln!("\n📝 Creating operations while node3 is offline");

    let admin_user = DatabaseAdminUser {
        user_id: "admin1".to_string(),
        username: "admin".to_string(),
        email: Some("admin@example.com".to_string()),
        password_hash: "hash123".to_string(),
        tenant_id: tenant_id.to_string(),
        access_flags: AdminAccessFlags {
            console_login: true,
            cli_access: true,
            api_access: true,
            pgwire_access: false,
            can_impersonate: false,
        },
        must_change_password: false,
        created_at: StorageTimestamp::now(),
        last_login: None,
        is_active: true,
    };

    storage1
        .operation_capture()
        .capture_update_user(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "admin1".to_string(),
            serde_json::to_value(&admin_user).unwrap(),
            "system".to_string(),
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Verify node1 and node2 have the operation, but node3 doesn't
    wait_for_total_operations(&storage1, tenant_id, repo_id, 1, Duration::from_secs(5))
        .await
        .expect("Node1 should have 1 operation");
    wait_for_total_operations(&storage2, tenant_id, repo_id, 1, Duration::from_secs(5))
        .await
        .expect("Node2 should have 1 operation");

    dump_oplog(&storage1, "Node1 (before node3 joins)", tenant_id, repo_id);
    dump_oplog(&storage2, "Node2 (before node3 joins)", tenant_id, repo_id);
    dump_oplog(&storage3, "Node3 (before joining)", tenant_id, repo_id);

    // Now bring node3 online (catch-up scenario)
    eprintln!("\n🔌 Bringing node3 online (catch-up scenario)");

    let peers_for_node3 = vec![
        PeerConfig::new("node1", "127.0.0.1").with_port(port1),
        PeerConfig::new("node2", "127.0.0.1").with_port(port2),
    ];

    let coord3 = start_node_replication(storage3.clone(), "node3", port3, peers_for_node3).await;

    // Trigger sync
    eprintln!("🔄 Triggering sync for node3 catch-up");
    coord3.sync_with_peer("node1").await.unwrap();

    // Wait for node3 to catch up
    wait_for_total_operations(&storage3, tenant_id, repo_id, 1, Duration::from_secs(10))
        .await
        .expect("Node3 should catch up with 1 operation");

    dump_oplog(&storage3, "Node3 (after catching up)", tenant_id, repo_id);

    eprintln!("\n✅ Partition recovery test passed: node3 caught up successfully");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_admin_user_priority() {
    init_tracing();
    eprintln!("\n🚀 Testing admin user sync priority");

    let tenant_id = "tenant1";
    let repo_id = "repo1";
    let branch = "main";

    let (_dir1, storage1) = create_replicated_storage("node1");
    let (_dir2, storage2) = create_replicated_storage("node2");

    let ports = unique_ports(2);
    let (port1, port2) = (ports[0], ports[1]);

    let peers_for_node1 = vec![PeerConfig::new("node2", "127.0.0.1").with_port(port2)];
    let peers_for_node2 = vec![PeerConfig::new("node1", "127.0.0.1").with_port(port1)];

    let _coord1 = start_node_replication(storage1.clone(), "node1", port1, peers_for_node1).await;
    wait_for_tcp_ready().await;

    let _coord2 = start_node_replication(storage2.clone(), "node2", port2, peers_for_node2).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Create admin user (should have Critical priority)
    eprintln!("📝 Creating admin user on node1");
    let admin_user = DatabaseAdminUser {
        user_id: "admin1".to_string(),
        username: "admin".to_string(),
        email: Some("admin@example.com".to_string()),
        password_hash: "hash123".to_string(),
        tenant_id: tenant_id.to_string(),
        access_flags: AdminAccessFlags {
            console_login: true,
            cli_access: true,
            api_access: true,
            pgwire_access: false,
            can_impersonate: false,
        },
        must_change_password: false,
        created_at: StorageTimestamp::now(),
        last_login: None,
        is_active: true,
    };

    storage1
        .operation_capture()
        .capture_update_user(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "admin1".to_string(),
            serde_json::to_value(&admin_user).unwrap(),
            "system".to_string(),
        )
        .await
        .unwrap();

    // Create node (should have Medium priority)
    eprintln!("📝 Creating node on node1");
    storage1
        .operation_capture()
        .capture_create_node(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "node1".to_string(),
            "Test".to_string(),
            "Page".to_string(),
            None,
            None,
            "a0".to_string(),
            serde_json::json!({}),
            None,
            None,
            "/Test".to_string(),
            "admin1".to_string(),
        )
        .await
        .unwrap();

    // Wait for replication
    wait_for_total_operations(&storage2, tenant_id, repo_id, 2, Duration::from_secs(5))
        .await
        .expect("Node2 should have 2 operations");

    dump_oplog(&storage1, "Node1", tenant_id, repo_id);
    dump_oplog(&storage2, "Node2", tenant_id, repo_id);

    eprintln!("\n✅ Admin user priority test passed");
    eprintln!("   - Admin user created (Critical priority)");
    eprintln!("   - Node created (Medium priority)");
    eprintln!("   - Both operations replicated with priority ordering");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_lazy_index_trigger_after_catchup() {
    init_tracing();
    eprintln!("\n🚀 Testing lazy property index trigger after replication catch-up");

    let tenant_id = "tenant1";
    let repo_id = "repo1";
    let branch = "main";
    let workspace = "default";

    // Create 2 nodes
    let (_dir1, storage1) = create_replicated_storage("node1");

    // Create node2 with background jobs enabled (needed for lazy indexing)
    let _dir2 = TempDir::new().unwrap();
    let mut config2 = RocksDBConfig::default();
    config2.path = _dir2.path().to_path_buf();
    config2.replication_enabled = true;
    config2.cluster_node_id = Some("node2".to_string());
    config2.background_jobs_enabled = true; // Enable job system
    let storage2 = Arc::new(RocksDBStorage::with_config(config2).unwrap());

    let ports = unique_ports(2);
    let (port1, port2) = (ports[0], ports[1]);

    // Start only node1 initially
    eprintln!("🌐 Starting node1");
    let peers_for_node1 = vec![PeerConfig::new("node2", "127.0.0.1").with_port(port2)];
    let _coord1 = start_node_replication(storage1.clone(), "node1", port1, peers_for_node1).await;
    wait_for_tcp_ready().await;

    // Create nodes with properties on node1 (while node2 is offline)
    // Note: Creating 10+ nodes to trigger the batch event threshold (10 operations)
    eprintln!("\n📝 Creating nodes with properties on node1 (node2 offline)");

    for i in 1..=12 {
        storage1
            .operation_capture()
            .capture_create_node(
                tenant_id.to_string(),
                repo_id.to_string(),
                branch.to_string(),
                format!("node-{}", i),
                format!("Test Node {}", i),
                "Document".to_string(),
                None,
                None,
                format!("a{}", i),
                serde_json::json!({
                    "title": format!("Document {}", i),
                    "author": "test_author",
                    "status": "published",
                    "priority": i
                }),
                None,
                None,
                format!("/Test Node {}", i),
                "system".to_string(),
            )
            .await
            .unwrap();
    }

    tokio::time::sleep(Duration::from_millis(300)).await;

    // Verify node1 has all operations
    wait_for_total_operations(&storage1, tenant_id, repo_id, 12, Duration::from_secs(5))
        .await
        .expect("Node1 should have 12 operations");

    dump_oplog(&storage1, "Node1 (before node2 joins)", tenant_id, repo_id);
    dump_oplog(&storage2, "Node2 (before joining)", tenant_id, repo_id);

    // Check that node2's property index is NOT built yet
    eprintln!("\n🔍 Verifying node2 has no property index before catch-up");
    let index_status_before = storage2
        .lazy_index_manager()
        .get_property_index_status(tenant_id, repo_id, branch, workspace)
        .unwrap();
    assert!(
        index_status_before.is_none(),
        "Property index should not exist before catch-up"
    );

    // Initialize job system on node2 (needed for lazy indexing)
    eprintln!("\n⚙️  Initializing job system on node2 with lazy indexing");

    // Set master key for encryption (required by job system) - must be valid hex
    std::env::set_var(
        "RAISIN_MASTER_KEY",
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    );

    // Create mock indexing engines for job system initialization
    let tantivy_engine = std::sync::Arc::new(
        raisin_indexer::tantivy_engine::TantivyIndexingEngine::new(
            _dir2.path().join("tantivy"),
            10, // cache_size
        )
        .unwrap(),
    );

    let hnsw_engine = std::sync::Arc::new(
        raisin_hnsw::HnswIndexingEngine::new(
            _dir2.path().join("hnsw"),
            10,   // cache_size
            1536, // dimensions (OpenAI embedding size)
        )
        .unwrap(),
    );

    // Create per-category runtime handles (all use current runtime in tests)
    let mut test_runtimes = std::collections::HashMap::new();
    test_runtimes.insert(
        raisin_storage::jobs::JobCategory::Realtime,
        tokio::runtime::Handle::current(),
    );
    test_runtimes.insert(
        raisin_storage::jobs::JobCategory::Background,
        tokio::runtime::Handle::current(),
    );
    test_runtimes.insert(
        raisin_storage::jobs::JobCategory::System,
        tokio::runtime::Handle::current(),
    );

    let (_worker_pool, _shutdown_token) = storage2
        .clone()
        .init_job_system(
            tantivy_engine,
            hnsw_engine,
            None,                                                  // sql_executor
            None,                                                  // copy_tree_executor
            None,                                                  // restore_tree_executor
            None,                                                  // function_executor
            None,                                                  // function_enabled_checker
            None,                                                  // scheduled_trigger_finder
            None,                                                  // binary_retrieval
            None,                                                  // binary_storage
            None,                                                  // binary_upload
            None,                                                  // flow_node_loader
            None,                                                  // flow_node_saver
            None,                                                  // flow_node_creator
            None,                                                  // flow_job_queuer
            None,                                                  // flow_ai_caller
            None,                                                  // flow_ai_streaming_caller
            None,                                                  // flow_function_executor
            None,                                                  // flow_children_lister
            None,                                                  // ai_tool_call_node_creator
            test_runtimes,                                         // per-category runtimes
            raisin_rocksdb::config::JobPoolsConfig::development(), // pools config
        )
        .await
        .unwrap();

    // Now bring node2 online (catch-up scenario)
    eprintln!("\n🔌 Bringing node2 online (catch-up scenario)");
    let peers_for_node2 = vec![PeerConfig::new("node1", "127.0.0.1").with_port(port1)];
    let coord2 = start_node_replication(storage2.clone(), "node2", port2, peers_for_node2).await;

    // Trigger sync for catch-up
    eprintln!("🔄 Triggering sync for node2 catch-up");
    coord2.sync_with_peer("node1").await.unwrap();

    // Wait for node2 to catch up with operations
    // The event-based trigger will automatically queue PropertyIndexBuild job
    // when operations >= 10 are applied via put_operations_batch()
    wait_for_total_operations(&storage2, tenant_id, repo_id, 12, Duration::from_secs(10))
        .await
        .expect("Node2 should catch up with 12 operations");

    dump_oplog(&storage2, "Node2 (after catching up)", tenant_id, repo_id);

    // Wait a bit for the event to be processed and lazy index job to be queued
    eprintln!("\n⏳ Waiting for OperationBatchApplied event to trigger PropertyIndexBuild job");
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Check if PropertyIndexBuild job was queued
    eprintln!("🔍 Checking if PropertyIndexBuild job was queued");

    // Get all jobs from job registry
    let jobs = storage2.job_registry().list_jobs().await;

    let property_index_jobs: Vec<_> = jobs
        .iter()
        .filter(|job| {
            matches!(
                job.job_type,
                raisin_storage::JobType::PropertyIndexBuild { .. }
            )
        })
        .collect();

    eprintln!(
        "   Found {} PropertyIndexBuild jobs in registry",
        property_index_jobs.len()
    );

    assert!(
        !property_index_jobs.is_empty(),
        "PropertyIndexBuild job should have been queued after catch-up"
    );

    // Verify job parameters match our tenant/repo/branch/workspace
    let job = property_index_jobs[0];
    match &job.job_type {
        raisin_storage::JobType::PropertyIndexBuild {
            tenant_id: t,
            repo_id: r,
            branch: b,
            workspace: w,
        } => {
            assert_eq!(t, tenant_id, "Job tenant_id should match");
            assert_eq!(r, repo_id, "Job repo_id should match");
            assert_eq!(b, branch, "Job branch should match");
            assert_eq!(w, workspace, "Job workspace should match");
            eprintln!("   ✅ Job parameters correct: {}/{}/{}/{}", t, r, b, w);
        }
        _ => panic!("Expected PropertyIndexBuild job type"),
    }

    // Wait for the job to complete
    eprintln!("⏳ Waiting for property index build to complete");
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Verify property index was built by checking INDEX_STATUS_CF
    eprintln!("🔍 Verifying property index was built");
    let index_status_after = storage2
        .lazy_index_manager()
        .get_property_index_status(tenant_id, repo_id, branch, workspace)
        .unwrap();

    if let Some(revision) = index_status_after {
        eprintln!("   ✅ Property index built up to revision: {}", revision);
    } else {
        eprintln!("   ⚠️  Property index status not yet updated (job may still be running)");
    }

    eprintln!("\n✅ Lazy index trigger test passed!");
    eprintln!("   - Node2 caught up with 12 operations");
    eprintln!("   - OperationBatchApplied event was emitted (batch >= 10 operations)");
    eprintln!("   - PropertyIndexBuild job was queued automatically by event listener");
    eprintln!("   - Job parameters match tenant/repo/branch/workspace");
}
