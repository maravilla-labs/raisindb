//! FlowCallbacks trait implementation for RocksDBFlowCallbacks.
//!
//! Implements the `FlowCallbacks` trait from `raisin-flow-runtime`,
//! bridging each callback operation to the configured closure.

use async_trait::async_trait;
use raisin_flow_runtime::types::{
    AiCallContext, FlowCallbacks, FlowError, FlowExecutionEvent, FlowInstance, FlowResult,
};
use serde_json::Value;

use super::builder::RocksDBFlowCallbacks;

#[async_trait]
impl FlowCallbacks for RocksDBFlowCallbacks {
    async fn load_instance(&self, path: &str) -> FlowResult<FlowInstance> {
        tracing::debug!(path = %path, "Loading flow instance from storage");

        let loader = self
            .node_loader
            .as_ref()
            .ok_or_else(|| FlowError::Other("Node loader callback not configured".to_string()))?;

        let result = loader(
            self.tenant_id.clone(),
            self.repo_id.clone(),
            self.branch.clone(),
            self.flows_workspace.clone(),
            path.to_string(),
        )
        .await
        .map_err(|e| FlowError::Other(format!("Failed to load instance: {}", e)))?;

        let node = result
            .ok_or_else(|| FlowError::NodeNotFound(format!("Flow instance not found: {}", path)))?;

        // The loader returns the entire Node struct - extract the properties field
        // which contains the FlowInstance data
        let properties = node
            .get("properties")
            .ok_or_else(|| FlowError::Serialization("Node has no properties field".to_string()))?;

        // Deserialize properties to FlowInstance
        serde_json::from_value(properties.clone())
            .map_err(|e| FlowError::Serialization(format!("Failed to parse flow instance: {}", e)))
    }

    async fn save_instance(&self, instance: &FlowInstance) -> FlowResult<()> {
        tracing::debug!(instance_id = %instance.id, "Saving flow instance to storage");

        let path = self.instance_path(&instance.id);
        let properties = serde_json::to_value(instance).map_err(|e| {
            FlowError::Serialization(format!("Failed to serialize instance: {}", e))
        })?;

        // First check if the node exists
        let exists = if let Some(loader) = &self.node_loader {
            loader(
                self.tenant_id.clone(),
                self.repo_id.clone(),
                self.branch.clone(),
                self.flows_workspace.clone(),
                path.clone(),
            )
            .await
            .map_err(|e| FlowError::Other(format!("Failed to check if instance exists: {}", e)))?
            .is_some()
        } else {
            false
        };

        if exists {
            // Update existing node
            let saver = self.node_saver.as_ref().ok_or_else(|| {
                FlowError::Other("Node saver callback not configured".to_string())
            })?;

            saver(
                self.tenant_id.clone(),
                self.repo_id.clone(),
                self.branch.clone(),
                self.flows_workspace.clone(),
                path,
                properties,
            )
            .await
            .map_err(|e| FlowError::Other(format!("Failed to update instance: {}", e)))?;
        } else {
            // Create new node
            let creator = self.node_creator.as_ref().ok_or_else(|| {
                FlowError::Other("Node creator callback not configured".to_string())
            })?;

            creator(
                self.tenant_id.clone(),
                self.repo_id.clone(),
                self.branch.clone(),
                self.flows_workspace.clone(),
                "raisin:FlowInstance".to_string(),
                path,
                properties,
            )
            .await
            .map_err(|e| FlowError::Other(format!("Failed to create instance: {}", e)))?;
        }

        Ok(())
    }

    async fn save_instance_with_version(
        &self,
        instance: &FlowInstance,
        expected_version: i32,
    ) -> FlowResult<()> {
        tracing::debug!(
            instance_id = %instance.id,
            expected_version = expected_version,
            "Saving flow instance with version check"
        );

        // For now, we use a simple version check via node metadata
        // The node's _version field is used for OCC
        // In production, this would use RocksDB's CAS or transaction support
        let path = self.instance_path(&instance.id);

        // Load current version
        let current = self.load_instance(&path).await?;

        // Check version from the loaded instance's metadata
        // FlowInstance should track version internally
        let current_version = current.version;
        if current_version != expected_version {
            return Err(FlowError::VersionConflict);
        }

        // Update with new version
        let mut updated_instance = instance.clone();
        updated_instance.version = expected_version + 1;

        self.save_instance(&updated_instance).await
    }

    async fn create_node(
        &self,
        node_type: &str,
        path: &str,
        properties: Value,
    ) -> FlowResult<Value> {
        tracing::debug!(
            node_type = %node_type,
            path = %path,
            "Creating node from flow"
        );

        let creator = self
            .node_creator
            .as_ref()
            .ok_or_else(|| FlowError::Other("Node creator callback not configured".to_string()))?;

        creator(
            self.tenant_id.clone(),
            self.repo_id.clone(),
            self.branch.clone(),
            self.flows_workspace.clone(),
            node_type.to_string(),
            path.to_string(),
            properties,
        )
        .await
        .map_err(|e| FlowError::Other(format!("Failed to create node: {}", e)))
    }

    async fn update_node(&self, path: &str, properties: Value) -> FlowResult<Value> {
        tracing::debug!(path = %path, "Updating node from flow");

        let saver = self
            .node_saver
            .as_ref()
            .ok_or_else(|| FlowError::Other("Node saver callback not configured".to_string()))?;

        saver(
            self.tenant_id.clone(),
            self.repo_id.clone(),
            self.branch.clone(),
            self.flows_workspace.clone(),
            path.to_string(),
            properties.clone(),
        )
        .await
        .map_err(|e| FlowError::Other(format!("Failed to update node: {}", e)))?;

        Ok(properties)
    }

    async fn get_node(&self, path: &str) -> FlowResult<Option<Value>> {
        tracing::debug!(path = %path, "Getting node for flow");

        let loader = self
            .node_loader
            .as_ref()
            .ok_or_else(|| FlowError::Other("Node loader callback not configured".to_string()))?;

        loader(
            self.tenant_id.clone(),
            self.repo_id.clone(),
            self.branch.clone(),
            self.flows_workspace.clone(),
            path.to_string(),
        )
        .await
        .map_err(|e| FlowError::Other(format!("Failed to get node: {}", e)))
    }

    async fn list_children(&self, path: &str) -> FlowResult<Vec<Value>> {
        tracing::debug!(path = %path, "Listing children for flow");

        let lister = match self.children_lister.as_ref() {
            Some(l) => l,
            None => return Ok(Vec::new()), // No lister configured, return empty
        };

        lister(
            self.tenant_id.clone(),
            self.repo_id.clone(),
            self.branch.clone(),
            self.flows_workspace.clone(),
            path.to_string(),
        )
        .await
        .map_err(|e| FlowError::Other(format!("Failed to list children: {}", e)))
    }

    async fn queue_job(&self, job_type: &str, payload: Value) -> FlowResult<String> {
        tracing::debug!(job_type = %job_type, "Queuing job from flow");

        let queuer = self
            .job_queuer
            .as_ref()
            .ok_or_else(|| FlowError::Other("Job queuer callback not configured".to_string()))?;

        queuer(
            job_type.to_string(),
            payload,
            self.tenant_id.clone(),
            self.repo_id.clone(),
            self.branch.clone(),
            self.flows_workspace.clone(),
        )
        .await
        .map_err(|e| FlowError::Other(format!("Failed to queue job: {}", e)))
    }

    async fn call_ai(
        &self,
        agent_workspace: &str,
        agent_ref: &str,
        messages: Vec<Value>,
        response_format: Option<Value>,
    ) -> FlowResult<Value> {
        tracing::debug!(
            agent_workspace = %agent_workspace,
            agent_ref = %agent_ref,
            message_count = messages.len(),
            has_response_format = response_format.is_some(),
            "Calling AI from flow"
        );

        let caller = self.ai_caller.as_ref().ok_or_else(|| {
            FlowError::AIProvider("AI caller callback not configured".to_string())
        })?;

        let ctx = AiCallContext {
            tenant_id: self.tenant_id.clone(),
            repo_id: self.repo_id.clone(),
            branch: self.branch.clone(),
            workspace: agent_workspace.to_string(),
            agent_ref: agent_ref.to_string(),
        };

        caller(ctx, messages, response_format)
            .await
            .map_err(|e| FlowError::AIProvider(format!("AI call failed: {}", e)))
    }

    async fn call_ai_streaming(
        &self,
        agent_workspace: &str,
        agent_ref: &str,
        messages: Vec<Value>,
        response_format: Option<Value>,
    ) -> FlowResult<tokio::sync::mpsc::Receiver<Value>> {
        if let Some(caller) = &self.ai_streaming_caller {
            let ctx = AiCallContext {
                tenant_id: self.tenant_id.clone(),
                repo_id: self.repo_id.clone(),
                branch: self.branch.clone(),
                workspace: agent_workspace.to_string(),
                agent_ref: agent_ref.to_string(),
            };

            caller(ctx, messages, response_format)
                .await
                .map_err(|e| FlowError::AIProvider(format!("Streaming AI call failed: {}", e)))
        } else {
            // Fall back to default (non-streaming)
            let response = self
                .call_ai(agent_workspace, agent_ref, messages, response_format)
                .await?;
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            let _ = tx.send(response).await;
            Ok(rx)
        }
    }

    async fn create_node_in_workspace(
        &self,
        workspace: &str,
        node_type: &str,
        path: &str,
        properties: Value,
    ) -> FlowResult<Value> {
        tracing::debug!(
            workspace = %workspace,
            node_type = %node_type,
            path = %path,
            "Creating node in explicit workspace"
        );

        let creator = self
            .node_creator
            .as_ref()
            .ok_or_else(|| FlowError::Other("Node creator callback not configured".to_string()))?;

        creator(
            self.tenant_id.clone(),
            self.repo_id.clone(),
            self.branch.clone(),
            workspace.to_string(),
            node_type.to_string(),
            path.to_string(),
            properties,
        )
        .await
        .map_err(|e| FlowError::Other(format!("Failed to create node: {}", e)))
    }

    async fn get_node_in_workspace(
        &self,
        workspace: &str,
        path: &str,
    ) -> FlowResult<Option<Value>> {
        let loader = self
            .node_loader
            .as_ref()
            .ok_or_else(|| FlowError::Other("Node loader callback not configured".to_string()))?;

        loader(
            self.tenant_id.clone(),
            self.repo_id.clone(),
            self.branch.clone(),
            workspace.to_string(),
            path.to_string(),
        )
        .await
        .map_err(|e| FlowError::Other(format!("Failed to get node: {}", e)))
    }

    async fn list_children_in_workspace(
        &self,
        workspace: &str,
        path: &str,
    ) -> FlowResult<Vec<Value>> {
        tracing::debug!(
            workspace = %workspace,
            path = %path,
            "Listing children in explicit workspace"
        );

        let lister = match self.children_lister.as_ref() {
            Some(l) => l,
            None => return Ok(Vec::new()),
        };

        lister(
            self.tenant_id.clone(),
            self.repo_id.clone(),
            self.branch.clone(),
            workspace.to_string(),
            path.to_string(),
        )
        .await
        .map_err(|e| FlowError::Other(format!("Failed to list children: {}", e)))
    }

    async fn update_node_in_workspace(
        &self,
        workspace: &str,
        path: &str,
        properties: Value,
    ) -> FlowResult<Value> {
        let saver = self
            .node_saver
            .as_ref()
            .ok_or_else(|| FlowError::Other("Node saver callback not configured".to_string()))?;

        saver(
            self.tenant_id.clone(),
            self.repo_id.clone(),
            self.branch.clone(),
            workspace.to_string(),
            path.to_string(),
            properties.clone(),
        )
        .await
        .map_err(|e| FlowError::Other(format!("Failed to update node: {}", e)))?;

        Ok(properties)
    }

    async fn execute_function(&self, function_ref: &str, input: Value) -> FlowResult<Value> {
        tracing::debug!(function_ref = %function_ref, "Executing function from flow");

        let executor = self.function_executor.as_ref().ok_or_else(|| {
            FlowError::FunctionExecution("Function executor callback not configured".to_string())
        })?;

        executor(
            function_ref.to_string(),
            input,
            self.tenant_id.clone(),
            self.repo_id.clone(),
            self.branch.clone(),
            "functions".to_string(), // Functions are always in "functions" workspace
        )
        .await
        .map_err(|e| FlowError::FunctionExecution(format!("Function execution failed: {}", e)))
    }

    async fn emit_event(&self, instance_id: &str, event: FlowExecutionEvent) -> FlowResult<()> {
        // If no event emitter is configured, silently succeed (no-op)
        // This allows the flow to run without SSE streaming configured
        if let Some(emitter) = &self.event_emitter {
            tracing::trace!(
                instance_id = %instance_id,
                event_type = ?std::mem::discriminant(&event),
                "Emitting flow execution event"
            );

            emitter(instance_id.to_string(), event)
                .await
                .map_err(|e| FlowError::Other(format!("Failed to emit event: {}", e)))?;
        }
        Ok(())
    }
}
