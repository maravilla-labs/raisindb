//! Callback type aliases for flow operations.
//!
//! Defines the async callback types used by `RocksDBFlowCallbacks` for
//! node loading, saving, creation, job queuing, AI calls, and event emission.

use raisin_flow_runtime::types::{AiCallContext, FlowExecutionEvent};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Type for async node loading callback
pub type NodeLoaderCallback = Arc<
    dyn Fn(
            String,
            String,
            String,
            String,
            String,
        ) -> Pin<Box<dyn Future<Output = Result<Option<Value>, String>> + Send>>
        + Send
        + Sync,
>;

/// Type for async node saving callback
pub type NodeSaverCallback = Arc<
    dyn Fn(
            String,
            String,
            String,
            String,
            String,
            Value,
        ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>>
        + Send
        + Sync,
>;

/// Type for async node creation callback
pub type NodeCreatorCallback = Arc<
    dyn Fn(
            String,
            String,
            String,
            String,
            String,
            String,
            Value,
        ) -> Pin<Box<dyn Future<Output = Result<Value, String>> + Send>>
        + Send
        + Sync,
>;

/// Type for async job queuing callback
pub type JobQueuerCallback = Arc<
    dyn Fn(
            String,
            Value,
            String,
            String,
            String,
            String,
        ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>>
        + Send
        + Sync,
>;

/// Type for async AI call callback
/// Parameters: (context, messages, response_format)
pub type AICallerCallback = Arc<
    dyn Fn(
            AiCallContext,
            Vec<Value>,
            Option<Value>,
        ) -> Pin<Box<dyn Future<Output = Result<Value, String>> + Send>>
        + Send
        + Sync,
>;

/// Type for streaming AI call callback
/// Parameters: (context, messages, response_format)
/// Returns a channel receiver that yields streaming chunks as JSON values.
pub type AIStreamingCallerCallback = Arc<
    dyn Fn(
            AiCallContext,
            Vec<Value>,
            Option<Value>,
        ) -> Pin<
            Box<dyn Future<Output = Result<tokio::sync::mpsc::Receiver<Value>, String>> + Send>,
        > + Send
        + Sync,
>;

/// Type for async function execution callback
pub type FunctionExecutorCallback = Arc<
    dyn Fn(
            String,
            Value,
            String,
            String,
            String,
            String,
        ) -> Pin<Box<dyn Future<Output = Result<Value, String>> + Send>>
        + Send
        + Sync,
>;

/// Type for async children listing callback
/// Parameters: (tenant_id, repo_id, branch, workspace, path)
/// Returns the child nodes as JSON values
pub type ChildrenListerCallback = Arc<
    dyn Fn(
            String,
            String,
            String,
            String,
            String,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<Value>, String>> + Send>>
        + Send
        + Sync,
>;

/// Type for flow event emitter callback
/// Parameters: (instance_id, event)
/// Used for real-time step-level event streaming to SSE clients
pub type FlowEventEmitterCallback = Arc<
    dyn Fn(String, FlowExecutionEvent) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>>
        + Send
        + Sync,
>;
