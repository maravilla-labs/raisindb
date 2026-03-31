// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Helper functions returning mock data for the MockFunctionApi

use serde_json::Value;

/// Create a mock node value
pub fn mock_node(workspace: &str, path: &str, id: &str, node_type: &str) -> Value {
    serde_json::json!({
        "id": id,
        "workspace": workspace,
        "path": path,
        "node_type": node_type,
        "properties": {}
    })
}

/// Create a mock node from creation data
pub fn mock_created_node(workspace: &str, parent_path: &str, data: &Value) -> Value {
    let name = data.get("name").and_then(|n| n.as_str()).unwrap_or("new");
    let default_type = serde_json::json!("raisin:Page");
    let node_type = data
        .get("type")
        .or(data.get("node_type"))
        .unwrap_or(&default_type);
    let default_props = serde_json::json!({});
    let properties = data.get("properties").unwrap_or(&default_props);

    serde_json::json!({
        "id": "new-mock-id",
        "workspace": workspace,
        "path": format!("{}/{}", parent_path, name),
        "node_type": node_type,
        "properties": properties
    })
}

/// Create a mock updated node
pub fn mock_updated_node(workspace: &str, path: &str, data: &Value) -> Value {
    let default_props = serde_json::json!({});
    let properties = data.get("properties").unwrap_or(&default_props);

    serde_json::json!({
        "id": "mock-node-id",
        "workspace": workspace,
        "path": path,
        "node_type": "raisin:Page",
        "properties": properties
    })
}

/// Create a mock moved node
pub fn mock_moved_node(workspace: &str, node_path: &str, new_parent_path: &str) -> Value {
    let node_name = node_path.split('/').next_back().unwrap_or("node");
    let new_path = if new_parent_path == "/" {
        format!("/{}", node_name)
    } else {
        format!("{}/{}", new_parent_path, node_name)
    };
    serde_json::json!({
        "id": "mock-node-id",
        "workspace": workspace,
        "path": new_path,
        "node_type": "raisin:Page",
        "properties": {}
    })
}

/// Create mock query results
pub fn mock_query_results(workspace: &str, query: &Value, limit: usize) -> Vec<Value> {
    let default_type = serde_json::json!("raisin:Page");
    let node_type = query.get("node_type").unwrap_or(&default_type);
    (0..limit.min(3))
        .map(|i| {
            serde_json::json!({
                "id": format!("mock-{}", i),
                "workspace": workspace,
                "path": format!("/mock-{}", i),
                "node_type": node_type,
                "properties": {}
            })
        })
        .collect()
}

/// Create mock children
pub fn mock_children(workspace: &str, parent_path: &str, count: usize) -> Vec<Value> {
    (0..count.min(3))
        .map(|i| {
            serde_json::json!({
                "id": format!("child-{}", i),
                "workspace": workspace,
                "path": format!("{}/child-{}", parent_path, i),
                "node_type": "raisin:Page",
                "properties": {}
            })
        })
        .collect()
}

/// Create mock AI completion response
pub fn mock_ai_completion(request: &Value) -> Value {
    let messages = request.get("messages").and_then(|m| m.as_array());

    let last_content = messages
        .and_then(|msgs| msgs.last())
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("");

    let model = request
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("gpt-4o");

    serde_json::json!({
        "message": {
            "role": "assistant",
            "content": format!("Mock AI response to: {}", last_content)
        },
        "model": model,
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 20,
            "total_tokens": 30
        },
        "stop_reason": "stop"
    })
}

/// Create a mock resource upload result
pub fn mock_resource_result(
    workspace: &str,
    node_path: &str,
    property_path: &str,
    upload_data: &Value,
) -> Value {
    let uuid = uuid::Uuid::new_v4().to_string();
    let name = upload_data
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or(property_path);
    let mime_type = upload_data
        .get("mimeType")
        .and_then(|m| m.as_str())
        .unwrap_or("application/octet-stream");

    serde_json::json!({
        "uuid": uuid,
        "name": name,
        "size": 1024,
        "mimeType": mime_type,
        "storageKey": format!("uploads/{}/{}/{}", workspace, node_path, uuid),
    })
}

/// Create a mock transactional node from creation data
pub fn mock_tx_created_node(workspace: &str, parent_path: &str, data: &Value) -> Value {
    let id = uuid::Uuid::new_v4().to_string();
    let name = data
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("new-node");
    let path = format!("{}/{}", parent_path, name);
    let default_type = serde_json::json!("raisin:Page");
    let node_type = data
        .get("node_type")
        .or(data.get("type"))
        .unwrap_or(&default_type);
    let default_props = serde_json::json!({});
    let properties = data.get("properties").unwrap_or(&default_props);

    serde_json::json!({
        "id": id,
        "workspace": workspace,
        "path": path,
        "name": name,
        "node_type": node_type,
        "properties": properties
    })
}

/// Create a mock transactional added node
pub fn mock_tx_added_node(workspace: &str, data: &Value) -> Value {
    let id = data
        .get("id")
        .and_then(|i| i.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let path = data
        .get("path")
        .and_then(|p| p.as_str())
        .unwrap_or("/mock-path");
    let default_type = serde_json::json!("raisin:Page");
    let node_type = data
        .get("node_type")
        .or(data.get("type"))
        .unwrap_or(&default_type);
    let default_props = serde_json::json!({});
    let properties = data.get("properties").unwrap_or(&default_props);

    serde_json::json!({
        "id": id,
        "workspace": workspace,
        "path": path,
        "node_type": node_type,
        "properties": properties
    })
}
