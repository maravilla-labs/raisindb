// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Callback type definitions for flow execution

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub use raisin_flow_runtime::types::AiCallContext;

/// Type for async node loading callback
pub type NodeLoaderCallback = Arc<
    dyn Fn(
            String,
            String,
            String,
            String,
            String,
        )
            -> Pin<Box<dyn Future<Output = Result<Option<serde_json::Value>, String>> + Send>>
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
            serde_json::Value,
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
            serde_json::Value,
        ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, String>> + Send>>
        + Send
        + Sync,
>;

/// Type for async job queuing callback
pub type JobQueuerCallback = Arc<
    dyn Fn(
            String,
            serde_json::Value,
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
            Vec<serde_json::Value>,
            Option<serde_json::Value>,
        ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, String>> + Send>>
        + Send
        + Sync,
>;

/// Type for streaming AI call callback
/// Parameters: (context, messages, response_format)
/// Returns a channel receiver that yields streaming chunks as JSON values.
pub type AIStreamingCallerCallback = Arc<
    dyn Fn(
            AiCallContext,
            Vec<serde_json::Value>,
            Option<serde_json::Value>,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<tokio::sync::mpsc::Receiver<serde_json::Value>, String>>
                    + Send,
            >,
        > + Send
        + Sync,
>;

/// Type for async function execution callback
pub type FunctionExecutorCallback = Arc<
    dyn Fn(
            String,
            serde_json::Value,
            String,
            String,
            String,
            String,
        ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, String>> + Send>>
        + Send
        + Sync,
>;

/// Type for async children listing callback
/// Parameters: (tenant_id, repo_id, branch, workspace, parent_path)
/// Returns the child nodes as JSON values
pub type ChildrenListerCallback = Arc<
    dyn Fn(
            String,
            String,
            String,
            String,
            String,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<serde_json::Value>, String>> + Send>>
        + Send
        + Sync,
>;

/// Collection of flow execution callbacks
pub struct FlowCallbacks {
    pub node_loader: NodeLoaderCallback,
    pub node_saver: NodeSaverCallback,
    pub node_creator: NodeCreatorCallback,
    pub job_queuer: JobQueuerCallback,
    pub ai_caller: AICallerCallback,
    pub ai_streaming_caller: Option<AIStreamingCallerCallback>,
    pub function_executor: FunctionExecutorCallback,
    pub children_lister: ChildrenListerCallback,
}
