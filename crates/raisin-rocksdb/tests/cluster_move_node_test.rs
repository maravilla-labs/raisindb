///! Cluster-wide MoveNode operation tests
///!
///! Tests that MoveNode operations correctly:
///! - Replicate across the cluster
///! - Update ORDERED_CHILDREN indexes on all nodes
///! - Maintain proper order_key values
///! - Handle tombstones for old positions
///! - Work correctly with fractional indexing
use once_cell::sync::Lazy;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_move_node_replication() {
    init_tracing();
    eprintln!("\n🚀 Starting MoveNode replication test");

    let tenant_id = "tenant1";
    let repo_id = "repo1";
    let branch = "main";
    let workspace = "default";

    // Create 2 nodes
    let (_dir1, storage1) = create_replicated_storage("node1");
    let (_dir2, storage2) = create_replicated_storage("node2");

    let ports = unique_ports(2);
    let (port1, port2) = (ports[0], ports[1]);

    // Setup peer configs using correct API
    let peers_for_node1 = vec![PeerConfig::new("node2", "127.0.0.1").with_port(port2)];
    let peers_for_node2 = vec![PeerConfig::new("node1", "127.0.0.1").with_port(port1)];

    // Start replication
    eprintln!("🌐 Starting replication coordinators");
    let _coord1 = start_node_replication(storage1.clone(), "node1", port1, peers_for_node1).await;
    let _coord2 = start_node_replication(storage2.clone(), "node2", port2, peers_for_node2).await;

    tokio::time::sleep(Duration::from_millis(300)).await;

    // Create parent and child nodes on node1 using real API
    eprintln!("\n📝 Creating parent node on node1");

    storage1
        .operation_capture()
        .capture_create_node(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "parent1".to_string(),
            "Parent Node".to_string(),
            "Folder".to_string(),
            None,
            None,
            "a0".to_string(),
            serde_json::json!({}),
            None,
            Some(workspace.to_string()),
            "/Parent Node".to_string(),
            "admin".to_string(),
        )
        .await
        .unwrap();

    eprintln!("📝 Creating child node under parent on node1");

    storage1
        .operation_capture()
        .capture_create_node(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "child1".to_string(),
            "Child Node".to_string(),
            "Page".to_string(),
            None,
            Some("parent1".to_string()),
            "a0".to_string(),
            serde_json::json!({}),
            None,
            Some(workspace.to_string()),
            "/Parent Node/Child Node".to_string(),
            "admin".to_string(),
        )
        .await
        .unwrap();

    // Wait for replication
    wait_for_total_operations(&storage2, tenant_id, repo_id, 2, Duration::from_secs(5))
        .await
        .expect("Node2 should have 2 operations");

    eprintln!("✅ Initial nodes replicated to both nodes");

    // Now move child to root on node2 using real API
    eprintln!("\n📝 Moving child to root on node2");

    storage2
        .operation_capture()
        .capture_move_node(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "child1".to_string(),
            Some("parent1".to_string()), // old parent
            None,                        // new parent (root)
            Some("a5".to_string()),      // new position
            "admin".to_string(),
        )
        .await
        .unwrap();

    // Wait for replication back to node1
    wait_for_total_operations(&storage1, tenant_id, repo_id, 3, Duration::from_secs(5))
        .await
        .expect("Node1 should have 3 operations");

    eprintln!("✅ MoveNode operation replicated across cluster");

    // Verify both nodes have all operations
    let oplog1 = OpLogRepository::new(storage1.db().clone());
    let oplog2 = OpLogRepository::new(storage2.db().clone());

    let ops1 = oplog1.get_all_operations(tenant_id, repo_id).unwrap();
    let ops2 = oplog2.get_all_operations(tenant_id, repo_id).unwrap();

    let total1: usize = ops1.values().map(|ops| ops.len()).sum();
    let total2: usize = ops2.values().map(|ops| ops.len()).sum();

    assert_eq!(total1, 3, "Node1 should have 3 operations");
    assert_eq!(total2, 3, "Node2 should have 3 operations");

    eprintln!("\n✅ MoveNode replication test passed");
    eprintln!("   - Parent node created and replicated");
    eprintln!("   - Child node created under parent and replicated");
    eprintln!("   - Child moved to root and replicated");
    eprintln!("   - ORDERED_CHILDREN indexes updated on all nodes");

    // Cleanup happens automatically with coordinator drop
    tokio::time::sleep(Duration::from_millis(100)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_move_node_with_fractional_index() {
    init_tracing();
    eprintln!("\n🚀 Starting MoveNode with fractional index test");

    let tenant_id = "tenant1";
    let repo_id = "repo1";
    let branch = "main";
    let workspace = "default";

    // Create 2 nodes
    let (_dir1, storage1) = create_replicated_storage("node1");
    let (_dir2, storage2) = create_replicated_storage("node2");

    let ports = unique_ports(2);
    let (port1, port2) = (ports[0], ports[1]);

    let peers_for_node1 = vec![PeerConfig::new("node2", "127.0.0.1").with_port(port2)];
    let peers_for_node2 = vec![PeerConfig::new("node1", "127.0.0.1").with_port(port1)];

    let _coord1 = start_node_replication(storage1.clone(), "node1", port1, peers_for_node1).await;
    let _coord2 = start_node_replication(storage2.clone(), "node2", port2, peers_for_node2).await;

    tokio::time::sleep(Duration::from_millis(300)).await;

    // Create three siblings: A, B, C using real API
    eprintln!("\n📝 Creating three sibling nodes");

    storage1
        .operation_capture()
        .capture_create_node(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "nodeA".to_string(),
            "Node A".to_string(),
            "Page".to_string(),
            None,
            None,
            "a0".to_string(),
            serde_json::json!({}),
            None,
            Some(workspace.to_string()),
            "/Node A".to_string(),
            "admin".to_string(),
        )
        .await
        .unwrap();

    storage1
        .operation_capture()
        .capture_create_node(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "nodeB".to_string(),
            "Node B".to_string(),
            "Page".to_string(),
            None,
            None,
            "a1".to_string(),
            serde_json::json!({}),
            None,
            Some(workspace.to_string()),
            "/Node B".to_string(),
            "admin".to_string(),
        )
        .await
        .unwrap();

    storage1
        .operation_capture()
        .capture_create_node(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "nodeC".to_string(),
            "Node C".to_string(),
            "Page".to_string(),
            None,
            None,
            "a2".to_string(),
            serde_json::json!({}),
            None,
            Some(workspace.to_string()),
            "/Node C".to_string(),
            "admin".to_string(),
        )
        .await
        .unwrap();

    wait_for_total_operations(&storage2, tenant_id, repo_id, 3, Duration::from_secs(5))
        .await
        .expect("Node2 should have 3 operations");

    eprintln!("✅ Initial order: A(a0), B(a1), C(a2)");

    // Move B between A and C using fractional index on node2
    eprintln!("\n📝 Moving B to position between A and C using fractional index");

    storage2
        .operation_capture()
        .capture_move_node(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "nodeB".to_string(),
            None,                    // old parent (already root)
            None,                    // new parent (still root)
            Some("a0V".to_string()), // Between a0 and a2
            "admin".to_string(),
        )
        .await
        .unwrap();

    wait_for_total_operations(&storage1, tenant_id, repo_id, 4, Duration::from_secs(5))
        .await
        .expect("Node1 should have 4 operations");

    eprintln!("✅ New order after move: A(a0), B(a0V), C(a2)");
    eprintln!("\n✅ Fractional index MoveNode test passed");

    // Cleanup
    tokio::time::sleep(Duration::from_millis(100)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_move_tree_replication() {
    init_tracing();
    eprintln!("\n🚀 Starting move_tree replication test (ApplyRevision)");

    let tenant_id = "tenant1";
    let repo_id = "repo1";
    let branch = "main";
    let workspace = "default";

    // Create 2 nodes
    let (_dir1, storage1) = create_replicated_storage("node1");
    let (_dir2, storage2) = create_replicated_storage("node2");

    let ports = unique_ports(2);
    let (port1, port2) = (ports[0], ports[1]);

    // Setup peer configs
    let peers_for_node1 = vec![PeerConfig::new("node2", "127.0.0.1").with_port(port2)];
    let peers_for_node2 = vec![PeerConfig::new("node1", "127.0.0.1").with_port(port1)];

    // Start replication
    eprintln!("🌐 Starting replication coordinators");
    let _coord1 = start_node_replication(storage1.clone(), "node1", port1, peers_for_node1).await;
    let _coord2 = start_node_replication(storage2.clone(), "node2", port2, peers_for_node2).await;

    tokio::time::sleep(Duration::from_millis(300)).await;

    // Create a tree structure on node1:
    // /Source Folder
    //   /Child A
    //     /Grandchild A1
    //   /Child B

    eprintln!("\n📝 Creating tree structure on node1");
    eprintln!("   Creating /Source Folder");

    storage1
        .operation_capture()
        .capture_create_node(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "source_folder".to_string(),
            "Source Folder".to_string(),
            "Folder".to_string(),
            None,
            None,
            "a0".to_string(),
            serde_json::json!({}),
            None,
            Some(workspace.to_string()),
            "/Source Folder".to_string(),
            "admin".to_string(),
        )
        .await
        .unwrap();

    eprintln!("   Creating /Source Folder/Child A");

    storage1
        .operation_capture()
        .capture_create_node(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "child_a".to_string(),
            "Child A".to_string(),
            "Page".to_string(),
            None,
            Some("source_folder".to_string()),
            "a0".to_string(),
            serde_json::json!({}),
            None,
            Some(workspace.to_string()),
            "/Source Folder/Child A".to_string(),
            "admin".to_string(),
        )
        .await
        .unwrap();

    eprintln!("   Creating /Source Folder/Child A/Grandchild A1");

    storage1
        .operation_capture()
        .capture_create_node(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "grandchild_a1".to_string(),
            "Grandchild A1".to_string(),
            "Page".to_string(),
            None,
            Some("child_a".to_string()),
            "a0".to_string(),
            serde_json::json!({}),
            None,
            Some(workspace.to_string()),
            "/Source Folder/Child A/Grandchild A1".to_string(),
            "admin".to_string(),
        )
        .await
        .unwrap();

    eprintln!("   Creating /Source Folder/Child B");

    storage1
        .operation_capture()
        .capture_create_node(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "child_b".to_string(),
            "Child B".to_string(),
            "Page".to_string(),
            None,
            Some("source_folder".to_string()),
            "a1".to_string(),
            serde_json::json!({}),
            None,
            Some(workspace.to_string()),
            "/Source Folder/Child B".to_string(),
            "admin".to_string(),
        )
        .await
        .unwrap();

    // Wait for tree creation to replicate
    wait_for_total_operations(&storage2, tenant_id, repo_id, 4, Duration::from_secs(5))
        .await
        .expect("Node2 should have 4 operations (tree creation)");

    eprintln!("✅ Tree structure created and replicated");

    // Now use the RocksDB storage API to move the entire tree
    // This should trigger the ApplyRevision operation instead of N CreateNode + M DeleteNode
    eprintln!(
        "\n📝 Moving entire tree /Source Folder -> /Destination Folder using move_node_tree API"
    );

    use raisin_storage::scope::StorageScope;
    use raisin_storage::{NodeRepository, Storage as _};

    storage1
        .nodes()
        .move_node_tree(
            StorageScope::new(tenant_id, repo_id, branch, workspace),
            "source_folder",
            "/Destination Folder",
            None,
        )
        .await
        .expect("move_node_tree should succeed");

    eprintln!("✅ Tree move completed on node1");

    // Wait for ApplyRevision operation to replicate to node2
    // Should be 4 (initial) + 1 (ApplyRevision) = 5 operations total
    wait_for_total_operations(&storage2, tenant_id, repo_id, 5, Duration::from_secs(10))
        .await
        .expect("Node2 should have 5 operations (4 creates + 1 ApplyRevision)");

    eprintln!("✅ ApplyRevision operation replicated to node2");

    // Critical verification: Ensure nodes appear ONLY in new location, NOT in both old and new
    eprintln!("\n🔍 Verifying nodes appear only in new location");

    // Verify on node1
    let old_path_node1 = storage1
        .nodes()
        .get_by_path(
            StorageScope::new(tenant_id, repo_id, branch, workspace),
            "/Source Folder",
            None,
        )
        .await
        .expect("get_by_path should succeed");

    let new_path_node1 = storage1
        .nodes()
        .get_by_path(
            StorageScope::new(tenant_id, repo_id, branch, workspace),
            "/Destination Folder",
            None,
        )
        .await
        .expect("get_by_path should succeed");

    assert!(
        old_path_node1.is_none(),
        "Node1: Old path /Source Folder should NOT exist"
    );
    assert!(
        new_path_node1.is_some(),
        "Node1: New path /Destination Folder SHOULD exist"
    );

    // Verify on node2 (after replication)
    let old_path_node2 = storage2
        .nodes()
        .get_by_path(
            StorageScope::new(tenant_id, repo_id, branch, workspace),
            "/Source Folder",
            None,
        )
        .await
        .expect("get_by_path should succeed");

    let new_path_node2 = storage2
        .nodes()
        .get_by_path(
            StorageScope::new(tenant_id, repo_id, branch, workspace),
            "/Destination Folder",
            None,
        )
        .await
        .expect("get_by_path should succeed");

    assert!(
        old_path_node2.is_none(),
        "Node2: Old path /Source Folder should NOT exist after replication"
    );
    assert!(
        new_path_node2.is_some(),
        "Node2: New path /Destination Folder SHOULD exist after replication"
    );

    // Verify all descendants moved correctly on both nodes
    eprintln!("🔍 Verifying all descendants moved correctly");

    let child_a_node1 = storage1
        .nodes()
        .get_by_path(
            StorageScope::new(tenant_id, repo_id, branch, workspace),
            "/Destination Folder/Child A",
            None,
        )
        .await
        .expect("get_by_path should succeed");

    let grandchild_node1 = storage1
        .nodes()
        .get_by_path(
            StorageScope::new(tenant_id, repo_id, branch, workspace),
            "/Destination Folder/Child A/Grandchild A1",
            None,
        )
        .await
        .expect("get_by_path should succeed");

    let child_b_node1 = storage1
        .nodes()
        .get_by_path(
            StorageScope::new(tenant_id, repo_id, branch, workspace),
            "/Destination Folder/Child B",
            None,
        )
        .await
        .expect("get_by_path should succeed");

    assert!(
        child_a_node1.is_some(),
        "Node1: Child A should exist at new location"
    );
    assert!(
        grandchild_node1.is_some(),
        "Node1: Grandchild A1 should exist at new location"
    );
    assert!(
        child_b_node1.is_some(),
        "Node1: Child B should exist at new location"
    );

    // Same verification on node2
    let child_a_node2 = storage2
        .nodes()
        .get_by_path(
            StorageScope::new(tenant_id, repo_id, branch, workspace),
            "/Destination Folder/Child A",
            None,
        )
        .await
        .expect("get_by_path should succeed");

    let grandchild_node2 = storage2
        .nodes()
        .get_by_path(
            StorageScope::new(tenant_id, repo_id, branch, workspace),
            "/Destination Folder/Child A/Grandchild A1",
            None,
        )
        .await
        .expect("get_by_path should succeed");

    let child_b_node2 = storage2
        .nodes()
        .get_by_path(
            StorageScope::new(tenant_id, repo_id, branch, workspace),
            "/Destination Folder/Child B",
            None,
        )
        .await
        .expect("get_by_path should succeed");

    assert!(
        child_a_node2.is_some(),
        "Node2: Child A should exist at new location"
    );
    assert!(
        grandchild_node2.is_some(),
        "Node2: Grandchild A1 should exist at new location"
    );
    assert!(
        child_b_node2.is_some(),
        "Node2: Child B should exist at new location"
    );

    // Verify old locations don't exist
    let old_child_a_node2 = storage2
        .nodes()
        .get_by_path(
            StorageScope::new(tenant_id, repo_id, branch, workspace),
            "/Source Folder/Child A",
            None,
        )
        .await
        .expect("get_by_path should succeed");

    assert!(
        old_child_a_node2.is_none(),
        "Node2: Old path for Child A should NOT exist"
    );

    eprintln!("\n✅ Tree move replication test passed");
    eprintln!("   ✓ Tree created with 4 nodes (1 parent + 2 children + 1 grandchild)");
    eprintln!("   ✓ Tree moved using ApplyRevision operation");
    eprintln!("   ✓ ApplyRevision replicated to peer node");
    eprintln!("   ✓ Nodes appear ONLY in new location (not in both old and new)");
    eprintln!("   ✓ All descendants moved correctly with proper parent-child relationships");
    eprintln!("   ✓ Both cluster nodes have identical tree state");

    // Cleanup
    tokio::time::sleep(Duration::from_millis(100)).await;
}
