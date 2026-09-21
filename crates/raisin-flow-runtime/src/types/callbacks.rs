// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Callback trait for flow runtime to interact with storage and external systems.

use async_trait::async_trait;
use serde_json::Value;

use super::{FlowExecutionEvent, FlowInstance, FlowResult};

/// Named context for AI callback invocations.
///
/// Groups the five positional `String` parameters (`tenant_id`, `repo_id`,
/// `branch`, `workspace`, `agent_ref`) into a single typed struct, making
/// call sites self-documenting.
#[derive(Clone, Debug, Default)]
pub struct AiCallContext {
    pub tenant_id: String,
    pub repo_id: String,
    pub branch: String,
    pub workspace: String,
    pub agent_ref: String,

    /// Extra tool definitions to offer the model for THIS CALL ONLY, appended
    /// to the tools the agent node declares.
    ///
    /// This is how CONTROL TOOLS reach the provider. An agent's own `tools`
    /// are functions to execute; these are verbs the RUNTIME implements —
    /// ending a chat with a result, handing off, asking a human — and they
    /// belong to the step rather than the agent, because the same agent ends
    /// a triage chat and a drafting chat with different payloads.
    ///
    /// Carried on the context rather than as a sixth callback parameter so the
    /// `AICallerCallback` / `AIStreamingCallerCallback` type aliases (and every
    /// mock built against them) keep their shape.
    pub extra_tools: Vec<Value>,

    /// Skill references (`raisin:Skill`, the `{raisin:ref, raisin:workspace}`
    /// envelope an agent's `skills:` uses) a workflow STEP adds to its agent's
    /// own skills, for THIS CALL ONLY — so an automation can give one step a
    /// skill without editing the agent. Empty for every other caller.
    ///
    /// Carried on the context for the same reason as `extra_tools`.
    pub skills: Vec<Value>,

    /// Turns skills ON for this call: the capped skills index in the system
    /// prompt, the `load-skill` tool, and `_skill_grant` in the response.
    ///
    /// OFF by default, and set only by a caller that RUNS A TOOL LOOP and
    /// hands `load-skill` its grant (`agent_step`, `ai_tool_loop` / chat step,
    /// the AI container — reached through `call_ai_with_options` /
    /// `call_ai_streaming_with_options`).
    /// Every other caller — the decision, competition and agent-assignee
    /// steps — makes one call and expects its structured
    /// answer back. Offered a skill there, a model loads it: the step gets a
    /// `tool_call` it cannot execute instead of its decision. So skills stay
    /// off unless a caller says it can use them, and a global skill appearing
    /// on the server never changes what a decision step is asked.
    pub offer_skills: bool,
}

/// Callbacks provided by the transport/storage layer to the flow runtime.
///
/// This trait abstracts away storage operations, AI calls, and job queueing,
/// allowing the runtime to be storage-agnostic.
#[async_trait]
pub trait FlowCallbacks: Send + Sync {
    /// Load a flow instance from storage by path
    async fn load_instance(&self, path: &str) -> FlowResult<FlowInstance>;

    /// Save a flow instance to storage
    async fn save_instance(&self, instance: &FlowInstance) -> FlowResult<()>;

    /// Save instance with version check (OCC)
    async fn save_instance_with_version(
        &self,
        instance: &FlowInstance,
        expected_version: i32,
    ) -> FlowResult<()>;

    /// Create a node in the database
    async fn create_node(
        &self,
        node_type: &str,
        path: &str,
        properties: Value,
    ) -> FlowResult<Value>;

    /// Update a node's properties
    async fn update_node(&self, path: &str, properties: Value) -> FlowResult<Value>;

    /// Get a node by path
    async fn get_node(&self, path: &str) -> FlowResult<Option<Value>>;

    /// List children of a node at the given path
    ///
    /// Returns the child nodes as JSON values, each containing at minimum
    /// a `properties` object. Used by the AI container to load conversation
    /// history from AIMessage children.
    ///
    /// Default implementation returns an empty vec for backward compatibility.
    async fn list_children(&self, _path: &str) -> FlowResult<Vec<Value>> {
        Ok(Vec::new())
    }

    /// Queue a job for asynchronous execution
    async fn queue_job(&self, job_type: &str, payload: Value) -> FlowResult<String>;

    /// Queue a job to be executed at (not before) a future time.
    ///
    /// Used for wait timeouts, retry backoffs, and scheduled delays. The
    /// default implementation embeds the schedule in the payload as
    /// `__scheduled_at` (RFC 3339) and delegates to `queue_job`; the job
    /// queuer implementation is responsible for honoring it.
    async fn queue_job_at(
        &self,
        job_type: &str,
        mut payload: Value,
        scheduled_at: chrono::DateTime<chrono::Utc>,
    ) -> FlowResult<String> {
        if let Value::Object(ref mut map) = payload {
            map.insert(
                "__scheduled_at".to_string(),
                Value::String(scheduled_at.to_rfc3339()),
            );
        }
        self.queue_job(job_type, payload).await
    }

    /// Call an AI provider
    ///
    /// # Arguments
    /// * `agent_workspace` - Workspace where the agent is stored (e.g., "functions")
    /// * `agent_ref` - Path to the agent node within the workspace
    /// * `messages` - Conversation messages to send
    /// * `response_format` - Optional structured output configuration (format type + schema)
    async fn call_ai(
        &self,
        agent_workspace: &str,
        agent_ref: &str,
        messages: Vec<Value>,
        response_format: Option<Value>,
    ) -> FlowResult<Value>;

    /// Call an AI provider with streaming response.
    ///
    /// Returns a channel receiver that yields stream-chunk JSON values.
    /// The callback implementation spawns a task that calls the provider's
    /// `stream_complete()` and forwards chunks to the sender.
    ///
    /// Default implementation falls back to non-streaming `call_ai()` and
    /// sends the complete response as a single chunk.
    async fn call_ai_streaming(
        &self,
        agent_workspace: &str,
        agent_ref: &str,
        messages: Vec<Value>,
        response_format: Option<Value>,
    ) -> FlowResult<tokio::sync::mpsc::Receiver<Value>> {
        // Default: fall back to non-streaming
        let response = self
            .call_ai(agent_workspace, agent_ref, messages, response_format)
            .await?;
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let _ = tx.send(response).await;
        Ok(rx)
    }

    /// Call an AI provider, offering `extra_tools` alongside the agent's own.
    ///
    /// `extra_tools` are CONTROL TOOLS — see [`AiCallContext::extra_tools`].
    /// The default implementation drops them and delegates to [`call_ai`],
    /// which is correct for a callback with no provider behind it (every mock
    /// in the test suite): a model that is never offered a control tool simply
    /// never calls one, and the tool-loop's interception side is unaffected.
    ///
    /// [`call_ai`]: FlowCallbacks::call_ai
    async fn call_ai_with_tools(
        &self,
        agent_workspace: &str,
        agent_ref: &str,
        messages: Vec<Value>,
        response_format: Option<Value>,
        extra_tools: Vec<Value>,
    ) -> FlowResult<Value> {
        let _ = extra_tools;
        self.call_ai(agent_workspace, agent_ref, messages, response_format)
            .await
    }

    /// Streaming counterpart of [`call_ai_with_tools`].
    ///
    /// [`call_ai_with_tools`]: FlowCallbacks::call_ai_with_tools
    async fn call_ai_streaming_with_tools(
        &self,
        agent_workspace: &str,
        agent_ref: &str,
        messages: Vec<Value>,
        response_format: Option<Value>,
        extra_tools: Vec<Value>,
    ) -> FlowResult<tokio::sync::mpsc::Receiver<Value>> {
        let _ = extra_tools;
        self.call_ai_streaming(agent_workspace, agent_ref, messages, response_format)
            .await
    }

    /// Call an AI provider with the step-level options: control tools
    /// (`extra_tools`) and the step's own `skills`, added to the agent's.
    ///
    /// The default implementation drops `skills` and delegates to
    /// [`call_ai_with_tools`], so a callback with no provider behind it (every
    /// mock) is unchanged. The real one carries both on [`AiCallContext`].
    ///
    /// This is the TOOL-LOOP entry point: the real implementation sets
    /// [`AiCallContext::offer_skills`], so the agent's skills (index +
    /// `load-skill`) are offered only here. A caller that cannot execute a
    /// tool call must use [`call_ai`] / [`call_ai_with_tools`] instead.
    ///
    /// [`call_ai`]: FlowCallbacks::call_ai
    /// [`call_ai_with_tools`]: FlowCallbacks::call_ai_with_tools
    async fn call_ai_with_options(
        &self,
        agent_workspace: &str,
        agent_ref: &str,
        messages: Vec<Value>,
        response_format: Option<Value>,
        extra_tools: Vec<Value>,
        skills: Vec<Value>,
    ) -> FlowResult<Value> {
        let _ = skills;
        self.call_ai_with_tools(
            agent_workspace,
            agent_ref,
            messages,
            response_format,
            extra_tools,
        )
        .await
    }

    /// Streaming counterpart of [`call_ai_with_options`] — also a tool-loop
    /// entry point, so it offers skills too.
    ///
    /// [`call_ai_with_options`]: FlowCallbacks::call_ai_with_options
    async fn call_ai_streaming_with_options(
        &self,
        agent_workspace: &str,
        agent_ref: &str,
        messages: Vec<Value>,
        response_format: Option<Value>,
        extra_tools: Vec<Value>,
        skills: Vec<Value>,
    ) -> FlowResult<tokio::sync::mpsc::Receiver<Value>> {
        let _ = skills;
        self.call_ai_streaming_with_tools(
            agent_workspace,
            agent_ref,
            messages,
            response_format,
            extra_tools,
        )
        .await
    }

    /// Execute a function synchronously
    async fn execute_function(&self, function_ref: &str, input: Value) -> FlowResult<Value>;

    /// Execute a function AS a named AGENT — an agent's own tool call.
    ///
    /// Defaults to [`Self::execute_function`], so an implementation with no
    /// notion of agent identity is unchanged. The real one composes the agent
    /// onto the flow's marker and lets the agent's `execution_context` decide
    /// whose permissions apply: without this, an agent restricted in the UI
    /// still ran with full system rights the moment it was called from inside a
    /// flow, which is the opposite of what its configuration says.
    async fn execute_function_as_agent(
        &self,
        function_ref: &str,
        input: Value,
        agent_path: &str,
    ) -> FlowResult<Value> {
        let _ = agent_path;
        self.execute_function(function_ref, input).await
    }

    // === Workspace-Aware Node Operations ===
    //
    // These methods allow creating/reading/updating nodes in a specific workspace,
    // independent of the default flows_workspace. Used by conversation persistence
    // to store user conversations in `raisin:access_control` for proper access control.

    /// Create a node in an explicit workspace
    ///
    /// Default delegates to `create_node` (ignoring workspace).
    /// RocksDB overrides to route to the specified workspace.
    async fn create_node_in_workspace(
        &self,
        _workspace: &str,
        node_type: &str,
        path: &str,
        properties: Value,
    ) -> FlowResult<Value> {
        self.create_node(node_type, path, properties).await
    }

    /// Get a node by path from an explicit workspace
    async fn get_node_in_workspace(
        &self,
        _workspace: &str,
        path: &str,
    ) -> FlowResult<Option<Value>> {
        self.get_node(path).await
    }

    /// List children of a node in an explicit workspace
    ///
    /// Default delegates to `list_children` (ignoring workspace).
    /// RocksDB overrides to route to the specified workspace.
    async fn list_children_in_workspace(
        &self,
        _workspace: &str,
        path: &str,
    ) -> FlowResult<Vec<Value>> {
        self.list_children(path).await
    }

    /// Update a node in an explicit workspace.
    ///
    /// The properties REPLACE the node's properties. To change a few fields of
    /// a node whose other properties must survive (an inbox task's `status`,
    /// `flow_instance_id`, `options`, ...), use `patch_node_in_workspace`.
    async fn update_node_in_workspace(
        &self,
        _workspace: &str,
        path: &str,
        properties: Value,
    ) -> FlowResult<Value> {
        self.update_node(path, properties).await
    }

    /// Merge `patch` into a node's existing properties and write the result.
    ///
    /// Reads the node first so the write carries everything it already had.
    /// Escalating an inbox task used to call `update_node_in_workspace` with
    /// only the escalation fields, which replaced the whole property set: the
    /// task lost its `status`, `flow_instance_id`, `title` and `options`,
    /// vanished from the pending list, and could no longer resume its flow.
    async fn patch_node_in_workspace(
        &self,
        workspace: &str,
        path: &str,
        patch: Value,
    ) -> FlowResult<Value> {
        let mut merged = self
            .get_node_in_workspace(workspace, path)
            .await?
            .and_then(|node| node.get("properties").cloned())
            .filter(Value::is_object)
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
        if let (Some(dst), Some(src)) = (merged.as_object_mut(), patch.as_object()) {
            for (key, value) in src {
                dst.insert(key.clone(), value.clone());
            }
        }
        self.update_node_in_workspace(workspace, path, merged).await
    }

    /// Emit a flow execution event for real-time tracking
    ///
    /// This enables step-level visibility into flow execution, supporting:
    /// - Canvas highlighting (show which step is executing)
    /// - Timeline visualization (step progress, variables, logs)
    /// - Execution monitoring and debugging
    ///
    /// Default implementation is a no-op. Storage backends that support
    /// real-time event streaming should override this method.
    async fn emit_event(&self, _instance_id: &str, _event: FlowExecutionEvent) -> FlowResult<()> {
        // Default no-op - backends override to enable event streaming
        Ok(())
    }

    // === Isolated Branch Operations (for AI safety) ===

    /// Create an isolated branch for step execution
    ///
    /// Creates a new branch from the current HEAD (or specified base branch).
    /// Returns the branch name that was created.
    ///
    /// # Arguments
    /// * `branch_name` - Name for the new branch (e.g., "flow-step-{step_id}")
    /// * `base_branch` - Optional base branch to fork from (defaults to current branch)
    async fn create_branch(
        &self,
        _branch_name: &str,
        _base_branch: Option<&str>,
    ) -> FlowResult<String> {
        // Default: not supported
        Err(super::FlowError::NotSupported(
            "Isolated branch mode not supported by this storage backend".to_string(),
        ))
    }

    /// Merge an isolated branch back to the target branch
    ///
    /// Merges the specified branch into the target (or current) branch.
    /// Returns error if there are merge conflicts.
    ///
    /// # Arguments
    /// * `branch_name` - Branch to merge from
    /// * `target_branch` - Optional target branch (defaults to original branch)
    async fn merge_branch(
        &self,
        _branch_name: &str,
        _target_branch: Option<&str>,
    ) -> FlowResult<()> {
        // Default: not supported
        Err(super::FlowError::NotSupported(
            "Isolated branch mode not supported by this storage backend".to_string(),
        ))
    }

    /// Delete an isolated branch
    ///
    /// Removes the branch without merging. Used for cleanup after failed steps
    /// or when explicitly discarding changes.
    ///
    /// # Arguments
    /// * `branch_name` - Branch to delete
    async fn delete_branch(&self, _branch_name: &str) -> FlowResult<()> {
        // Default: not supported
        Err(super::FlowError::NotSupported(
            "Isolated branch mode not supported by this storage backend".to_string(),
        ))
    }

    /// Check if merging would result in conflicts
    ///
    /// # Arguments
    /// * `branch_name` - Branch to check for conflicts
    /// * `target_branch` - Optional target branch (defaults to original branch)
    ///
    /// # Returns
    /// * `Ok(true)` - Merge would have conflicts
    /// * `Ok(false)` - Merge would succeed without conflicts
    async fn has_merge_conflicts(
        &self,
        _branch_name: &str,
        _target_branch: Option<&str>,
    ) -> FlowResult<bool> {
        // Default: not supported
        Err(super::FlowError::NotSupported(
            "Isolated branch mode not supported by this storage backend".to_string(),
        ))
    }

    /// Switch to a different branch for subsequent operations
    ///
    /// # Arguments
    /// * `branch_name` - Branch to switch to
    async fn switch_branch(&self, _branch_name: &str) -> FlowResult<()> {
        // Default: not supported
        Err(super::FlowError::NotSupported(
            "Isolated branch mode not supported by this storage backend".to_string(),
        ))
    }

    /// Get the current branch name
    async fn current_branch(&self) -> FlowResult<String> {
        // Default: not supported
        Err(super::FlowError::NotSupported(
            "Isolated branch mode not supported by this storage backend".to_string(),
        ))
    }

    // === Security & Permission Operations ===

    /// Validate execution identity has permission for an operation
    ///
    /// Checks if the specified execution identity (agent, caller, or function)
    /// has permission to perform the requested operation on the target path.
    ///
    /// # Arguments
    /// * `identity_mode` - The execution identity mode (agent, caller, function)
    /// * `operation` - The operation being performed (e.g., "read", "write", "execute")
    /// * `target_path` - The path being accessed
    /// * `caller_id` - Optional ID of the original caller (for caller identity mode)
    ///
    /// # Returns
    /// * `Ok(true)` - Operation is permitted
    /// * `Ok(false)` - Operation is denied
    async fn validate_permission(
        &self,
        _identity_mode: &str,
        _operation: &str,
        _target_path: &str,
        _caller_id: Option<&str>,
    ) -> FlowResult<bool> {
        // Default: allow all operations (no permission system configured)
        // Production implementations should override with proper permission checks
        Ok(true)
    }

    /// Log a security-relevant audit event
    ///
    /// Creates an audit trail for sensitive operations like:
    /// - Identity escalation (caller -> function)
    /// - Access to sensitive paths
    /// - Failed permission checks
    /// - External API calls
    ///
    /// # Arguments
    /// * `event_type` - Type of audit event (e.g., "permission_check", "identity_escalation")
    /// * `details` - Additional event details
    async fn audit_log(&self, _event_type: &str, _details: Value) -> FlowResult<()> {
        // Default: no-op (audit logging not configured)
        // Production implementations should override with proper audit logging
        Ok(())
    }

    /// Get the effective identity for step execution
    ///
    /// Resolves the actual identity to use based on the execution_identity mode:
    /// - Agent: Returns the agent's service account ID
    /// - Caller: Returns the original trigger caller's ID
    /// - Function: Returns the function's elevated service account ID
    ///
    /// # Arguments
    /// * `identity_mode` - The execution identity mode
    /// * `agent_ref` - Optional reference to the agent (for agent mode)
    /// * `function_ref` - Optional reference to the function (for function mode)
    /// * `caller_id` - Optional original caller ID (for caller mode)
    ///
    /// # Returns
    /// The effective identity ID to use for permission checks
    async fn resolve_identity(
        &self,
        identity_mode: &str,
        _agent_ref: Option<&str>,
        _function_ref: Option<&str>,
        caller_id: Option<&str>,
    ) -> FlowResult<String> {
        // Default: return caller_id or a default service account
        Ok(caller_id
            .map(String::from)
            .unwrap_or_else(|| format!("flow-runtime-{}", identity_mode)))
    }
}

#[cfg(test)]
mod patch_tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// A node store of exactly one node, recording what the last write held.
    struct OneNode {
        properties: Mutex<Value>,
    }

    #[async_trait]
    impl FlowCallbacks for OneNode {
        async fn load_instance(&self, _path: &str) -> FlowResult<FlowInstance> {
            unimplemented!()
        }
        async fn save_instance(&self, _instance: &FlowInstance) -> FlowResult<()> {
            Ok(())
        }
        async fn save_instance_with_version(
            &self,
            _instance: &FlowInstance,
            _expected_version: i32,
        ) -> FlowResult<()> {
            Ok(())
        }
        async fn create_node(&self, _t: &str, _p: &str, props: Value) -> FlowResult<Value> {
            Ok(props)
        }
        async fn update_node(&self, _path: &str, properties: Value) -> FlowResult<Value> {
            *self.properties.lock().unwrap() = properties.clone();
            Ok(properties)
        }
        async fn get_node(&self, _path: &str) -> FlowResult<Option<Value>> {
            Ok(Some(serde_json::json!({
                "properties": self.properties.lock().unwrap().clone()
            })))
        }
        async fn queue_job(&self, _job_type: &str, _payload: Value) -> FlowResult<String> {
            Ok("job".to_string())
        }
        async fn call_ai(
            &self,
            _w: &str,
            _a: &str,
            _m: Vec<Value>,
            _f: Option<Value>,
        ) -> FlowResult<Value> {
            Ok(Value::Null)
        }
        async fn execute_function(&self, _f: &str, _i: Value) -> FlowResult<Value> {
            Ok(Value::Null)
        }
    }

    /// THE BUG THIS GUARDS: escalating a task wrote only the escalation
    /// fields, replacing the whole property set. A patch keeps the rest.
    #[tokio::test]
    async fn test_patch_keeps_the_properties_it_does_not_name() {
        let store = OneNode {
            properties: Mutex::new(serde_json::json!({
                "status": "pending",
                "flow_instance_id": "inst-1",
                "title": "Approve",
                "assignee": "/agents/bot",
            })),
        };

        store
            .patch_node_in_workspace(
                "raisin:access_control",
                "/agents/bot/inbox/task-1",
                serde_json::json!({
                    "assignee": "/users/admin",
                    "escalated_from": "/agents/bot",
                }),
            )
            .await
            .unwrap();

        let written = store.properties.lock().unwrap().clone();
        assert_eq!(written["status"], "pending", "status must survive");
        assert_eq!(
            written["flow_instance_id"], "inst-1",
            "the flow link must survive"
        );
        assert_eq!(written["title"], "Approve");
        assert_eq!(written["assignee"], "/users/admin", "patched field wins");
        assert_eq!(written["escalated_from"], "/agents/bot", "new field added");
    }
}
