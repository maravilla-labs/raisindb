//! RocksDBFlowCallbacks struct and builder pattern.
//!
//! Contains the struct definition and builder methods for configuring
//! the flow callbacks with the various operation callbacks.

use super::types::*;

/// Implementation of FlowCallbacks for RocksDB storage.
///
/// This provides the bridge between the flow runtime and actual storage/job
/// operations. It uses callbacks provided by the transport layer for the
/// actual implementations.
pub struct RocksDBFlowCallbacks {
    /// Tenant ID for this flow execution
    pub tenant_id: String,

    /// Repository ID for this flow execution
    pub repo_id: String,

    /// Branch name for this flow execution
    pub branch: String,

    /// Workspace where flow instances are stored (default: "flows")
    pub flows_workspace: String,

    /// Callback for loading nodes
    pub(super) node_loader: Option<NodeLoaderCallback>,

    /// Callback for saving nodes (update)
    pub(super) node_saver: Option<NodeSaverCallback>,

    /// Callback for creating nodes
    pub(super) node_creator: Option<NodeCreatorCallback>,

    /// Callback for queuing jobs
    pub(super) job_queuer: Option<JobQueuerCallback>,

    /// Callback for AI calls
    pub(super) ai_caller: Option<AICallerCallback>,

    /// Callback for streaming AI calls
    pub(super) ai_streaming_caller: Option<AIStreamingCallerCallback>,

    /// Callback for function execution
    pub(super) function_executor: Option<FunctionExecutorCallback>,

    /// Callback for listing children of a node
    pub(super) children_lister: Option<ChildrenListerCallback>,

    /// Callback for emitting flow execution events (for SSE streaming)
    pub(super) event_emitter: Option<FlowEventEmitterCallback>,
}

impl RocksDBFlowCallbacks {
    /// Create new callbacks with required context
    pub fn new(tenant_id: String, repo_id: String, branch: String) -> Self {
        Self {
            tenant_id,
            repo_id,
            branch,
            flows_workspace: "raisin:system".to_string(),
            node_loader: None,
            node_saver: None,
            node_creator: None,
            job_queuer: None,
            ai_caller: None,
            ai_streaming_caller: None,
            function_executor: None,
            children_lister: None,
            event_emitter: None,
        }
    }

    /// Set a custom workspace for flow instances
    pub fn with_flows_workspace(mut self, workspace: String) -> Self {
        self.flows_workspace = workspace;
        self
    }

    /// Set the node loader callback
    pub fn with_node_loader(mut self, loader: NodeLoaderCallback) -> Self {
        self.node_loader = Some(loader);
        self
    }

    /// Set the node saver callback
    pub fn with_node_saver(mut self, saver: NodeSaverCallback) -> Self {
        self.node_saver = Some(saver);
        self
    }

    /// Set the node creator callback
    pub fn with_node_creator(mut self, creator: NodeCreatorCallback) -> Self {
        self.node_creator = Some(creator);
        self
    }

    /// Set the job queuer callback
    pub fn with_job_queuer(mut self, queuer: JobQueuerCallback) -> Self {
        self.job_queuer = Some(queuer);
        self
    }

    /// Set the AI caller callback
    pub fn with_ai_caller(mut self, caller: AICallerCallback) -> Self {
        self.ai_caller = Some(caller);
        self
    }

    /// Set the streaming AI caller callback
    pub fn with_ai_streaming_caller(mut self, caller: AIStreamingCallerCallback) -> Self {
        self.ai_streaming_caller = Some(caller);
        self
    }

    /// Set the function executor callback
    pub fn with_function_executor(mut self, executor: FunctionExecutorCallback) -> Self {
        self.function_executor = Some(executor);
        self
    }

    /// Set the children lister callback
    pub fn with_children_lister(mut self, lister: ChildrenListerCallback) -> Self {
        self.children_lister = Some(lister);
        self
    }

    /// Set the flow event emitter callback for SSE streaming
    pub fn with_event_emitter(mut self, emitter: FlowEventEmitterCallback) -> Self {
        self.event_emitter = Some(emitter);
        self
    }

    /// Build path for a flow instance
    pub(super) fn instance_path(&self, instance_id: &str) -> String {
        format!("/flows/instances/{}", instance_id)
    }
}
