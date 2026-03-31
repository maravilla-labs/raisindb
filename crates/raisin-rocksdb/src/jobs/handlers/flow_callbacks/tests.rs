//! Tests for flow callbacks.

use super::*;
use raisin_flow_runtime::types::{FlowCallbacks, FlowError};

#[test]
fn test_instance_path() {
    let callbacks = RocksDBFlowCallbacks::new(
        "tenant1".to_string(),
        "repo1".to_string(),
        "main".to_string(),
    );

    assert_eq!(callbacks.instance_path("abc123"), "/flows/instances/abc123");
}

#[test]
fn test_with_flows_workspace() {
    let callbacks = RocksDBFlowCallbacks::new(
        "tenant1".to_string(),
        "repo1".to_string(),
        "main".to_string(),
    )
    .with_flows_workspace("custom_flows".to_string());

    assert_eq!(callbacks.flows_workspace, "custom_flows");
}

#[tokio::test]
async fn test_load_instance_no_callback_returns_error() {
    let callbacks = RocksDBFlowCallbacks::new(
        "tenant1".to_string(),
        "repo1".to_string(),
        "main".to_string(),
    );

    let result = callbacks.load_instance("/flows/instances/test").await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), FlowError::Other(_)));
}

#[tokio::test]
async fn test_queue_job_no_callback_returns_error() {
    let callbacks = RocksDBFlowCallbacks::new(
        "tenant1".to_string(),
        "repo1".to_string(),
        "main".to_string(),
    );

    let result = callbacks.queue_job("test_job", serde_json::json!({})).await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), FlowError::Other(_)));
}

#[tokio::test]
async fn test_call_ai_no_callback_returns_error() {
    let callbacks = RocksDBFlowCallbacks::new(
        "tenant1".to_string(),
        "repo1".to_string(),
        "main".to_string(),
    );

    let result = callbacks
        .call_ai("functions", "/agents/test", vec![], None)
        .await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), FlowError::AIProvider(_)));
}
