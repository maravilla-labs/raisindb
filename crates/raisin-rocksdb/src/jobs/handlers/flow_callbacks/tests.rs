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

/// Decision (1): a flow-backed trigger must carry provenance to everything its
/// steps do.
///
/// `RocksDBFlowCallbacks` is the single funnel between the flow runtime and
/// storage / jobs / functions, so the marker it holds is what every egress point
/// stamps. Before this, none of these four callbacks received an identity at all
/// -- `function_callback.rs` literally passed `None // auth context` -- so a
/// flow-driven write was attributable to nobody.
mod agent_provenance {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// What a flow-backed trigger mints: the flow, composed with the trigger
    /// that started it. Exactly one `@`, kind leftmost so `LIKE 'flow:%'` works.
    const MARKER: &str = "flow:/flows/publish-approval@trigger:/triggers/on-order-created";

    fn recorder() -> (
        Arc<Mutex<Vec<Option<String>>>>,
        Arc<Mutex<Vec<Option<String>>>>,
    ) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        (seen.clone(), seen)
    }

    #[tokio::test]
    async fn every_egress_point_receives_the_flow_marker() {
        let (seen, sink) = recorder();

        let node_sink = sink.clone();
        let saver: NodeSaverCallback = Arc::new(move |_t, _r, _b, _w, _p, _props, agent| {
            node_sink.lock().unwrap().push(agent);
            Box::pin(async { Ok(()) })
        });

        let creator_sink = sink.clone();
        let creator: NodeCreatorCallback =
            Arc::new(move |_t, _r, _b, _w, _ty, _p, _props, agent| {
                creator_sink.lock().unwrap().push(agent);
                Box::pin(async { Ok(serde_json::json!({})) })
            });

        let queue_sink = sink.clone();
        let queuer: JobQueuerCallback = Arc::new(move |_j, _payload, _t, _r, _b, _w, agent| {
            queue_sink.lock().unwrap().push(agent);
            Box::pin(async { Ok("job-1".to_string()) })
        });

        let exec_sink = sink.clone();
        let executor: FunctionExecutorCallback =
            Arc::new(move |_f, _input, _t, _r, _b, _w, agent| {
                exec_sink.lock().unwrap().push(agent);
                Box::pin(async { Ok(serde_json::json!(null)) })
            });

        let callbacks = RocksDBFlowCallbacks::new(
            "tenant1".to_string(),
            "repo1".to_string(),
            "main".to_string(),
        )
        .with_agent(Some(MARKER.to_string()))
        .with_node_saver(saver)
        .with_node_creator(creator)
        .with_job_queuer(queuer)
        .with_function_executor(executor);

        callbacks
            .create_node("test:Doc", "/doc", serde_json::json!({}))
            .await
            .expect("create_node");
        callbacks
            .update_node("/doc", serde_json::json!({}))
            .await
            .expect("update_node");
        callbacks
            .queue_job("function_execution", serde_json::json!({}))
            .await
            .expect("queue_job");
        callbacks
            .execute_function("/lib/x", serde_json::json!({}))
            .await
            .expect("execute_function");

        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 4, "all four egress points must be exercised");
        for got in &seen {
            assert_eq!(
                got.as_deref(),
                Some(MARKER),
                "every flow egress point must carry the flow+trigger marker"
            );
        }
    }

    /// A flow with no marker must behave exactly as before: `None` everywhere,
    /// so nothing acquires an agent it did not have.
    #[tokio::test]
    async fn an_unmarked_flow_passes_no_agent() {
        let (seen, sink) = recorder();

        let creator_sink = sink.clone();
        let creator: NodeCreatorCallback =
            Arc::new(move |_t, _r, _b, _w, _ty, _p, _props, agent| {
                creator_sink.lock().unwrap().push(agent);
                Box::pin(async { Ok(serde_json::json!({})) })
            });

        let callbacks = RocksDBFlowCallbacks::new(
            "tenant1".to_string(),
            "repo1".to_string(),
            "main".to_string(),
        )
        .with_node_creator(creator);

        callbacks
            .create_node("test:Doc", "/doc", serde_json::json!({}))
            .await
            .expect("create_node");

        assert_eq!(seen.lock().unwrap().as_slice(), &[None]);
    }
}
