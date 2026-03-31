// Two-node replication test via REST API

mod helpers;

use helpers::multi_node::{authenticate, create_node, wait_for_node, ServerConfig, ServerHandle};
use serde_json::json;
use std::time::Duration;

#[tokio::test]
#[ignore] // Run with --include-ignored
async fn test_two_node_replication_via_rest_api() {
    println!("\n🧪 Testing Two-Node Replication via REST API\n");

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

    // Authenticate to both nodes
    let token1 = authenticate(&server1.base_url, "default", "admin", "admin123!@#")
        .await
        .expect("Failed to authenticate to node1");

    let token2 = authenticate(&server2.base_url, "default", "admin", "admin123!@#")
        .await
        .expect("Failed to authenticate to node2");

    println!("✅ Authenticated to both nodes");

    // Create a node on node1
    let node_id = "replicated-node-1";
    println!("\n📝 Creating node on node1...");
    create_node(
        &server1.base_url,
        &token1,
        "workspace",
        "main",
        "workspace",
        node_id,
        "Replicated Document 1",
        "Document",
        json!({
            "title": "Test Replication",
            "content": "This document should replicate to node2",
            "created_on": "node1"
        }),
    )
    .await
    .expect("Failed to create node on node1");

    println!("✅ Node created on node1: {}", node_id);

    // Wait for replication to node2
    println!("\n⏳ Waiting for replication to node2...");
    let replicated_node = wait_for_node(
        &server2.base_url,
        &token2,
        "workspace",
        "main",
        "workspace",
        node_id,
        Duration::from_secs(10),
    )
    .await
    .expect("Node did not replicate to node2");

    println!("✅ Node replicated to node2!");
    println!("   ID: {}", replicated_node["id"]);
    println!("   Name: {}", replicated_node["name"]);
    println!(
        "   Properties: {}",
        serde_json::to_string_pretty(&replicated_node["properties"]).unwrap()
    );

    // Verify data integrity
    assert_eq!(replicated_node["id"], node_id);
    assert_eq!(replicated_node["name"], "Replicated Document 1");
    assert_eq!(replicated_node["node_type"], "Document");
    assert_eq!(replicated_node["properties"]["title"], "Test Replication");
    assert_eq!(replicated_node["properties"]["created_on"], "node1");

    // Create a node on node2 and verify it replicates to node1
    let node_id_2 = "replicated-node-2";
    println!("\n📝 Creating node on node2...");
    create_node(
        &server2.base_url,
        &token2,
        "workspace",
        "main",
        "workspace",
        node_id_2,
        "Replicated Document 2",
        "Document",
        json!({
            "title": "Reverse Replication Test",
            "content": "This document should replicate to node1",
            "created_on": "node2"
        }),
    )
    .await
    .expect("Failed to create node on node2");

    println!("✅ Node created on node2: {}", node_id_2);

    // Wait for replication to node1
    println!("\n⏳ Waiting for replication to node1...");
    let replicated_node_2 = wait_for_node(
        &server1.base_url,
        &token1,
        "workspace",
        "main",
        "workspace",
        node_id_2,
        Duration::from_secs(10),
    )
    .await
    .expect("Node did not replicate to node1");

    println!("✅ Node replicated to node1!");
    assert_eq!(replicated_node_2["id"], node_id_2);
    assert_eq!(replicated_node_2["properties"]["created_on"], "node2");

    println!("\n✅ Two-node bidirectional replication test completed successfully!");
}
