// Single-node CRUD test via REST API

mod helpers;

use helpers::multi_node::{
    authenticate, create_node, get_node_by_id, get_node_by_path, ServerConfig, ServerHandle,
};
use serde_json::json;

#[tokio::test]
#[ignore] // Run with --include-ignored
async fn test_single_node_crud_via_rest_api() {
    println!("\n🧪 Testing Single Node CRUD via REST API\n");

    // Start server
    let config = ServerConfig::new(8081);
    let server = ServerHandle::start(config)
        .await
        .expect("Failed to start server");

    println!("✅ Server started on port 8081");

    // Authenticate as admin
    let token = authenticate(&server.base_url, "default", "admin", "admin123!@#")
        .await
        .expect("Failed to authenticate");

    println!("✅ Authenticated successfully");

    // Create a test node
    let node_id = "test-node-1";
    let create_response = create_node(
        &server.base_url,
        &token,
        "workspace",
        "main",
        "workspace",
        node_id,
        "Test Node 1",
        "Document",
        json!({
            "title": "Test Document",
            "content": "This is a test document",
            "tags": ["test", "crud"]
        }),
    )
    .await
    .expect("Failed to create node");

    println!("✅ Created node: {}", node_id);
    println!(
        "   Response: {}",
        serde_json::to_string_pretty(&create_response).unwrap()
    );

    // Read node by ID
    let node_by_id = get_node_by_id(
        &server.base_url,
        &token,
        "workspace",
        "main",
        "workspace",
        node_id,
    )
    .await
    .expect("Failed to get node by ID")
    .expect("Node not found by ID");

    println!("✅ Retrieved node by ID");
    assert_eq!(node_by_id["id"], node_id);
    assert_eq!(node_by_id["name"], "Test Node 1");
    assert_eq!(node_by_id["node_type"], "Document");
    assert_eq!(node_by_id["properties"]["title"], "Test Document");

    // Read node by path
    let node_by_path = get_node_by_path(
        &server.base_url,
        &token,
        "workspace",
        "main",
        "workspace",
        "Test Node 1",
    )
    .await
    .expect("Failed to get node by path")
    .expect("Node not found by path");

    println!("✅ Retrieved node by path");
    assert_eq!(node_by_path["id"], node_id);
    assert_eq!(node_by_path["name"], "Test Node 1");

    // Create another node
    let node_id_2 = "test-node-2";
    create_node(
        &server.base_url,
        &token,
        "workspace",
        "main",
        "workspace",
        node_id_2,
        "Test Node 2",
        "Document",
        json!({
            "title": "Another Test Document",
            "content": "This is another test",
            "tags": ["test", "multiple"]
        }),
    )
    .await
    .expect("Failed to create second node");

    println!("✅ Created second node: {}", node_id_2);

    // Verify second node exists
    let node_2 = get_node_by_id(
        &server.base_url,
        &token,
        "workspace",
        "main",
        "workspace",
        node_id_2,
    )
    .await
    .expect("Failed to get second node")
    .expect("Second node not found");

    assert_eq!(node_2["id"], node_id_2);
    assert_eq!(node_2["properties"]["title"], "Another Test Document");

    println!("\n✅ Single-node CRUD test completed successfully!");
}
