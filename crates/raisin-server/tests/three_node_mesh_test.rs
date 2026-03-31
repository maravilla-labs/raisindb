// Three-node mesh replication test via REST API

mod helpers;

use helpers::multi_node::{authenticate, create_node, wait_for_node, ServerConfig, ServerHandle};
use serde_json::json;
use std::time::Duration;

#[tokio::test]
#[ignore] // Run with --include-ignored
async fn test_three_node_mesh_replication() {
    println!("\n🧪 Testing Three-Node Mesh Replication via REST API\n");

    // Start node 1
    let config1 = ServerConfig::new(8081).with_cluster("node1".to_string(), 9001);
    let server1 = ServerHandle::start(config1)
        .await
        .expect("Failed to start node1");
    println!("✅ Node1 started (HTTP: 8081, Replication: 9001)");

    // Start node 2
    let config2 = ServerConfig::new(8082).with_cluster("node2".to_string(), 9002);
    let server2 = ServerHandle::start(config2)
        .await
        .expect("Failed to start node2");
    println!("✅ Node2 started (HTTP: 8082, Replication: 9002)");

    // Start node 3
    let config3 = ServerConfig::new(8083).with_cluster("node3".to_string(), 9003);
    let server3 = ServerHandle::start(config3)
        .await
        .expect("Failed to start node3");
    println!("✅ Node3 started (HTTP: 8083, Replication: 9003)");

    // Authenticate to all nodes
    let token1 = authenticate(&server1.base_url, "default", "admin", "admin123!@#")
        .await
        .expect("Failed to authenticate to node1");

    let token2 = authenticate(&server2.base_url, "default", "admin", "admin123!@#")
        .await
        .expect("Failed to authenticate to node2");

    let token3 = authenticate(&server3.base_url, "default", "admin", "admin123!@#")
        .await
        .expect("Failed to authenticate to node3");

    println!("✅ Authenticated to all three nodes\n");

    // Create a node on each server
    println!("📝 Creating nodes on all three servers...");

    // Node from node1
    let node_id_1 = "mesh-node-from-1";
    create_node(
        &server1.base_url,
        &token1,
        "workspace",
        "main",
        "workspace",
        node_id_1,
        "Document from Node1",
        "Document",
        json!({
            "title": "Created on Node1",
            "origin": "node1",
            "timestamp": chrono::Utc::now().to_rfc3339()
        }),
    )
    .await
    .expect("Failed to create node on node1");
    println!("  ✓ Created {} on node1", node_id_1);

    // Node from node2
    let node_id_2 = "mesh-node-from-2";
    create_node(
        &server2.base_url,
        &token2,
        "workspace",
        "main",
        "workspace",
        node_id_2,
        "Document from Node2",
        "Document",
        json!({
            "title": "Created on Node2",
            "origin": "node2",
            "timestamp": chrono::Utc::now().to_rfc3339()
        }),
    )
    .await
    .expect("Failed to create node on node2");
    println!("  ✓ Created {} on node2", node_id_2);

    // Node from node3
    let node_id_3 = "mesh-node-from-3";
    create_node(
        &server3.base_url,
        &token3,
        "workspace",
        "main",
        "workspace",
        node_id_3,
        "Document from Node3",
        "Document",
        json!({
            "title": "Created on Node3",
            "origin": "node3",
            "timestamp": chrono::Utc::now().to_rfc3339()
        }),
    )
    .await
    .expect("Failed to create node on node3");
    println!("  ✓ Created {} on node3", node_id_3);

    println!("\n⏳ Waiting for mesh replication...");

    // Verify node1's document appears on node2 and node3
    println!("\n  Checking node1 → node2, node3...");
    wait_for_node(
        &server2.base_url,
        &token2,
        "workspace",
        "main",
        "workspace",
        node_id_1,
        Duration::from_secs(15),
    )
    .await
    .expect("Node1's document did not replicate to node2");

    wait_for_node(
        &server3.base_url,
        &token3,
        "workspace",
        "main",
        "workspace",
        node_id_1,
        Duration::from_secs(15),
    )
    .await
    .expect("Node1's document did not replicate to node3");
    println!("    ✅ node1 → node2, node3");

    // Verify node2's document appears on node1 and node3
    println!("  Checking node2 → node1, node3...");
    wait_for_node(
        &server1.base_url,
        &token1,
        "workspace",
        "main",
        "workspace",
        node_id_2,
        Duration::from_secs(15),
    )
    .await
    .expect("Node2's document did not replicate to node1");

    wait_for_node(
        &server3.base_url,
        &token3,
        "workspace",
        "main",
        "workspace",
        node_id_2,
        Duration::from_secs(15),
    )
    .await
    .expect("Node2's document did not replicate to node3");
    println!("    ✅ node2 → node1, node3");

    // Verify node3's document appears on node1 and node2
    println!("  Checking node3 → node1, node2...");
    wait_for_node(
        &server1.base_url,
        &token1,
        "workspace",
        "main",
        "workspace",
        node_id_3,
        Duration::from_secs(15),
    )
    .await
    .expect("Node3's document did not replicate to node1");

    wait_for_node(
        &server2.base_url,
        &token2,
        "workspace",
        "main",
        "workspace",
        node_id_3,
        Duration::from_secs(15),
    )
    .await
    .expect("Node3's document did not replicate to node2");
    println!("    ✅ node3 → node1, node2");

    println!("\n✅ Three-node mesh replication test completed successfully!");
    println!("   All nodes have all 3 documents!");
}
