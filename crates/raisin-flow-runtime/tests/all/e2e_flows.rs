// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! End-to-end flow engine tests.
//!
//! A deterministic in-memory harness implements `FlowCallbacks` and pumps
//! the job queue the way the production job system would: function jobs
//! execute scripted results and resume the flow, child-flow jobs create and
//! run child instances, timeout-check jobs run `check_flow_timeout`.
//!
//! Covered scenarios: template substitution, decision branching, loops,
//! retry/error-edge/rollback, human tasks (wait + resume + timeout),
//! agent steps, agent-as-assignee (decide + escalate), sub-flows, and
//! parallel fork/join.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::{json, Value};

use raisin_flow_runtime::runtime::{check_flow_timeout, execute_flow, resume_flow};
use raisin_flow_runtime::types::{
    FlowCallbacks, FlowError, FlowExecutionEvent, FlowInstance, FlowResult, FlowStatus, WaitType,
};

// ===========================================================================
// Harness
// ===========================================================================

#[derive(Default)]
struct Harness {
    /// instance id -> instance (version maintained like storage would)
    instances: Mutex<HashMap<String, FlowInstance>>,
    /// "{workspace}:{path}" -> node value
    nodes: Mutex<HashMap<String, Value>>,
    /// queued jobs: (job_type, payload)
    jobs: Mutex<VecDeque<(String, Value)>>,
    /// scripted results per function ref (queue per ref)
    function_results: Mutex<HashMap<String, VecDeque<Value>>>,
    /// recorded function-execution job inputs: (function_ref, arguments)
    function_invocations: Mutex<Vec<(String, Value)>>,
    /// functions executed synchronously (compensations): (ref, input)
    sync_executions: Mutex<Vec<(String, Value)>>,
    /// scripted AI responses (FIFO)
    ai_responses: Mutex<VecDeque<Value>>,
    /// recorded AI calls: (workspace, agent_ref, messages, response_format)
    ai_calls: Mutex<Vec<(String, String, Vec<Value>, Option<Value>)>>,
    /// emitted events
    events: Mutex<Vec<FlowExecutionEvent>>,
    /// counter for deterministic child instance ids
    child_counter: Mutex<usize>,
}

impl Harness {
    fn new() -> Self {
        Self::default()
    }

    fn node_key(workspace: &str, path: &str) -> String {
        format!("{}:{}", workspace, path)
    }

    fn seed_instance(&self, instance: FlowInstance) -> String {
        let id = instance.id.clone();
        self.instances.lock().unwrap().insert(id.clone(), instance);
        id
    }

    fn seed_node(&self, workspace: &str, path: &str, node: Value) {
        self.nodes
            .lock()
            .unwrap()
            .insert(Self::node_key(workspace, path), node);
    }

    fn get_stored_node(&self, workspace: &str, path: &str) -> Option<Value> {
        self.nodes
            .lock()
            .unwrap()
            .get(&Self::node_key(workspace, path))
            .cloned()
    }

    /// Find the single node whose key starts with the given prefix.
    fn find_node_by_prefix(&self, workspace: &str, path_prefix: &str) -> Option<(String, Value)> {
        let prefix = Self::node_key(workspace, path_prefix);
        self.nodes
            .lock()
            .unwrap()
            .iter()
            .find(|(k, _)| k.starts_with(&prefix))
            .map(|(k, v)| (k.splitn(2, ':').nth(1).unwrap_or(k).to_string(), v.clone()))
    }

    fn instance(&self, id: &str) -> FlowInstance {
        self.instances
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .unwrap_or_else(|| panic!("instance {} not found", id))
    }

    fn script_function(&self, function_ref: &str, result: Value) {
        self.function_results
            .lock()
            .unwrap()
            .entry(function_ref.to_string())
            .or_default()
            .push_back(result);
    }

    fn script_ai(&self, response: Value) {
        self.ai_responses.lock().unwrap().push_back(response);
    }

    fn pop_job(&self) -> Option<(String, Value)> {
        self.jobs.lock().unwrap().pop_front()
    }

    /// Drive queued jobs to completion, simulating the production job
    /// system. Individual job failures (e.g. a flow ending Failed) are
    /// swallowed like the production handler does.
    ///
    /// Timeout-check jobs scheduled for the FUTURE are fast-forwarded
    /// (virtual time) - deadlines always fire. Tests where waits are
    /// answered before their deadline use [`Self::pump_real_time`].
    async fn pump(&self) {
        self.pump_inner(true).await
    }

    /// Like [`Self::pump`], but DISCARDS timeout-check jobs whose
    /// scheduled time has not arrived (real-time semantics: the wait is
    /// completed before the deadline, so the check never fires).
    async fn pump_real_time(&self) {
        self.pump_inner(false).await
    }

    async fn pump_inner(&self, virtual_time: bool) {
        let mut iterations = 0;
        while let Some((job_type, payload)) = self.pop_job() {
            iterations += 1;
            assert!(iterations < 200, "job pump runaway (possible loop)");

            if !virtual_time
                && job_type == "FlowInstanceExecution"
                && payload["execution_type"].as_str() == Some("timeout_check")
            {
                let scheduled_future = payload
                    .get("__scheduled_at")
                    .and_then(|v| v.as_str())
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|t| t.with_timezone(&chrono::Utc) > chrono::Utc::now())
                    .unwrap_or(false);
                if scheduled_future {
                    continue; // deadline not reached - drop the check
                }
            }

            match job_type.as_str() {
                "function_execution" => {
                    let function_ref = payload["function_path"].as_str().unwrap().to_string();
                    let instance_id = payload["instance_id"].as_str().unwrap().to_string();
                    self.function_invocations
                        .lock()
                        .unwrap()
                        .push((function_ref.clone(), payload["arguments"].clone()));

                    let result = self
                        .function_results
                        .lock()
                        .unwrap()
                        .get_mut(&function_ref)
                        .and_then(|q| q.pop_front())
                        .unwrap_or_else(|| {
                            panic!("no scripted result for function {}", function_ref)
                        });

                    let _ = resume_flow(&instance_id, result, self).await;
                }
                "FlowInstanceExecution" => {
                    let instance_id = payload["instance_id"].as_str().unwrap().to_string();
                    match payload["execution_type"]
                        .as_str()
                        .unwrap_or("timeout_check")
                    {
                        "timeout_check" => {
                            // The production job system dispatches this job
                            // only once its scheduled time arrives - simulate
                            // the passage of time for future schedules.
                            let scheduled_future = payload
                                .get("__scheduled_at")
                                .and_then(|v| v.as_str())
                                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                                .map(|t| t.with_timezone(&chrono::Utc) > chrono::Utc::now())
                                .unwrap_or(false);
                            if scheduled_future {
                                let mut map = self.instances.lock().unwrap();
                                if let Some(inst) = map.get_mut(&instance_id) {
                                    if let Some(w) = inst.wait_info.as_mut() {
                                        w.timeout_at =
                                            Some(chrono::Utc::now() - chrono::Duration::seconds(1));
                                    }
                                }
                            }
                            let _ = check_flow_timeout(&instance_id, self).await;
                        }
                        "resume" => {
                            let _ = resume_flow(
                                &instance_id,
                                payload.get("resume_data").cloned().unwrap_or(Value::Null),
                                self,
                            )
                            .await;
                        }
                        "start" => {
                            let _ = execute_flow(&instance_id, self).await;
                        }
                        other => panic!("unexpected execution_type {}", other),
                    }
                }
                // Sub-flow: child referenced by path (functions workspace)
                "flow_execution" => {
                    let flow_path = payload["flow_path"].as_str().unwrap();
                    let parent_id = payload["parent_instance_id"].as_str().unwrap().to_string();
                    let parent_step = payload
                        .get("parent_step_id")
                        .and_then(|v| v.as_str())
                        .map(String::from);

                    let flow_node = self
                        .get_stored_node("functions", flow_path)
                        .unwrap_or_else(|| panic!("flow node {} not seeded", flow_path));
                    let workflow_data = flow_node["properties"]["workflow_data"].clone();

                    // Set when a parallel container fans out over a DEPLOYED
                    // flow, so the child echoes it back on completion and the
                    // join can correlate (mirrors job_callback.rs).
                    let branch_id = payload
                        .get("branch_id")
                        .and_then(|v| v.as_str())
                        .map(String::from);

                    let child_id = payload["__child_id"].as_str().unwrap().to_string();
                    self.spawn_child(
                        child_id,
                        workflow_data,
                        payload.get("input").cloned().unwrap_or(Value::Null),
                        parent_id,
                        parent_step,
                        branch_id,
                    )
                    .await;
                }
                // Parallel branch: inline definition
                "create_child_flow" => {
                    let parent_id = payload["parent_instance_id"].as_str().unwrap().to_string();
                    let branch_id = payload
                        .get("branch_id")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let child_id = payload["__child_id"].as_str().unwrap().to_string();
                    self.spawn_child(
                        child_id,
                        payload["flow_definition"].clone(),
                        payload.get("input").cloned().unwrap_or(Value::Null),
                        parent_id,
                        None,
                        branch_id,
                    )
                    .await;
                }
                other => panic!("unexpected job type {}", other),
            }
        }
    }

    async fn spawn_child(
        &self,
        child_id: String,
        workflow_data: Value,
        input: Value,
        parent_id: String,
        parent_step_id: Option<String>,
        branch_id: Option<String>,
    ) {
        let mut child = make_instance(workflow_data, input);
        child.id = child_id;
        child.parent_instance_ref = Some(parent_id);
        if let Value::Object(ref mut vars) = child.variables {
            if let Some(step_id) = &parent_step_id {
                vars.insert("__parent_step_id".to_string(), json!(step_id));
            }
            if let Some(bid) = &branch_id {
                vars.insert("__branch_id".to_string(), json!(bid));
            }
        }
        let child_id = self.seed_instance(child);
        let _ = execute_flow(&child_id, self).await;
    }
}

#[async_trait]
impl FlowCallbacks for Harness {
    async fn load_instance(&self, path: &str) -> FlowResult<FlowInstance> {
        let id = path.rsplit('/').next().unwrap_or(path);
        self.instances
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| FlowError::NodeNotFound(format!("instance {} not found", id)))
    }

    async fn save_instance(&self, instance: &FlowInstance) -> FlowResult<()> {
        let mut map = self.instances.lock().unwrap();
        let mut updated = instance.clone();
        updated.version = map
            .get(&instance.id)
            .map(|cur| cur.version + 1)
            .unwrap_or(instance.version + 1);
        map.insert(instance.id.clone(), updated);
        Ok(())
    }

    async fn save_instance_with_version(
        &self,
        instance: &FlowInstance,
        expected_version: i32,
    ) -> FlowResult<()> {
        let mut map = self.instances.lock().unwrap();
        if let Some(current) = map.get(&instance.id) {
            if current.version != expected_version {
                return Err(FlowError::VersionConflict);
            }
        }
        let mut updated = instance.clone();
        updated.version = expected_version + 1;
        map.insert(instance.id.clone(), updated);
        Ok(())
    }

    async fn create_node(
        &self,
        node_type: &str,
        path: &str,
        properties: Value,
    ) -> FlowResult<Value> {
        self.create_node_in_workspace("raisin:system", node_type, path, properties)
            .await
    }

    async fn update_node(&self, path: &str, properties: Value) -> FlowResult<Value> {
        self.update_node_in_workspace("raisin:system", path, properties)
            .await
    }

    async fn get_node(&self, path: &str) -> FlowResult<Option<Value>> {
        self.get_node_in_workspace("raisin:system", path).await
    }

    async fn create_node_in_workspace(
        &self,
        workspace: &str,
        node_type: &str,
        path: &str,
        properties: Value,
    ) -> FlowResult<Value> {
        let node = json!({
            "node_type": node_type,
            "path": path,
            "properties": properties,
        });
        self.seed_node(workspace, path, node.clone());
        Ok(node)
    }

    async fn get_node_in_workspace(
        &self,
        workspace: &str,
        path: &str,
    ) -> FlowResult<Option<Value>> {
        Ok(self.get_stored_node(workspace, path))
    }

    async fn update_node_in_workspace(
        &self,
        workspace: &str,
        path: &str,
        properties: Value,
    ) -> FlowResult<Value> {
        let mut nodes = self.nodes.lock().unwrap();
        let key = Self::node_key(workspace, path);
        let node = nodes
            .get_mut(&key)
            .ok_or_else(|| FlowError::NodeNotFound(format!("node {} not found", key)))?;
        if let (Some(existing), Some(updates)) =
            (node["properties"].as_object_mut(), properties.as_object())
        {
            for (k, v) in updates {
                existing.insert(k.clone(), v.clone());
            }
        }
        Ok(node.clone())
    }

    async fn queue_job(&self, job_type: &str, payload: Value) -> FlowResult<String> {
        let mut payload = payload;
        // Child-flow jobs return the (pre-assigned) child INSTANCE id,
        // mirroring the production job queuer contract
        let ret = if job_type == "flow_execution" || job_type == "create_child_flow" {
            let mut counter = self.child_counter.lock().unwrap();
            *counter += 1;
            let child_id = format!("child-{}", *counter);
            if let Value::Object(ref mut map) = payload {
                map.insert("__child_id".to_string(), json!(child_id));
            }
            child_id
        } else {
            format!("job-{}", self.jobs.lock().unwrap().len() + 1)
        };
        self.jobs
            .lock()
            .unwrap()
            .push_back((job_type.to_string(), payload));
        Ok(ret)
    }

    async fn call_ai(
        &self,
        agent_workspace: &str,
        agent_ref: &str,
        messages: Vec<Value>,
        response_format: Option<Value>,
    ) -> FlowResult<Value> {
        self.ai_calls.lock().unwrap().push((
            agent_workspace.to_string(),
            agent_ref.to_string(),
            messages,
            response_format,
        ));
        self.ai_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| FlowError::AIProvider("no scripted AI response".to_string()))
    }

    async fn execute_function(&self, function_ref: &str, input: Value) -> FlowResult<Value> {
        self.sync_executions
            .lock()
            .unwrap()
            .push((function_ref.to_string(), input));
        Ok(json!({"compensated": true}))
    }

    async fn emit_event(&self, _instance_id: &str, event: FlowExecutionEvent) -> FlowResult<()> {
        self.events.lock().unwrap().push(event);
        Ok(())
    }
}

// ===========================================================================
// Helpers
// ===========================================================================

fn make_instance(workflow_data: Value, input: Value) -> FlowInstance {
    FlowInstance {
        id: "flow-1".to_string(),
        version: 1,
        flow_ref: "/flows/test".to_string(),
        flow_version: 1,
        flow_definition_snapshot: workflow_data,
        status: FlowStatus::Pending,
        current_node_id: "start".to_string(),
        wait_info: None,
        variables: json!({}),
        input,
        output: None,
        compensation_stack: Vec::new(),
        error: None,
        retry_count: 0,
        checkpoints: Vec::new(),
        started_at: chrono::Utc::now(),
        completed_at: None,
        parent_instance_ref: None,
        metrics: Default::default(),
        test_config: None,
    }
}

/// Start a flow and pump all resulting jobs.
async fn run_to_quiescence(harness: &Harness, workflow_data: Value, input: Value) -> String {
    let id = harness.seed_instance(make_instance(workflow_data, input));
    if let Err(e) = execute_flow(&id, harness).await {
        eprintln!(
            "execute_flow error (often expected for failure tests): {}",
            e
        );
    }
    harness.pump().await;
    id
}

// ===========================================================================
// 1. Linear flow with template substitution
// ===========================================================================

#[tokio::test]
async fn linear_flow_resolves_templates_across_steps() {
    let harness = Harness::new();
    harness.script_function(
        "/lib/greet",
        json!({"success": true, "result": {"greeting": "Hello Alice"}}),
    );
    harness.script_function(
        "/lib/save",
        json!({"success": true, "result": {"saved": true}}),
    );

    let flow = json!({
        "nodes": [
            { "id": "start", "step_type": "start", "next_node": "greet" },
            {
                "id": "greet",
                "step_type": "function_step",
                "properties": {
                    "function_ref": "/lib/greet",
                    "arguments": {
                        "message": "Hello {{ input.user.name }}",
                        "age_next_year": "${input.user.age + 1}"
                    }
                },
                "next_node": "save"
            },
            {
                "id": "save",
                "step_type": "function_step",
                "properties": {
                    "function_ref": "/lib/save",
                    "arguments": { "text": "${steps.greet.greeting}" }
                },
                "next_node": "end"
            },
            { "id": "end", "step_type": "end" }
        ]
    });

    let id = run_to_quiescence(
        &harness,
        flow,
        json!({"user": {"name": "Alice", "age": 30}}),
    )
    .await;

    let instance = harness.instance(&id);
    assert_eq!(instance.status, FlowStatus::Completed);

    let invocations = harness.function_invocations.lock().unwrap();
    assert_eq!(invocations.len(), 2);
    // First step: templates resolved from input (native types preserved)
    assert_eq!(invocations[0].1["message"], json!("Hello Alice"));
    assert_eq!(invocations[0].1["age_next_year"], json!(31));
    // Second step: references the first step's output via steps.*
    assert_eq!(invocations[1].1["text"], json!("Hello Alice"));
}

// ===========================================================================
// 2. Decision branching (incl. steps.* in conditions)
// ===========================================================================

#[tokio::test]
async fn decision_routes_on_step_output() {
    let harness = Harness::new();
    harness.script_function(
        "/lib/score",
        json!({"success": true, "result": {"score": 8}}),
    );
    harness.script_function(
        "/lib/high",
        json!({"success": true, "result": {"path": "high"}}),
    );

    let flow = json!({
        "nodes": [
            { "id": "start", "step_type": "start", "next_node": "score" },
            {
                "id": "score",
                "step_type": "function_step",
                "properties": { "function_ref": "/lib/score" },
                "next_node": "check"
            },
            {
                "id": "check",
                "step_type": "decision",
                "properties": {
                    "condition": "steps.score.score > 5",
                    "yes_branch": "high",
                    "no_branch": "end"
                }
            },
            {
                "id": "high",
                "step_type": "function_step",
                "properties": { "function_ref": "/lib/high" },
                "next_node": "end"
            },
            { "id": "end", "step_type": "end" }
        ]
    });

    let id = run_to_quiescence(&harness, flow, json!({})).await;
    let instance = harness.instance(&id);
    assert_eq!(instance.status, FlowStatus::Completed);

    let invocations = harness.function_invocations.lock().unwrap();
    let called: Vec<&str> = invocations.iter().map(|(f, _)| f.as_str()).collect();
    assert_eq!(called, vec!["/lib/score", "/lib/high"]);
}

#[tokio::test]
async fn decision_routes_no_branch() {
    let harness = Harness::new();

    let flow = json!({
        "nodes": [
            { "id": "start", "step_type": "start", "next_node": "check" },
            {
                "id": "check",
                "step_type": "decision",
                "properties": {
                    "condition": "input.value > 10",
                    "yes_branch": "end",
                    "no_branch": "low"
                }
            },
            { "id": "low", "step_type": "start", "next_node": "end" },
            { "id": "end", "step_type": "end" }
        ]
    });

    let id = run_to_quiescence(&harness, flow, json!({"value": 3})).await;
    let instance = harness.instance(&id);
    assert_eq!(instance.status, FlowStatus::Completed);
    // The no-branch path records check_result = false
    assert_eq!(instance.variables["check_result"], json!(false));
}

// ===========================================================================
// 3. Loop over a collection
// ===========================================================================

#[tokio::test]
async fn for_each_loop_iterates_collection() {
    let harness = Harness::new();
    for i in 0..3 {
        harness.script_function(
            "/lib/process",
            json!({"success": true, "result": {"processed": i}}),
        );
    }

    let flow = json!({
        "nodes": [
            { "id": "start", "step_type": "start", "next_node": "each" },
            {
                "id": "each",
                "step_type": "loop",
                "properties": {
                    "loop_type": "for_each",
                    "collection": "${input.items}",
                    "item_var": "current",
                    "body_step": "body"
                },
                "next_node": "end"
            },
            {
                "id": "body",
                "step_type": "function_step",
                "properties": {
                    "function_ref": "/lib/process",
                    "arguments": { "item": "${current}" }
                },
                "next_node": "each"
            },
            { "id": "end", "step_type": "end" }
        ]
    });

    let id = run_to_quiescence(&harness, flow, json!({"items": ["a", "b", "c"]})).await;
    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "error: {:?}",
        instance.error
    );

    let invocations = harness.function_invocations.lock().unwrap();
    assert_eq!(invocations.len(), 3);
    let items: Vec<&Value> = invocations.iter().map(|(_, args)| &args["item"]).collect();
    assert_eq!(items, vec![&json!("a"), &json!("b"), &json!("c")]);
}

// ===========================================================================
// 3b. Designer-format loop container (container_type: loop)
// ===========================================================================

/// A designer-format loop container iterates its children once per item:
/// the lowering produces a Loop node, chains the body back to it, and the
/// aggregated results are visible to the step after the loop.
#[tokio::test]
async fn designer_loop_container_iterates_collection() {
    let harness = Harness::new();
    for i in 0..3 {
        harness.script_function(
            "/lib/process",
            json!({"success": true, "result": {"processed": i}}),
        );
    }
    harness.script_function(
        "/lib/after",
        json!({"success": true, "result": {"after": true}}),
    );

    let flow = json!({
        "version": 1,
        "error_strategy": "fail_fast",
        "nodes": [
            {
                "id": "each",
                "node_type": "raisin:FlowContainer",
                "container_type": "loop",
                "loop": {
                    "over": "${input.items}",
                    "item": "current",
                    "index": "current_index"
                },
                "children": [
                    {
                        "id": "body",
                        "node_type": "raisin:FlowStep",
                        "properties": {
                            "function_ref": "/lib/process",
                            "arguments": {
                                "item": "${current}",
                                "index": "${current_index}"
                            }
                        }
                    }
                ]
            },
            {
                "id": "after",
                "node_type": "raisin:FlowStep",
                "properties": {
                    "function_ref": "/lib/after",
                    "arguments": { "count": "${steps.each.count}" }
                }
            }
        ]
    });

    let id = run_to_quiescence(&harness, flow, json!({"items": ["a", "b", "c"]})).await;
    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "error: {:?}",
        instance.error
    );

    let invocations = harness.function_invocations.lock().unwrap();
    // 3 body iterations + the step after the loop
    assert_eq!(invocations.len(), 4);
    let items: Vec<&Value> = invocations
        .iter()
        .take(3)
        .map(|(_, args)| &args["item"])
        .collect();
    assert_eq!(items, vec![&json!("a"), &json!("b"), &json!("c")]);
    let indexes: Vec<&Value> = invocations
        .iter()
        .take(3)
        .map(|(_, args)| &args["index"])
        .collect();
    assert_eq!(indexes, vec![&json!(0), &json!(1), &json!(2)]);
    // Aggregated loop output is visible to the next step
    assert_eq!(invocations[3].1["count"], json!(3));
}

/// The `until` REL condition exits the loop after the iteration that
/// satisfies it - remaining items are never processed.
#[tokio::test]
async fn designer_loop_container_until_exits_early() {
    let harness = Harness::new();
    harness.script_function(
        "/lib/ask",
        json!({"success": true, "result": {"response": "reject"}}),
    );
    harness.script_function(
        "/lib/ask",
        json!({"success": true, "result": {"response": "accept"}}),
    );
    // No scripted result for a third call: the until condition must stop
    // the loop before the third candidate is asked.

    let flow = json!({
        "version": 1,
        "error_strategy": "fail_fast",
        "nodes": [
            {
                "id": "ask_each",
                "node_type": "raisin:FlowContainer",
                "container_type": "loop",
                "loop": {
                    "over": "${input.candidates}",
                    "item": "candidate",
                    "max_iterations": 10,
                    "until": "steps.ask.response == 'accept'"
                },
                "children": [
                    {
                        "id": "ask",
                        "node_type": "raisin:FlowStep",
                        "properties": {
                            "function_ref": "/lib/ask",
                            "arguments": { "who": "${candidate}" }
                        }
                    }
                ]
            }
        ]
    });

    let id = run_to_quiescence(&harness, flow, json!({"candidates": ["ana", "bo", "cy"]})).await;
    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "error: {:?}",
        instance.error
    );

    let invocations = harness.function_invocations.lock().unwrap();
    assert_eq!(invocations.len(), 2, "third candidate must not be asked");
    assert_eq!(invocations[0].1["who"], json!("ana"));
    assert_eq!(invocations[1].1["who"], json!("bo"));
    // Results collected up to the early exit are aggregated
    assert_eq!(
        instance.variables["ask_each_results"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

/// `max_iterations` caps how many items of the collection are processed.
#[tokio::test]
async fn designer_loop_container_max_iterations_caps_items() {
    let harness = Harness::new();
    for _ in 0..2 {
        harness.script_function("/lib/process", json!({"success": true, "result": {}}));
    }

    let flow = json!({
        "version": 1,
        "error_strategy": "fail_fast",
        "nodes": [
            {
                "id": "each",
                "node_type": "raisin:FlowContainer",
                "container_type": "loop",
                "loop": { "over": "${input.items}", "max_iterations": 2 },
                "children": [
                    {
                        "id": "body",
                        "node_type": "raisin:FlowStep",
                        "properties": {
                            "function_ref": "/lib/process",
                            "arguments": { "item": "${item}" }
                        }
                    }
                ]
            }
        ]
    });

    let id = run_to_quiescence(&harness, flow, json!({"items": [1, 2, 3, 4]})).await;
    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "error: {:?}",
        instance.error
    );
    assert_eq!(harness.function_invocations.lock().unwrap().len(), 2);
}

// ===========================================================================
// 4. Error handling: retry, error edge, rollback
// ===========================================================================

#[tokio::test]
async fn failed_function_retries_then_takes_error_edge() {
    let harness = Harness::new();
    // Two failures: initial + 1 retry
    harness.script_function("/lib/flaky", json!({"success": false, "error": "boom 1"}));
    harness.script_function("/lib/flaky", json!({"success": false, "error": "boom 2"}));

    let flow = json!({
        "nodes": [
            { "id": "start", "step_type": "start", "next_node": "flaky" },
            {
                "id": "flaky",
                "step_type": "function_step",
                "properties": {
                    "function_ref": "/lib/flaky",
                    "max_retries": 1,
                    "error_edge": "cleanup"
                },
                "next_node": "end"
            },
            { "id": "cleanup", "step_type": "start", "next_node": "end" },
            { "id": "end", "step_type": "end" }
        ]
    });

    let id = run_to_quiescence(&harness, flow, json!({})).await;
    let instance = harness.instance(&id);
    eprintln!(
        "RETRY DEBUG status={:?} error={:?} node={} retry_count={} wait={:?} vars={}",
        instance.status,
        instance.error,
        instance.current_node_id,
        instance.retry_count,
        instance.wait_info,
        instance.variables
    );

    // Flow recovered via the error edge
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "error: {:?}",
        instance.error
    );
    // Both attempts hit the function
    assert_eq!(harness.function_invocations.lock().unwrap().len(), 2);
    // Error context was populated for the handler path
    assert!(instance.variables.get("error").is_some());
}

#[tokio::test]
async fn failed_function_rolls_back_compensations_lifo() {
    let harness = Harness::new();
    harness.script_function(
        "/lib/book-flight",
        json!({"success": true, "result": {"booking_id": "F1"}}),
    );
    harness.script_function(
        "/lib/book-hotel",
        json!({"success": true, "result": {"booking_id": "H1"}}),
    );
    harness.script_function(
        "/lib/charge",
        json!({"success": false, "error": "card declined"}),
    );

    let flow = json!({
        "nodes": [
            { "id": "start", "step_type": "start", "next_node": "flight" },
            {
                "id": "flight",
                "step_type": "function_step",
                "properties": {
                    "function_ref": "/lib/book-flight",
                    "compensation_ref": "/lib/cancel-flight",
                    "compensation_input_mapping": { "booking_id": "${output.booking_id}" }
                },
                "next_node": "hotel"
            },
            {
                "id": "hotel",
                "step_type": "function_step",
                "properties": {
                    "function_ref": "/lib/book-hotel",
                    "compensation_ref": "/lib/cancel-hotel",
                    "compensation_input_mapping": { "booking_id": "${output.booking_id}" }
                },
                "next_node": "charge"
            },
            {
                "id": "charge",
                "step_type": "function_step",
                "properties": { "function_ref": "/lib/charge", "max_retries": 0 },
                "next_node": "end"
            },
            { "id": "end", "step_type": "end" }
        ]
    });

    let id = run_to_quiescence(&harness, flow, json!({})).await;
    let instance = harness.instance(&id);
    eprintln!(
        "ROLLBACK DEBUG status={:?} error={:?} stack={:?}",
        instance.status, instance.error, instance.compensation_stack
    );

    assert_eq!(instance.status, FlowStatus::RolledBack);

    // Compensations executed in reverse (LIFO) order with mapped inputs
    let compensations = harness.sync_executions.lock().unwrap();
    assert_eq!(compensations.len(), 2);
    assert_eq!(compensations[0].0, "/lib/cancel-hotel");
    assert_eq!(compensations[0].1, json!({"booking_id": "H1"}));
    assert_eq!(compensations[1].0, "/lib/cancel-flight");
    assert_eq!(compensations[1].1, json!({"booking_id": "F1"}));

    // Entries retained for audit with executed status
    assert_eq!(instance.compensation_stack.len(), 2);
    for entry in &instance.compensation_stack {
        assert!(matches!(
            entry.compensation_status,
            raisin_flow_runtime::types::CompensationStatus::Executed { .. }
        ));
    }
}

// ===========================================================================
// 5. Human task: wait, complete, resume
// ===========================================================================

fn approval_flow() -> Value {
    json!({
        "nodes": [
            { "id": "start", "step_type": "start", "next_node": "approve" },
            {
                "id": "approve",
                "step_type": "human_task",
                "properties": {
                    "task_type": "approval",
                    "title": "Approve order {{ input.order_id }}",
                    "assignee": "/users/manager",
                    "options": [
                        {"value": "approve", "label": "Approve", "style": "success"},
                        {"value": "reject", "label": "Reject", "style": "danger"}
                    ]
                },
                "next_node": "check"
            },
            {
                "id": "check",
                "step_type": "decision",
                "properties": {
                    "condition": "__human_response.action == \"approve\"",
                    "yes_branch": "end",
                    "no_branch": "rejected"
                }
            },
            { "id": "rejected", "step_type": "start", "next_node": "end" },
            { "id": "end", "step_type": "end" }
        ]
    })
}

#[tokio::test]
async fn human_task_waits_and_resumes_with_response() {
    let harness = Harness::new();
    let id = run_to_quiescence(&harness, approval_flow(), json!({"order_id": "ORD-7"})).await;

    // Flow is waiting on the human task
    let instance = harness.instance(&id);
    assert_eq!(instance.status, FlowStatus::Waiting);
    let wait_info = instance.wait_info.as_ref().unwrap();
    assert_eq!(wait_info.wait_type, WaitType::HumanTask);
    assert!(wait_info.target_path.is_some(), "task path must be tracked");

    // The inbox task node was created in the canonical workspace with
    // resolved templates
    let (task_path, task) = harness
        .find_node_by_prefix("raisin:access_control", "/users/manager/inbox/")
        .expect("inbox task node created");
    assert_eq!(task["node_type"], json!("raisin:InboxTask"));
    assert_eq!(task["properties"]["title"], json!("Approve order ORD-7"));
    assert_eq!(task["properties"]["status"], json!("pending"));
    assert_eq!(task["properties"]["flow_instance_id"], json!(id));
    assert_eq!(task["properties"]["step_id"], json!("approve"));

    // Complete the task (as the inbox API would after validation)
    let _ = resume_flow(
        &id,
        json!({"action": "approve", "completed_by": "manager", "task_path": task_path}),
        &harness,
    )
    .await;
    harness.pump().await;

    let instance = harness.instance(&id);
    assert_eq!(instance.status, FlowStatus::Completed);
    // The approve branch was taken
    assert_eq!(instance.variables["check_result"], json!(true));
}

#[tokio::test]
async fn human_task_timeout_follows_timeout_edge_and_expires_task() {
    let harness = Harness::new();

    let flow = json!({
        "nodes": [
            { "id": "start", "step_type": "start", "next_node": "approve" },
            {
                "id": "approve",
                "step_type": "human_task",
                "properties": {
                    "task_type": "approval",
                    "title": "Approve",
                    "assignee": "/users/manager",
                    "due_in_seconds": 3600,
                    "timeout_edge": "escalate"
                },
                "next_node": "end"
            },
            { "id": "escalate", "step_type": "start", "next_node": "end" },
            { "id": "end", "step_type": "end" }
        ]
    });

    // Start without pumping so the scheduled deadline check stays queued
    let id = harness.seed_instance(make_instance(flow, json!({})));
    let _ = execute_flow(&id, &harness).await;

    // Waiting with a deadline derived from due_in_seconds
    let instance = harness.instance(&id);
    assert_eq!(instance.status, FlowStatus::Waiting);
    assert!(instance.wait_info.as_ref().unwrap().timeout_at.is_some());

    // Not yet due: a timeout check is a no-op
    let _ = check_flow_timeout(&id, &harness).await;
    assert_eq!(harness.instance(&id).status, FlowStatus::Waiting);

    // Pumping delivers the scheduled deadline check at its due time
    // (virtual time) -> timeout_edge path
    harness.pump().await;

    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "timeout edge should complete the flow"
    );

    // The inbox task was marked expired
    let (_, task) = harness
        .find_node_by_prefix("raisin:access_control", "/users/manager/inbox/")
        .unwrap();
    assert_eq!(task["properties"]["status"], json!("expired"));
}

/// Two runs of the same flow definition must each get their OWN inbox task:
/// run 2's task path differs from run 1's, and run 2 must not auto-complete
/// from run 1's (already completed) task node.
#[tokio::test]
async fn second_run_creates_fresh_task_and_ignores_previous_runs_completed_task() {
    let harness = Harness::new();

    // --- Run 1: wait, complete, flow finishes -----------------------------
    let id1 = run_to_quiescence(&harness, approval_flow(), json!({"order_id": "ORD-1"})).await;
    let instance1 = harness.instance(&id1);
    assert_eq!(instance1.status, FlowStatus::Waiting);
    let task_path1 = instance1
        .wait_info
        .as_ref()
        .unwrap()
        .target_path
        .clone()
        .expect("run 1 task path tracked");

    // Instance-scoped path: contains the instance slug and iteration marker
    assert!(
        task_path1.contains(&raisin_flow_runtime::instance_slug("flow-1"))
            && task_path1.ends_with("-it0"),
        "task path must be instance + iteration scoped, got: {}",
        task_path1
    );

    let _ = resume_flow(
        &id1,
        json!({"action": "approve", "completed_by": "manager", "task_path": task_path1}),
        &harness,
    )
    .await;
    harness.pump().await;
    assert_eq!(harness.instance(&id1).status, FlowStatus::Completed);

    // Run 1's task node is now completed (as the inbox API would leave it)
    harness
        .update_node_in_workspace(
            "raisin:access_control",
            &task_path1,
            json!({"status": "completed", "response": {"action": "approve"}}),
        )
        .await
        .unwrap();

    // --- Run 2: same flow definition, fresh instance ----------------------
    let mut instance2 = make_instance(approval_flow(), json!({"order_id": "ORD-2"}));
    instance2.id = "flow-2".to_string();
    let id2 = harness.seed_instance(instance2);
    let _ = execute_flow(&id2, &harness).await;
    harness.pump_real_time().await;

    // Run 2 must WAIT on its own task - never auto-complete from run 1's
    // completed task node
    let instance2 = harness.instance(&id2);
    assert_eq!(
        instance2.status,
        FlowStatus::Waiting,
        "run 2 must wait for its own task, not complete from run 1's response"
    );
    let task_path2 = instance2
        .wait_info
        .as_ref()
        .unwrap()
        .target_path
        .clone()
        .expect("run 2 task path tracked");
    assert_ne!(
        task_path2, task_path1,
        "run 2 must get its own task path, not reuse run 1's"
    );

    // Run 2's task is a fresh PENDING task; run 1's stays completed
    let task2 = harness
        .get_stored_node("raisin:access_control", &task_path2)
        .expect("run 2 task node created");
    assert_eq!(task2["properties"]["status"], json!("pending"));
    assert_eq!(task2["properties"]["flow_instance_id"], json!(id2));
    let task1 = harness
        .get_stored_node("raisin:access_control", &task_path1)
        .unwrap();
    assert_eq!(task1["properties"]["status"], json!("completed"));

    // Completing run 2's own task finishes run 2
    let _ = resume_flow(
        &id2,
        json!({"action": "approve", "completed_by": "manager", "task_path": task_path2}),
        &harness,
    )
    .await;
    harness.pump().await;
    assert_eq!(harness.instance(&id2).status, FlowStatus::Completed);
}

/// A stale COMPLETED task occupying the deterministic task path must never
/// satisfy a new wait: the step parks the new task at a fresh unique path
/// and the flow waits.
#[tokio::test]
async fn stale_completed_task_at_path_never_satisfies_new_wait() {
    let harness = Harness::new();

    // The deterministic path the step would use for instance "flow-1",
    // step "approve", iteration 0
    // Derived, not hardcoded: the slug is a hash of the whole instance id, so
    // the expectation follows the implementation instead of restating it.
    let base_path = format!(
        "/users/manager/inbox/task-approve-{}-it0",
        raisin_flow_runtime::instance_slug("flow-1")
    );
    let base_path = base_path.as_str();

    // Seed a stale COMPLETED task at that path (e.g. left over from an
    // earlier visit of the same step)
    harness.seed_node(
        "raisin:access_control",
        base_path,
        json!({
            "node_type": "raisin:InboxTask",
            "path": base_path,
            "properties": {
                "status": "completed",
                "flow_instance_id": "flow-1",
                "step_id": "approve",
                "response": {"action": "approve"},
            }
        }),
    );

    let id = run_to_quiescence(&harness, approval_flow(), json!({"order_id": "ORD-9"})).await;

    // The flow must WAIT on a NEW task, not resume from the stale response
    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Waiting,
        "a stale completed task must not satisfy the new wait"
    );
    let new_task_path = instance
        .wait_info
        .as_ref()
        .unwrap()
        .target_path
        .clone()
        .unwrap();
    assert_ne!(new_task_path, base_path, "new task needs a fresh path");
    assert!(
        new_task_path.starts_with(base_path),
        "fresh path keeps the scoped prefix, got: {}",
        new_task_path
    );

    let new_task = harness
        .get_stored_node("raisin:access_control", &new_task_path)
        .expect("fresh task created");
    assert_eq!(new_task["properties"]["status"], json!("pending"));

    // The stale node is untouched
    let stale = harness
        .get_stored_node("raisin:access_control", base_path)
        .unwrap();
    assert_eq!(stale["properties"]["status"], json!("completed"));
}

/// UPGRADE PATH: a task this instance created under the PREVIOUS (prefix-based)
/// slug scheme is adopted, not duplicated.
///
/// Without this, an instance that parked before the upgrade and is redelivered
/// after it regenerates a different path, finds nothing there, and opens a
/// SECOND task — leaving the first pending in someone's inbox forever while the
/// flow waits on the other. This is the deploy-window case, and it is the only
/// reason the old scheme is still computed at all.
#[tokio::test]
async fn pending_task_from_the_previous_path_scheme_is_adopted() {
    let harness = Harness::new();

    // What the old scheme (first 8 alphanumerics of "flow-1") would have built.
    let legacy_path = "/users/manager/inbox/task-approve-flow1-it0";
    let current_path = format!(
        "/users/manager/inbox/task-approve-{}-it0",
        raisin_flow_runtime::instance_slug("flow-1")
    );
    assert_ne!(
        legacy_path, current_path,
        "the schemes must actually differ"
    );

    harness.seed_node(
        "raisin:access_control",
        legacy_path,
        json!({
            "node_type": "raisin:InboxTask",
            "properties": {
                "flow_instance_id": "flow-1",
                "step_id": "approve",
                "status": "pending",
                "title": "Approve order"
            }
        }),
    );

    let id = run_to_quiescence(&harness, approval_flow(), json!({"order_id": "ORD-9"})).await;
    let instance = harness.instance(&id);

    assert_eq!(instance.status, FlowStatus::Waiting);
    assert_eq!(
        instance.wait_info.as_ref().unwrap().target_path.as_deref(),
        Some(legacy_path),
        "the flow must wait on the task the human can already see"
    );
    assert!(
        harness
            .get_stored_node("raisin:access_control", &current_path)
            .is_none(),
        "adopting means NOT opening a second task at the new path"
    );
}

/// A still-PENDING task for the same instance/step at the deterministic
/// path is reused (idempotent re-delivery) instead of duplicated.
#[tokio::test]
async fn pending_task_for_same_wait_is_reused_not_duplicated() {
    let harness = Harness::new();

    // Derived, not hardcoded: the slug is a hash of the whole instance id, so
    // the expectation follows the implementation instead of restating it.
    let base_path = format!(
        "/users/manager/inbox/task-approve-{}-it0",
        raisin_flow_runtime::instance_slug("flow-1")
    );
    let base_path = base_path.as_str();
    harness.seed_node(
        "raisin:access_control",
        base_path,
        json!({
            "node_type": "raisin:InboxTask",
            "path": base_path,
            "properties": {
                "status": "pending",
                "flow_instance_id": "flow-1",
                "step_id": "approve",
                "title": "Approve order ORD-7",
            }
        }),
    );

    let id = run_to_quiescence(&harness, approval_flow(), json!({"order_id": "ORD-7"})).await;

    let instance = harness.instance(&id);
    assert_eq!(instance.status, FlowStatus::Waiting);
    assert_eq!(
        instance.wait_info.as_ref().unwrap().target_path.as_deref(),
        Some(base_path),
        "the existing pending task for this wait is reused"
    );

    // No second task node was created under the inbox
    let inbox_tasks = harness
        .nodes
        .lock()
        .unwrap()
        .keys()
        .filter(|k| k.starts_with("raisin:access_control:/users/manager/inbox/"))
        .count();
    assert_eq!(inbox_tasks, 1, "no duplicate task node");
}

// ===========================================================================
// 6. Agent steps
// ===========================================================================

#[tokio::test]
async fn agent_step_resolves_prompt_template() {
    let harness = Harness::new();
    harness.script_ai(json!({"content": "Positive", "finish_reason": "stop"}));

    let flow = json!({
        "nodes": [
            { "id": "start", "step_type": "start", "next_node": "classify" },
            {
                "id": "classify",
                "step_type": "agent_step",
                "properties": {
                    "agent_ref": "/agents/classifier",
                    "prompt": "Classify the sentiment of: {{ input.text }}",
                    "next_node": "end"
                }
            },
            { "id": "end", "step_type": "end" }
        ]
    });

    let id = run_to_quiescence(&harness, flow, json!({"text": "great product"})).await;
    let instance = harness.instance(&id);
    assert_eq!(instance.status, FlowStatus::Completed);

    let calls = harness.ai_calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, "/agents/classifier");
    let content = calls[0].2[0]["content"].as_str().unwrap();
    assert_eq!(content, "Classify the sentiment of: great product");

    // The agent's response is recorded under step_outputs
    assert_eq!(
        instance.variables["step_outputs"]["classify"]["response"],
        json!("Positive")
    );
}

// ===========================================================================
// 7. Agent as assignee (agent-in-the-loop)
// ===========================================================================

fn agent_assignee_flow(extra_props: Value) -> Value {
    let mut props = json!({
        "task_type": "approval",
        "title": "Approve refund of {{ input.amount }}",
        "assignee": "/agents/refund-approver",
        "options": [
            {"value": "approve", "label": "Approve"},
            {"value": "reject", "label": "Reject"}
        ]
    });
    if let (Some(base), Some(extra)) = (props.as_object_mut(), extra_props.as_object()) {
        for (k, v) in extra {
            base.insert(k.clone(), v.clone());
        }
    }

    json!({
        "nodes": [
            { "id": "start", "step_type": "start", "next_node": "approve" },
            {
                "id": "approve",
                "step_type": "human_task",
                "properties": props,
                "next_node": "end"
            },
            { "id": "end", "step_type": "end" }
        ]
    })
}

fn seed_agent(harness: &Harness, path: &str) {
    harness.seed_node(
        "functions",
        path,
        json!({
            "node_type": "raisin:AIAgent",
            "path": path,
            "properties": { "name": "refund approver", "model": "test" }
        }),
    );
}

#[tokio::test]
async fn agent_assignee_decides_task_and_flow_continues() {
    let harness = Harness::new();
    seed_agent(&harness, "/agents/refund-approver");
    harness.script_ai(json!({
        "content": "{\"decision\": \"approve\", \"reasoning\": \"within policy\", \"confidence\": 0.95}",
        "finish_reason": "stop"
    }));

    let id = run_to_quiescence(
        &harness,
        agent_assignee_flow(json!({})),
        json!({"amount": 42}),
    )
    .await;

    // Flow completed without waiting - the agent decided
    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "error: {:?}",
        instance.error
    );

    // The agent was consulted with the task content
    let calls = harness.ai_calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].2[0]["content"]
        .as_str()
        .unwrap()
        .contains("Approve refund of 42"));

    // The inbox task exists for audit and is completed by the agent
    let (_, task) = harness
        .find_node_by_prefix("raisin:access_control", "/agents/refund-approver/inbox/")
        .expect("task node exists");
    assert_eq!(task["properties"]["status"], json!("completed"));
    assert_eq!(
        task["properties"]["completed_by"],
        json!("/agents/refund-approver")
    );
    assert_eq!(task["properties"]["response"]["action"], json!("approve"));

    // Downstream sees the same contract as a human response
    assert_eq!(
        instance.variables["__human_response"]["action"],
        json!("approve")
    );
}

#[tokio::test]
async fn agent_assignee_low_confidence_escalates_to_human() {
    let harness = Harness::new();
    seed_agent(&harness, "/agents/refund-approver");
    harness.script_ai(json!({
        "content": "{\"decision\": \"approve\", \"reasoning\": \"unsure\", \"confidence\": 0.2}",
        "finish_reason": "stop"
    }));

    let id = run_to_quiescence(
        &harness,
        agent_assignee_flow(json!({"escalation_assignee": "/users/boss"})),
        json!({"amount": 9999}),
    )
    .await;

    // Flow waits for the human
    let instance = harness.instance(&id);
    assert_eq!(instance.status, FlowStatus::Waiting);
    assert_eq!(
        instance.wait_info.as_ref().unwrap().wait_type,
        WaitType::HumanTask
    );

    // Task reassigned to the escalation human with audit fields
    let (_, task) = harness
        .find_node_by_prefix("raisin:access_control", "/agents/refund-approver/inbox/")
        .unwrap();
    assert_eq!(task["properties"]["status"], json!("pending"));
    assert_eq!(task["properties"]["assignee"], json!("/users/boss"));
    assert_eq!(
        task["properties"]["escalated_from"],
        json!("/agents/refund-approver")
    );
}

#[tokio::test]
async fn agent_assignee_ai_error_escalates() {
    let harness = Harness::new();
    seed_agent(&harness, "/agents/refund-approver");
    // No scripted AI response -> call_ai errors

    let id = run_to_quiescence(
        &harness,
        agent_assignee_flow(json!({"escalation_assignee": "/users/boss"})),
        json!({"amount": 10}),
    )
    .await;

    let instance = harness.instance(&id);
    assert_eq!(instance.status, FlowStatus::Waiting);
    let (_, task) = harness
        .find_node_by_prefix("raisin:access_control", "/agents/refund-approver/inbox/")
        .unwrap();
    assert_eq!(task["properties"]["assignee"], json!("/users/boss"));
    assert!(task["properties"]["escalation_reason"]
        .as_str()
        .unwrap()
        .contains("agent error"));
}

// ===========================================================================
// 8. Sub-flows
// ===========================================================================

#[tokio::test]
async fn sub_flow_completes_and_resumes_parent() {
    let harness = Harness::new();
    harness.script_function(
        "/lib/child-work",
        json!({"success": true, "result": {"answer": 42}}),
    );
    harness.script_function(
        "/lib/after",
        json!({"success": true, "result": {"done": true}}),
    );

    // Child flow definition stored as a raisin:Flow node
    harness.seed_node(
        "functions",
        "/flows/child",
        json!({
            "node_type": "raisin:Flow",
            "path": "/flows/child",
            "properties": {
                "workflow_data": {
                    "nodes": [
                        { "id": "start", "step_type": "start", "next_node": "work" },
                        {
                            "id": "work",
                            "step_type": "function_step",
                            "properties": {
                                "function_ref": "/lib/child-work",
                                "arguments": { "n": "${input.n}" }
                            },
                            "next_node": "end"
                        },
                        { "id": "end", "step_type": "end" }
                    ]
                }
            }
        }),
    );

    let parent_flow = json!({
        "nodes": [
            { "id": "start", "step_type": "start", "next_node": "sub" },
            {
                "id": "sub",
                "step_type": "sub_flow",
                "properties": {
                    "flow_ref": "/flows/child",
                    "input_mapping": { "n": "${input.value}" }
                },
                "next_node": "after"
            },
            {
                "id": "after",
                "step_type": "function_step",
                "properties": {
                    "function_ref": "/lib/after",
                    "arguments": { "from_child": "${steps.sub.answer}" }
                },
                "next_node": "end"
            },
            { "id": "end", "step_type": "end" }
        ]
    });

    let id = run_to_quiescence(&harness, parent_flow, json!({"value": 7})).await;

    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "error: {:?}",
        instance.error
    );

    let invocations = harness.function_invocations.lock().unwrap();
    // Child received the mapped input
    let child_call = invocations
        .iter()
        .find(|(f, _)| f == "/lib/child-work")
        .unwrap();
    assert_eq!(child_call.1["n"], json!(7));
    // Parent's next step saw the child's output via steps.sub
    let after_call = invocations.iter().find(|(f, _)| f == "/lib/after").unwrap();
    assert_eq!(after_call.1["from_child"], json!(42));
}

// ===========================================================================
// 9. Parallel fork/join
// ===========================================================================

#[tokio::test]
async fn parallel_forks_branches_and_joins_outputs() {
    let harness = Harness::new();
    harness.script_function(
        "/lib/branch-a",
        json!({"success": true, "result": {"a": 1}}),
    );
    harness.script_function(
        "/lib/branch-b",
        json!({"success": true, "result": {"b": 2}}),
    );

    let branch_def = |func: &str| {
        json!({
            "nodes": [
                { "id": "start", "step_type": "start", "next_node": "work" },
                {
                    "id": "work",
                    "step_type": "function_step",
                    "properties": { "function_ref": func },
                    "next_node": "end"
                },
                { "id": "end", "step_type": "end" }
            ]
        })
    };

    let flow = json!({
        "nodes": [
            { "id": "start", "step_type": "start", "next_node": "par" },
            {
                "id": "par",
                "step_type": "parallel",
                "properties": {
                    "branches": [
                        {
                            "id": "a",
                            "flow_definition": branch_def("/lib/branch-a"),
                            "input_mapping": { "tag": "${input.tag}" }
                        },
                        {
                            "id": "b",
                            "flow_definition": branch_def("/lib/branch-b")
                        }
                    ],
                    "merge_strategy": "merge_all"
                },
                "next_node": "end"
            },
            { "id": "end", "step_type": "end" }
        ]
    });

    let id = run_to_quiescence(&harness, flow, json!({"tag": "x"})).await;

    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "error: {:?}",
        instance.error
    );

    // Both branch functions ran
    let invocations = harness.function_invocations.lock().unwrap();
    let called: Vec<&str> = invocations.iter().map(|(f, _)| f.as_str()).collect();
    assert!(called.contains(&"/lib/branch-a"));
    assert!(called.contains(&"/lib/branch-b"));

    // Joined output landed in the parallel step's output
    let par_output = &instance.variables["step_outputs"]["par"];
    assert_eq!(par_output["branch_0"]["status"], json!("completed"));
    assert_eq!(par_output["branch_1"]["status"], json!("completed"));
}

// ===========================================================================
// 10. Scheduled waits (delay)
// ===========================================================================

#[tokio::test]
async fn delay_wait_resumes_at_due_time_not_before() {
    let harness = Harness::new();

    let flow = json!({
        "nodes": [
            { "id": "start", "step_type": "start", "next_node": "pause" },
            {
                "id": "pause",
                "step_type": "wait",
                "properties": { "wait_type": "delay", "duration": "1h" },
                "next_node": "end"
            },
            { "id": "end", "step_type": "end" }
        ]
    });

    // Start without pumping so the scheduled wake-up stays queued
    let id = harness.seed_instance(make_instance(flow, json!({})));
    let _ = execute_flow(&id, &harness).await;

    let instance = harness.instance(&id);
    assert_eq!(instance.status, FlowStatus::Waiting);
    assert_eq!(
        instance.wait_info.as_ref().unwrap().wait_type,
        WaitType::Scheduled
    );

    // An early timeout check (before the wake-up time) must NOT resume
    let _ = check_flow_timeout(&id, &harness).await;
    assert_eq!(harness.instance(&id).status, FlowStatus::Waiting);

    // Pumping delivers the scheduled wake-up at its due time (virtual time)
    harness.pump().await;
    assert_eq!(harness.instance(&id).status, FlowStatus::Completed);
}

// ===========================================================================
// 11. Designer-format flows (the format the visual designer saves)
// ===========================================================================

/// A flow authored in the designer format (node_type: raisin:FlowStep, no
/// explicit start/end, no next_node chaining) must run end to end: the
/// converter injects start/end, chains next_node, and maps human-task
/// properties (action -> title, task_description -> description).
#[tokio::test]
async fn designer_format_human_task_flow_runs_end_to_end() {
    let harness = Harness::new();

    let flow = json!({
        "version": 1,
        "error_strategy": "fail_fast",
        "nodes": [
            {
                "id": "approve",
                "node_type": "raisin:FlowStep",
                "properties": {
                    "action": "Approve order {{ input.order_id }}",
                    "step_type": "human_task",
                    "task_type": "approval",
                    "assignee": "/users/manager",
                    "task_description": "Please review",
                    "priority": 4,
                    "options": [
                        {"value": "approve", "label": "Approve", "style": "success"},
                        {"value": "reject", "label": "Reject", "style": "danger"}
                    ]
                }
            }
        ]
    });

    let id = run_to_quiescence(&harness, flow, json!({"order_id": "ORD-9"})).await;

    // Waits on the human task with fully mapped properties
    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Waiting,
        "error: {:?}",
        instance.error
    );
    let (_, task) = harness
        .find_node_by_prefix("raisin:access_control", "/users/manager/inbox/")
        .expect("inbox task created from designer-format flow");
    assert_eq!(task["properties"]["title"], json!("Approve order ORD-9"));
    assert_eq!(task["properties"]["description"], json!("Please review"));
    assert_eq!(task["properties"]["priority"], json!(4));
    assert_eq!(task["properties"]["task_type"], json!("approval"));

    // Completing the task resumes through the injected end node
    let _ = resume_flow(&id, json!({"action": "approve"}), &harness).await;
    harness.pump().await;
    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "error: {:?}",
        instance.error
    );
}

// ===========================================================================
// 12. Designer containers: AND / OR (REL routing) / Parallel / AI sequence
// ===========================================================================

fn designer_step(id: &str, func: &str, args: Value) -> Value {
    json!({
        "id": id,
        "node_type": "raisin:FlowStep",
        "properties": { "function_ref": func, "arguments": args }
    })
}

/// AND container: all children execute sequentially, then the flow continues.
#[tokio::test]
async fn designer_and_container_runs_children_sequentially() {
    let harness = Harness::new();
    harness.script_function("/lib/a", json!({"success": true, "result": {"a": 1}}));
    harness.script_function("/lib/b", json!({"success": true, "result": {"b": 2}}));
    harness.script_function(
        "/lib/after",
        json!({"success": true, "result": {"after": true}}),
    );

    let flow = json!({
        "version": 1,
        "error_strategy": "fail_fast",
        "nodes": [
            {
                "id": "both",
                "node_type": "raisin:FlowContainer",
                "container_type": "and",
                "children": [
                    designer_step("a", "/lib/a", json!({})),
                    designer_step("b", "/lib/b", json!({"prev": "${steps.a.a}"}))
                ]
            },
            designer_step("after", "/lib/after", json!({}))
        ]
    });

    let id = run_to_quiescence(&harness, flow, json!({})).await;
    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "error: {:?}",
        instance.error
    );

    let invocations = harness.function_invocations.lock().unwrap();
    let called: Vec<&str> = invocations.iter().map(|(f, _)| f.as_str()).collect();
    assert_eq!(called, vec!["/lib/a", "/lib/b", "/lib/after"]);
    // Cross-child template resolution inside the container
    assert_eq!(invocations[1].1["prev"], json!(1));
}

/// OR container: REL rules route to exactly one child; the rest are skipped.
#[tokio::test]
async fn designer_or_container_routes_by_rel_rules() {
    for (tier, expected_fn) in [("premium", "/lib/vip"), ("basic", "/lib/standard")] {
        let harness = Harness::new();
        harness.script_function(
            "/lib/vip",
            json!({"success": true, "result": {"path": "vip"}}),
        );
        harness.script_function(
            "/lib/standard",
            json!({"success": true, "result": {"path": "standard"}}),
        );
        harness.script_function("/lib/done", json!({"success": true, "result": {}}));

        let flow = json!({
            "version": 1,
            "error_strategy": "fail_fast",
            "nodes": [
                {
                    "id": "route",
                    "node_type": "raisin:FlowContainer",
                    "container_type": "or",
                    "rules": [
                        { "condition": "input.tier == \"premium\"", "next_step": "vip" },
                        { "condition": "input.tier == \"basic\"", "next_step": "standard" }
                    ],
                    "children": [
                        designer_step("vip", "/lib/vip", json!({})),
                        designer_step("standard", "/lib/standard", json!({}))
                    ]
                },
                designer_step("done", "/lib/done", json!({}))
            ]
        });

        let id = run_to_quiescence(&harness, flow, json!({"tier": tier})).await;
        let instance = harness.instance(&id);
        assert_eq!(
            instance.status,
            FlowStatus::Completed,
            "error: {:?}",
            instance.error
        );

        let invocations = harness.function_invocations.lock().unwrap();
        let called: Vec<&str> = invocations.iter().map(|(f, _)| f.as_str()).collect();
        assert_eq!(
            called,
            vec![expected_fn, "/lib/done"],
            "tier {} must route to {} only",
            tier,
            expected_fn
        );
    }
}

/// OR container with no matching rule: the container is skipped entirely.
#[tokio::test]
async fn designer_or_container_skips_when_no_rule_matches() {
    let harness = Harness::new();
    harness.script_function("/lib/done", json!({"success": true, "result": {}}));

    let flow = json!({
        "version": 1,
        "error_strategy": "fail_fast",
        "nodes": [
            {
                "id": "route",
                "node_type": "raisin:FlowContainer",
                "container_type": "or",
                "rules": [
                    { "condition": "input.tier == \"premium\"", "next_step": "vip" }
                ],
                "children": [ designer_step("vip", "/lib/vip", json!({})) ]
            },
            designer_step("done", "/lib/done", json!({}))
        ]
    });

    let id = run_to_quiescence(&harness, flow, json!({"tier": "unknown"})).await;
    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "error: {:?}",
        instance.error
    );

    let invocations = harness.function_invocations.lock().unwrap();
    let called: Vec<&str> = invocations.iter().map(|(f, _)| f.as_str()).collect();
    assert_eq!(called, vec!["/lib/done"], "vip must be skipped");
}

/// Parallel designer container: each child runs as its own branch flow and
/// the merged outputs land in the container's step output.
#[tokio::test]
async fn designer_parallel_container_forks_and_joins() {
    let harness = Harness::new();
    harness.script_function("/lib/left", json!({"success": true, "result": {"left": 1}}));
    harness.script_function(
        "/lib/right",
        json!({"success": true, "result": {"right": 2}}),
    );
    harness.script_function("/lib/done", json!({"success": true, "result": {}}));

    let flow = json!({
        "version": 1,
        "error_strategy": "fail_fast",
        "nodes": [
            {
                "id": "par",
                "node_type": "raisin:FlowContainer",
                "container_type": "parallel",
                "children": [
                    designer_step("left", "/lib/left", json!({})),
                    designer_step("right", "/lib/right", json!({}))
                ]
            },
            designer_step("done", "/lib/done", json!({}))
        ]
    });

    let id = run_to_quiescence(&harness, flow, json!({})).await;
    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "error: {:?}",
        instance.error
    );

    let invocations = harness.function_invocations.lock().unwrap();
    let called: Vec<&str> = invocations.iter().map(|(f, _)| f.as_str()).collect();
    assert!(called.contains(&"/lib/left") && called.contains(&"/lib/right"));
    assert_eq!(*called.last().unwrap(), "/lib/done");

    let par_output = &instance.variables["step_outputs"]["par"];
    assert_eq!(par_output["branch_0"]["status"], json!("completed"));
    assert_eq!(par_output["branch_1"]["status"], json!("completed"));
}

/// Designer ai_sequence container: lowered to an AIContainer step that calls
/// the agent and continues when the agent finishes (no tool calls).
#[tokio::test]
async fn designer_ai_sequence_container_completes() {
    let harness = Harness::new();
    harness.script_ai(json!({"content": "All done.", "finish_reason": "stop"}));
    harness.script_function("/lib/done", json!({"success": true, "result": {}}));

    let flow = json!({
        "version": 1,
        "error_strategy": "fail_fast",
        "nodes": [
            {
                "id": "assistant",
                "node_type": "raisin:FlowContainer",
                "container_type": "ai_sequence",
                "ai_config": {
                    "agent_ref": { "raisin:ref": "/agents/helper", "raisin:workspace": "functions" },
                    "tool_mode": "auto",
                    "explicit_tools": [],
                    "max_iterations": 5,
                    "thinking_enabled": false,
                    "on_error": "stop"
                },
                "children": []
            },
            designer_step("done", "/lib/done", json!({}))
        ]
    });

    let id = run_to_quiescence(&harness, flow, json!({"message": "hi"})).await;
    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "error: {:?}",
        instance.error
    );

    assert_eq!(harness.ai_calls.lock().unwrap().len(), 1);
    let out = &instance.variables["step_outputs"]["assistant"];
    assert_eq!(out["response"], json!("All done."));
}

// ===========================================================================
// 22. Test-run mocks: functions and agents bypass real execution
// ===========================================================================

#[tokio::test]
async fn test_run_mocks_bypass_functions_and_agents() {
    use raisin_flow_runtime::types::{FunctionMock, MockBehavior, TestRunConfig};

    let harness = Harness::new();
    // Deliberately NO scripted function and NO scripted AI response:
    // if the mocks don't intercept, the run fails.

    let flow = json!({
        "nodes": [
            { "id": "start", "step_type": "start", "next_node": "charge" },
            {
                "id": "charge",
                "step_type": "function_step",
                "properties": {
                    "function_ref": "/lib/charge-card",
                    "arguments": { "amount": "{{ input.amount }}" }
                },
                "next_node": "review"
            },
            {
                "id": "review",
                "step_type": "agent_step",
                "properties": {
                    "agent_ref": "/agents/reviewer",
                    "prompt": "Review the charge"
                },
                "next_node": "end"
            },
            { "id": "end", "step_type": "end" }
        ]
    });

    let test_config = TestRunConfig::new()
        .with_mock(
            "/lib/charge-card".to_string(),
            FunctionMock {
                behavior: MockBehavior::MockOutput,
                mock_output: Some(json!({"charged": true, "tx_id": "mock-tx-1"})),
                mock_delay_ms: None,
            },
        )
        .with_agent_mock(
            "/agents/reviewer".to_string(),
            FunctionMock {
                behavior: MockBehavior::MockOutput,
                mock_output: Some(json!({"decision": "approve", "confidence": 0.99})),
                mock_delay_ms: None,
            },
        );

    let mut instance = make_instance(flow, json!({"amount": 42}));
    instance.test_config = Some(test_config);
    let id = harness.seed_instance(instance);
    execute_flow(&id, &harness).await.expect("flow should run");
    harness.pump().await;

    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "{:?}",
        instance.error
    );

    // Mock outputs recorded as step outputs (downstream templates work)
    assert_eq!(
        instance.variables["step_outputs"]["charge"]["tx_id"],
        json!("mock-tx-1")
    );
    assert_eq!(
        instance.variables["step_outputs"]["review"]["decision"],
        json!("approve")
    );

    // No real AI call was made
    assert_eq!(harness.ai_calls.lock().unwrap().len(), 0);
}

// ===========================================================================
// 23. Agent step executes its agent's own tools in an internal loop
// ===========================================================================

#[tokio::test]
async fn agent_step_runs_internal_tool_loop() {
    let harness = Harness::new();

    // Turn 1: the agent answers with a tool call (lookup-price), turn 2: final
    harness.script_ai(json!({
        "content": "",
        "finish_reason": "tool_calls",
        "tool_calls": [{
            "id": "call_1",
            "type": "function",
            "function": { "name": "lookup-price", "arguments": "{\"sku\": \"A-1\"}" }
        }],
        "_tool_map": { "lookup-price": "/lib/pricing/lookup-price" }
    }));
    harness.script_ai(json!({
        "content": "The price of A-1 is 42 CHF.",
        "finish_reason": "stop"
    }));

    let flow = json!({
        "nodes": [
            { "id": "start", "step_type": "start", "next_node": "ask" },
            {
                "id": "ask",
                "step_type": "agent_step",
                "properties": {
                    "agent_ref": "/agents/pricing",
                    "prompt": "What does {{ input.sku }} cost?"
                },
                "next_node": "end"
            },
            { "id": "end", "step_type": "end" }
        ]
    });

    let id = run_to_quiescence(&harness, flow, json!({"sku": "A-1"})).await;
    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "{:?}",
        instance.error
    );

    // Two AI turns happened; the second one carried the tool transcript
    let calls = harness.ai_calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    let second_turn = &calls[1].2;
    assert_eq!(second_turn.len(), 3); // user + assistant(tool_calls) + tool result
    assert_eq!(second_turn[1]["role"], "assistant");
    assert_eq!(second_turn[2]["role"], "tool");
    assert_eq!(second_turn[2]["tool_call_id"], "call_1");
    drop(calls);

    // The tool was executed synchronously with parsed JSON arguments,
    // resolved through _tool_map to the function path
    let executions = harness.sync_executions.lock().unwrap();
    assert_eq!(executions.len(), 1);
    assert_eq!(executions[0].0, "/lib/pricing/lookup-price");
    assert_eq!(executions[0].1, json!({"sku": "A-1"}));
    drop(executions);

    // Step output records the final answer + tool audit trail
    let out = &instance.variables["step_outputs"]["ask"];
    assert_eq!(out["response"], json!("The price of A-1 is 42 CHF."));
    assert_eq!(out["tools_used"][0]["name"], json!("lookup-price"));
    assert_eq!(out["tool_iterations"], json!(1));
}

// ===========================================================================
// 24. AI-routed OR container: agent picks the branch; low confidence falls back
// ===========================================================================

fn ai_routed_flow() -> Value {
    json!({
        "nodes": [
            { "id": "start", "node_type": "raisin:FlowStep",
              "properties": { "action": "noop", "function_ref": "/lib/noop" } },
            { "id": "route", "node_type": "raisin:FlowContainer", "container_type": "or",
              "rules": [ { "condition": "input.amount > 10000", "next_step": "escalate" } ],
              "router": {
                  "agent_ref": "/agents/dispatcher",
                  "prompt": "Order from {{ input.customer }} for {{ input.amount }} CHF. Route it.",
                  "min_confidence": 0.6,
                  "default_branch": "standard"
              },
              "children": [
                  { "id": "escalate", "node_type": "raisin:FlowStep",
                    "properties": { "action": "Escalate to ops", "function_ref": "/lib/escalate" } },
                  { "id": "vip", "node_type": "raisin:FlowStep",
                    "properties": { "action": "VIP handling", "function_ref": "/lib/vip" } },
                  { "id": "standard", "node_type": "raisin:FlowStep",
                    "properties": { "action": "Standard handling", "function_ref": "/lib/standard" } }
              ] }
        ]
    })
}

#[tokio::test]
async fn ai_router_routes_to_agent_chosen_branch() {
    let harness = Harness::new();
    harness.script_function("/lib/noop", json!({"success": true, "result": {}}));
    harness.script_function(
        "/lib/vip",
        json!({"success": true, "result": {"handled": "vip"}}),
    );
    // Agent confidently picks the vip branch
    harness.script_ai(json!({
        "content": "{\"branch\": \"vip\", \"reasoning\": \"known premium customer\", \"confidence\": 0.92}",
        "finish_reason": "stop"
    }));

    // amount below the REL rule threshold -> falls through to the agent
    let id = run_to_quiescence(
        &harness,
        ai_routed_flow(),
        json!({"customer": "ACME", "amount": 750}),
    )
    .await;
    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "{:?}",
        instance.error
    );

    // The agent saw the templated prompt and the branch list
    let calls = harness.ai_calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, "/agents/dispatcher");
    let prompt = calls[0].2[0]["content"].as_str().unwrap();
    assert!(prompt.contains("Order from ACME for 750 CHF"));
    assert!(prompt.contains("- vip: VIP handling"));
    // Structured output schema constrains the branch enum
    let schema = calls[0].3.as_ref().unwrap();
    assert_eq!(schema["schema"]["properties"]["branch"]["enum"][1], "vip");
    drop(calls);

    // Routed to vip; the other branches never ran
    let outputs = &instance.variables["step_outputs"];
    assert_eq!(outputs["route__router"]["routed_to"], json!("vip"));
    assert_eq!(outputs["route__router"]["routed_by_agent"], json!(true));
    assert_eq!(outputs["route__router"]["confidence"], json!(0.92));
    assert_eq!(outputs["vip"]["handled"], json!("vip"));
    assert!(outputs.get("standard").is_none());
    assert!(outputs.get("escalate").is_none());
}

#[tokio::test]
async fn ai_router_low_confidence_takes_default_branch() {
    let harness = Harness::new();
    harness.script_function("/lib/noop", json!({"success": true, "result": {}}));
    harness.script_function(
        "/lib/standard",
        json!({"success": true, "result": {"handled": "standard"}}),
    );
    // Agent is unsure -> below min_confidence 0.6
    harness.script_ai(json!({
        "content": "{\"branch\": \"vip\", \"reasoning\": \"not sure\", \"confidence\": 0.3}",
        "finish_reason": "stop"
    }));

    let id = run_to_quiescence(
        &harness,
        ai_routed_flow(),
        json!({"customer": "Unknown GmbH", "amount": 80}),
    )
    .await;
    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "{:?}",
        instance.error
    );

    let outputs = &instance.variables["step_outputs"];
    assert_eq!(outputs["route__router"]["routed_to"], json!("standard"));
    assert_eq!(outputs["route__router"]["routed_by_agent"], json!(false));
    assert_eq!(outputs["standard"]["handled"], json!("standard"));
    assert!(outputs.get("vip").is_none());
}

#[tokio::test]
async fn ai_router_rel_rule_preempts_agent() {
    let harness = Harness::new();
    harness.script_function("/lib/noop", json!({"success": true, "result": {}}));
    harness.script_function(
        "/lib/escalate",
        json!({"success": true, "result": {"handled": "escalated"}}),
    );
    // NO scripted AI response: the REL rule must route without the agent

    let id = run_to_quiescence(
        &harness,
        ai_routed_flow(),
        json!({"customer": "Whale Corp", "amount": 50000}),
    )
    .await;
    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "{:?}",
        instance.error
    );

    // Deterministic rule won; the agent was never consulted
    assert_eq!(harness.ai_calls.lock().unwrap().len(), 0);
    let outputs = &instance.variables["step_outputs"];
    assert_eq!(outputs["escalate"]["handled"], json!("escalated"));
}

// ===========================================================================
// 25. Competing agents with referee: accept, and refine-then-accept
// ===========================================================================

fn competition_flow(max_rounds: u32) -> Value {
    json!({
        "nodes": [
            { "id": "compete", "node_type": "raisin:FlowContainer",
              "container_type": "competition",
              "prompt": "Write a tagline for {{ input.product }}.",
              "referee": {
                  "agent_ref": "/agents/referee",
                  "min_confidence": 0.7,
                  "max_rounds": max_rounds
              },
              "children": [
                  { "id": "claude_writer", "node_type": "raisin:FlowStep",
                    "properties": { "action": "Claude writer", "step_type": "ai_agent",
                                    "agent_ref": "/agents/writer-claude" } },
                  { "id": "gpt_writer", "node_type": "raisin:FlowStep",
                    "properties": { "action": "GPT writer", "step_type": "ai_agent",
                                    "agent_ref": "/agents/writer-gpt" } }
              ] }
        ]
    })
}

#[tokio::test]
async fn competition_referee_accepts_winner() {
    let harness = Harness::new();
    // Round 1: two competitors answer, referee accepts confidently
    harness.script_ai(json!({ "content": "Taste the cloud.", "model": "claude" }));
    harness.script_ai(json!({ "content": "Sky in a cup.", "model": "gpt" }));
    harness.script_ai(json!({
        "content": "{\"action\": \"accept\", \"winner\": \"gpt_writer\", \"confidence\": 0.9, \"reasoning\": \"more concrete\"}"
    }));

    let id = run_to_quiescence(
        &harness,
        competition_flow(1),
        json!({"product": "AirCoffee"}),
    )
    .await;
    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "{:?}",
        instance.error
    );

    let calls = harness.ai_calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0].1, "/agents/writer-claude");
    assert_eq!(calls[1].1, "/agents/writer-gpt");
    assert_eq!(calls[2].1, "/agents/referee");
    // Competitors got the templated shared task
    assert!(calls[0].2[0]["content"]
        .as_str()
        .unwrap()
        .contains("AirCoffee"));
    // Referee saw both answers and a structured-verdict schema
    let referee_prompt = calls[2].2[0]["content"].as_str().unwrap();
    assert!(referee_prompt.contains("Taste the cloud."));
    assert!(referee_prompt.contains("Sky in a cup."));
    assert_eq!(
        calls[2].3.as_ref().unwrap()["schema"]["properties"]["winner"]["enum"][0],
        "claude_writer"
    );
    drop(calls);

    let out = &instance.variables["step_outputs"]["compete"];
    assert_eq!(out["winner"], json!("gpt_writer"));
    assert_eq!(out["response"], json!("Sky in a cup."));
    assert_eq!(out["confidence"], json!(0.9));
    assert_eq!(out["confident"], json!(true));
    assert_eq!(out["rounds"], json!(1));
}

#[tokio::test]
async fn competition_referee_refines_then_accepts() {
    let harness = Harness::new();
    // Round 1: answers + referee asks ONLY claude_writer to refine
    harness.script_ai(json!({ "content": "Coffee but lighter." }));
    harness.script_ai(json!({ "content": "Sky in a cup." }));
    harness.script_ai(json!({
        "content": "{\"action\": \"refine\", \"winner\": \"gpt_writer\", \"confidence\": 0.5, \"feedback\": {\"claude_writer\": \"Too vague - name the product.\"}}"
    }));
    // Round 2: only claude_writer re-answers, referee accepts it
    harness.script_ai(json!({ "content": "AirCoffee: coffee, but lighter." }));
    harness.script_ai(json!({
        "content": "{\"action\": \"accept\", \"winner\": \"claude_writer\", \"confidence\": 0.85, \"reasoning\": \"now names the product\"}"
    }));

    let id = run_to_quiescence(
        &harness,
        competition_flow(1),
        json!({"product": "AirCoffee"}),
    )
    .await;
    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "{:?}",
        instance.error
    );

    let calls = harness.ai_calls.lock().unwrap();
    assert_eq!(calls.len(), 5); // 2 + referee + 1 refined + referee
                                // The refinement turn carried previous answer + referee feedback
    let refine_prompt = calls[3].2[0]["content"].as_str().unwrap();
    assert!(refine_prompt.contains("Coffee but lighter."));
    assert!(refine_prompt.contains("Too vague - name the product."));
    // gpt_writer was NOT re-asked (its answer stood)
    assert_eq!(calls[3].1, "/agents/writer-claude");
    drop(calls);

    let out = &instance.variables["step_outputs"]["compete"];
    assert_eq!(out["winner"], json!("claude_writer"));
    assert_eq!(out["response"], json!("AirCoffee: coffee, but lighter."));
    assert_eq!(out["rounds"], json!(2));
    assert_eq!(out["confident"], json!(true));
}

// ===========================================================================
// 26. include_context: agents receive the workflow context automatically
// ===========================================================================

#[tokio::test]
async fn agent_step_include_context_injects_workflow_state() {
    let harness = Harness::new();
    harness.script_function(
        "/lib/reserve",
        json!({"success": true, "result": {"reservation_id": "res-77", "total": 450}}),
    );
    harness.script_ai(json!({ "content": "Looks fine.", "finish_reason": "stop" }));

    let flow = json!({
        "nodes": [
            { "id": "start", "step_type": "start", "next_node": "reserve" },
            { "id": "reserve", "step_type": "function_step",
              "properties": { "function_ref": "/lib/reserve" }, "next_node": "review" },
            { "id": "review", "step_type": "agent_step",
              "properties": {
                  "agent_ref": "/agents/reviewer",
                  "prompt": "Review this order.",
                  "include_context": "full"
              }, "next_node": "end" },
            { "id": "end", "step_type": "end" }
        ]
    });

    let id = run_to_quiescence(&harness, flow, json!({"customer": "ACME"})).await;
    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "{:?}",
        instance.error
    );

    // The agent prompt contains the prompt text AND the injected context:
    // flow input + the previous step's output - without any templating
    let calls = harness.ai_calls.lock().unwrap();
    let prompt = calls[0].2[0]["content"].as_str().unwrap();
    assert!(prompt.starts_with("Review this order."));
    assert!(prompt.contains("# Workflow context"));
    assert!(prompt.contains("\"customer\": \"ACME\""));
    assert!(prompt.contains("\"reservation_id\": \"res-77\""));
}

#[tokio::test]
async fn ai_container_uses_templated_prompt_for_manual_runs() {
    let harness = Harness::new();
    harness.script_ai(json!({ "content": "Done.", "finish_reason": "stop" }));

    // Designer format: ai_sequence with a container-level prompt
    let flow = json!({
        "nodes": [
            { "id": "assist", "node_type": "raisin:FlowContainer",
              "container_type": "ai_sequence",
              "prompt": "Handle the request from {{ input.customer }}.",
              "ai_config": { "agent_ref": "/agents/assistant", "include_context": "input" },
              "children": [] }
        ]
    });

    let id = run_to_quiescence(
        &harness,
        flow,
        json!({"customer": "ACME", "urgency": "high"}),
    )
    .await;
    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "{:?}",
        instance.error
    );

    let calls = harness.ai_calls.lock().unwrap();
    assert!(
        !calls.is_empty(),
        "AI container should have called the agent"
    );
    let first_user = calls[0]
        .2
        .iter()
        .find(|m| m["role"] == "user")
        .expect("user message");
    let content = first_user["content"].as_str().unwrap();
    assert!(content.contains("Handle the request from ACME."));
    assert!(content.contains("\"urgency\": \"high\""));
}

// ===========================================================================
// 12. Shipped example: shiftboard fill-shift flow (designer format)
// ===========================================================================

/// Load the EXACT workflow_data shipped in the shiftboard example package
/// (examples/shiftboard/package/content/functions/flows/fill-shift/.node.yaml)
/// so the tutorial's flow definition is engine-validated by CI.
fn shiftboard_fill_shift_workflow() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/shiftboard/package/content/functions/flows/fill-shift/.node.yaml");
    let yaml = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {}", path.display(), e));
    let node: serde_yaml::Value = serde_yaml::from_str(&yaml).expect("valid YAML");
    let workflow_data = serde_json::to_value(&node["properties"]["workflow_data"])
        .expect("workflow_data converts to JSON");
    assert!(
        workflow_data.get("nodes").is_some(),
        "fill-shift .node.yaml must contain properties.workflow_data.nodes"
    );
    workflow_data
}

/// Start the fill-shift flow and pump in REAL TIME (the human-task
/// deadlines must not auto-fire; the candidates answer first).
async fn start_fill_shift(harness: &Harness, input: Value) -> String {
    let id = harness.seed_instance(make_instance(shiftboard_fill_shift_workflow(), input));
    execute_flow(&id, harness).await.expect("flow starts");
    harness.pump_real_time().await;
    id
}

/// The shiftboard "fill shift via inbox tasks" scenario: pick two
/// candidates, the loop asks Anna first (human task, flow waits), she
/// DECLINES, the loop moves on to Cara, she ACCEPTS (loop `until` exits
/// early), the accepter is resolved, the or-gate routes into assign-shift,
/// and the manager is notified - all from the shipped designer definition.
#[tokio::test]
async fn shiftboard_fill_shift_decline_then_accept() {
    let harness = Harness::new();

    let candidates = json!([
        {"name": "Anna", "email": "anna@example.com",
         "user_path": "/users/internal/anna-at-example-com"},
        {"name": "Cara", "email": "cara@example.com",
         "user_path": "/users/internal/cara-at-example-com"},
    ]);
    harness.script_function(
        "/lib/shiftboard/pick-candidates",
        json!({"success": true, "result": {
            "shift_path": "/shifts/sun-evening",
            "shift_title": "Sunday Evening",
            "day": "sunday", "start": "15:00", "end": "21:00",
            "location": "Restaurant",
            "candidates": candidates,
        }}),
    );
    let summary = "Cara accepted Sunday Evening (sunday 15:00-21:00, Restaurant) \
                   via inbox task and has been assigned. Declined before that: Anna.";
    harness.script_function(
        "/lib/shiftboard/resolve-accepter",
        json!({"success": true, "result": {
            "accepted": true,
            "accepter_name": "Cara",
            "accepter_email": "cara@example.com",
            "asked": 2,
            "declined_names": ["Anna"],
            "summary": summary,
        }}),
    );
    harness.script_function(
        "/lib/shiftboard/assign-shift",
        json!({"success": true, "result": {
            "shift_path": "/shifts/sun-evening", "assignee": "Cara", "status": "filled",
        }}),
    );
    harness.script_function(
        "/lib/shiftboard/message-staff",
        json!({"success": true, "result": {
            "delivered_to": "planner@example.com", "conversation_id": "chat-1",
        }}),
    );

    // Start: pick_candidates runs, the loop creates Anna's task and waits
    let id = start_fill_shift(&harness, json!({"shift_path": "/shifts/sun-evening"})).await;

    let instance = harness.instance(&id);
    assert_eq!(instance.status, FlowStatus::Waiting, "{:?}", instance.error);
    assert_eq!(
        instance.wait_info.as_ref().unwrap().wait_type,
        WaitType::HumanTask
    );

    // Anna's approval task: templates resolved, accept/decline options
    let (anna_task_path, anna_task) = harness
        .find_node_by_prefix(
            "raisin:access_control",
            "/users/internal/anna-at-example-com/inbox/",
        )
        .expect("Anna's inbox task created");
    assert_eq!(anna_task["node_type"], json!("raisin:InboxTask"));
    assert_eq!(anna_task["properties"]["task_type"], json!("approval"));
    let title = anna_task["properties"]["title"].as_str().unwrap();
    assert!(
        title.contains("Sunday Evening") && title.contains("15:00-21:00"),
        "task title must name the shift and time, got: {}",
        title
    );
    let description = anna_task["properties"]["description"].as_str().unwrap();
    assert!(
        description.contains("Hi Anna") && description.contains("/shifts/sun-evening"),
        "description must greet the candidate and carry the shift path, got: {}",
        description
    );
    let options = anna_task["properties"]["options"].as_array().unwrap();
    let values: Vec<&str> = options.iter().filter_map(|o| o["value"].as_str()).collect();
    assert_eq!(values, vec!["accept", "decline"]);

    // Sequential ask: Cara must NOT have a task yet
    assert!(
        harness
            .find_node_by_prefix(
                "raisin:access_control",
                "/users/internal/cara-at-example-com/inbox/",
            )
            .is_none(),
        "Cara must not be asked before Anna answered"
    );

    // Anna DECLINES -> the loop continues to Cara
    let _ = resume_flow(
        &id,
        json!({"action": "decline", "comment": "Sorry, can't.",
               "completed_by": "anna@example.com", "task_path": anna_task_path}),
        &harness,
    )
    .await;
    harness.pump_real_time().await;

    let instance = harness.instance(&id);
    assert_eq!(instance.status, FlowStatus::Waiting, "{:?}", instance.error);
    let (cara_task_path, cara_task) = harness
        .find_node_by_prefix(
            "raisin:access_control",
            "/users/internal/cara-at-example-com/inbox/",
        )
        .expect("Cara's inbox task created after Anna declined");
    assert!(cara_task["properties"]["description"]
        .as_str()
        .unwrap()
        .contains("Hi Cara"));

    // Cara ACCEPTS -> loop `until` exits early, accepter resolved,
    // or-gate routes to assign-shift, manager notified, flow completes
    let _ = resume_flow(
        &id,
        json!({"action": "accept", "comment": "Sure!",
               "completed_by": "cara@example.com", "task_path": cara_task_path}),
        &harness,
    )
    .await;
    harness.pump_real_time().await;

    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "error: {:?}",
        instance.error
    );

    let invocations = harness.function_invocations.lock().unwrap();
    let by_ref: Vec<(&str, &Value)> = invocations.iter().map(|(r, a)| (r.as_str(), a)).collect();

    // pick-candidates got the templated shift path
    assert_eq!(by_ref[0].0, "/lib/shiftboard/pick-candidates");
    assert_eq!(by_ref[0].1["shift_path"], json!("/shifts/sun-evening"));

    // resolve-accepter received the candidate list AND both loop results
    let resolve = by_ref
        .iter()
        .find(|(r, _)| *r == "/lib/shiftboard/resolve-accepter")
        .expect("resolve-accepter invoked");
    assert_eq!(resolve.1["candidates"].as_array().unwrap().len(), 2);
    let results = resolve.1["results"].as_array().unwrap();
    assert_eq!(results.len(), 2, "both candidates' answers collected");
    assert_eq!(results[0]["action"], json!("decline"));
    assert_eq!(results[0]["completed_by"], json!("anna@example.com"));
    assert_eq!(results[1]["action"], json!("accept"));
    assert_eq!(results[1]["completed_by"], json!("cara@example.com"));

    // The or-gate routed into assign-shift with the resolved accepter
    let assign = by_ref
        .iter()
        .find(|(r, _)| *r == "/lib/shiftboard/assign-shift")
        .expect("assign-shift invoked");
    assert_eq!(assign.1["shift_path"], json!("/shifts/sun-evening"));
    assert_eq!(assign.1["staff_name"], json!("Cara"));

    // The manager got the outcome summary
    let notify = by_ref
        .iter()
        .find(|(r, _)| *r == "/lib/shiftboard/message-staff")
        .expect("message-staff invoked");
    assert_eq!(notify.1["staff_email"], json!("planner@example.com"));
    assert_eq!(notify.1["message"], json!(summary));
}

/// Nobody accepts: the loop exhausts the candidates, resolve-accepter
/// reports no accepter, the or-gate is SKIPPED (no rule match - no
/// assignment happens), and the manager still gets the outcome message.
#[tokio::test]
async fn shiftboard_fill_shift_nobody_accepts_skips_assignment() {
    let harness = Harness::new();

    harness.script_function(
        "/lib/shiftboard/pick-candidates",
        json!({"success": true, "result": {
            "shift_path": "/shifts/sun-evening",
            "shift_title": "Sunday Evening",
            "day": "sunday", "start": "15:00", "end": "21:00",
            "location": "Restaurant",
            "candidates": [
                {"name": "Cara", "email": "cara@example.com",
                 "user_path": "/users/internal/cara-at-example-com"},
            ],
        }}),
    );
    let summary = "Could not fill Sunday Evening: nobody accepted. Declined: Cara.";
    harness.script_function(
        "/lib/shiftboard/resolve-accepter",
        json!({"success": true, "result": {
            "accepted": false, "accepter_name": null, "accepter_email": null,
            "asked": 1, "declined_names": ["Cara"], "summary": summary,
        }}),
    );
    harness.script_function(
        "/lib/shiftboard/message-staff",
        json!({"success": true, "result": {
            "delivered_to": "planner@example.com", "conversation_id": "chat-2",
        }}),
    );
    // NOTE: no scripted assign-shift result - invoking it would panic.

    let id = start_fill_shift(&harness, json!({"shift_path": "/shifts/sun-evening"})).await;
    assert_eq!(harness.instance(&id).status, FlowStatus::Waiting);

    let (task_path, _task) = harness
        .find_node_by_prefix(
            "raisin:access_control",
            "/users/internal/cara-at-example-com/inbox/",
        )
        .expect("Cara's inbox task created");
    let _ = resume_flow(
        &id,
        json!({"action": "decline", "completed_by": "cara@example.com",
               "task_path": task_path}),
        &harness,
    )
    .await;
    harness.pump_real_time().await;

    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "error: {:?}",
        instance.error
    );

    let invocations = harness.function_invocations.lock().unwrap();
    assert!(
        !invocations
            .iter()
            .any(|(r, _)| r == "/lib/shiftboard/assign-shift"),
        "assign-shift must NOT run when nobody accepted"
    );
    let notify = invocations
        .iter()
        .find(|(r, _)| r == "/lib/shiftboard/message-staff")
        .expect("manager must still be notified");
    assert_eq!(notify.1["message"], json!(summary));
}

// ===========================================================================
// 20. Dynamic fan-out of human tasks, joined (per-item review)
// ===========================================================================

/// The shape an application needs for "one task per row, then join": fan out
/// over a runtime collection, park a human task per item, and have the FLOW
/// own the join rather than deriving a roll-up from child state outside it.
///
/// Exercises, end to end and through the real job pump:
/// - `for_each` resolving a collection produced by an UPSTREAM step
/// - one child flow instance per item, each referencing a DEPLOYED flow by
///   path (so `flow_execution` + `branch_id` correlation is on the hook)
/// - `item` / `index` bound in the branch template's `input_mapping`
/// - a templated `due_in_seconds` inside the branch (the per-item deadline)
/// - a CUSTOM `task_type` (not one of the canonical four)
/// - the join waiting for EVERY child, then merging by branch id
#[tokio::test]
async fn fan_out_of_human_tasks_joins_after_every_person_answers() {
    let harness = Harness::new();

    // The deployed per-item flow: ask its owner, then record the answer.
    harness.seed_node(
        "functions",
        "/flows/review-item",
        json!({
            "properties": {
                "workflow_data": {
                    "nodes": [
                        { "id": "start", "step_type": "start", "next_node": "review" },
                        {
                            "id": "review",
                            "step_type": "human_task",
                            "properties": {
                                // Application-defined task type - would have
                                // been rejected outright before.
                                "task_type": "deliverable_review",
                                "title": "Review {{ input.item.name }}",
                                "assignee": "${input.item.owner}",
                                // Per-ITEM deadline, from data. Read
                                // pre-template this silently vanished.
                                "due_in_seconds": "${input.item.sla_seconds}",
                                "priority": "${input.item.priority}"
                            },
                            "next_node": "end"
                        },
                        { "id": "end", "step_type": "end" }
                    ]
                }
            }
        }),
    );

    harness.script_function(
        "/lib/plan",
        json!({"success": true, "result": {"items": [
            {"name": "Hero image", "owner": "/users/ana", "sla_seconds": 3600, "priority": 5},
            {"name": "Launch copy", "owner": "/users/bo", "sla_seconds": 7200, "priority": 2}
        ]}}),
    );
    harness.script_function(
        "/lib/publish",
        json!({"success": true, "result": {"published": true}}),
    );

    let flow = json!({
        "nodes": [
            { "id": "start", "step_type": "start", "next_node": "plan" },
            {
                "id": "plan",
                "step_type": "function_step",
                "properties": { "function_ref": "/lib/plan" },
                "next_node": "collect_reviews"
            },
            {
                "id": "collect_reviews",
                "step_type": "parallel",
                "properties": {
                    "for_each": "${steps.plan.items}",
                    "max_branches": 50,
                    "branch": {
                        "id": "review-${index}",
                        "flow_path": "/flows/review-item",
                        "input_mapping": { "item": "${item}", "index": "${index}" }
                    },
                    "merge_strategy": "all_success"
                },
                "next_node": "publish"
            },
            {
                "id": "publish",
                "step_type": "function_step",
                "properties": { "function_ref": "/lib/publish" },
                "next_node": "end"
            },
            { "id": "end", "step_type": "end" }
        ]
    });

    // Real-time pump: the per-item deadlines are an hour out, and the
    // virtual-time pump deliberately forces future deadlines to fire.
    let id = harness.seed_instance(make_instance(flow, json!({"campaign": "spring"})));
    let _ = execute_flow(&id, &harness).await;
    harness.pump_real_time().await;

    // ---- The parent is parked on the join, not finished ------------------
    let parent = harness.instance(&id);
    assert_eq!(
        parent.status,
        FlowStatus::Waiting,
        "parent must wait for the fan-out; error: {:?}",
        parent.error
    );
    assert!(
        harness
            .function_invocations
            .lock()
            .unwrap()
            .iter()
            .all(|(f, _)| f != "/lib/publish"),
        "publish must NOT run before every review is in"
    );

    // ---- One task per item, each with its OWN resolved deadline ---------
    let (ana_path, ana_task) = harness
        .find_node_by_prefix("raisin:access_control", "/users/ana/inbox/")
        .expect("a task was created for the first item's owner");
    let (bo_path, bo_task) = harness
        .find_node_by_prefix("raisin:access_control", "/users/bo/inbox/")
        .expect("a task was created for the second item's owner");

    assert_eq!(ana_task["properties"]["title"], json!("Review Hero image"));
    assert_eq!(bo_task["properties"]["title"], json!("Review Launch copy"));

    // Custom task type survived the whole round trip
    assert_eq!(
        ana_task["properties"]["task_type"],
        json!("deliverable_review")
    );

    // Per-item templated deadline + priority actually landed
    assert_eq!(ana_task["properties"]["due_in_seconds"], json!(3600));
    assert_eq!(ana_task["properties"]["priority"], json!(5));
    assert_eq!(bo_task["properties"]["due_in_seconds"], json!(7200));
    assert_eq!(bo_task["properties"]["priority"], json!(2));
    assert!(
        ana_task["properties"]["due_at"].is_string(),
        "due_at is materialized so the task is sortable/overdue-able"
    );

    // ---- Answering ONE does not release the join ------------------------
    let ana_instance = ana_task["properties"]["flow_instance_id"]
        .as_str()
        .expect("task records its owning child instance")
        .to_string();
    let _ = resume_flow(
        &ana_instance,
        json!({"action": "approved", "completed_by": "ana", "task_path": ana_path}),
        &harness,
    )
    .await;
    harness.pump_real_time().await;

    let parent = harness.instance(&id);
    assert_eq!(
        parent.status,
        FlowStatus::Waiting,
        "one of two answers must not release the join"
    );

    // ---- Answering the LAST one releases it ------------------------------
    let bo_instance = bo_task["properties"]["flow_instance_id"]
        .as_str()
        .unwrap()
        .to_string();
    let _ = resume_flow(
        &bo_instance,
        json!({"action": "approved", "completed_by": "bo", "task_path": bo_path}),
        &harness,
    )
    .await;
    harness.pump_real_time().await;

    let parent = harness.instance(&id);
    assert_eq!(
        parent.status,
        FlowStatus::Completed,
        "the join releases once the last person answered; error: {:?}",
        parent.error
    );

    // ---- The join output correlates results with their items -------------
    let joined = &parent.variables["step_outputs"]["collect_reviews"];
    let branches = joined["branches"].as_array().expect("ordered branch array");
    assert_eq!(branches.len(), 2);
    assert_eq!(branches[0]["branch_id"], json!("review-0"));
    assert_eq!(branches[0]["status"], json!("completed"));
    assert_eq!(branches[0]["output"]["completed_by"], json!("ana"));
    assert_eq!(branches[1]["branch_id"], json!("review-1"));
    assert_eq!(branches[1]["output"]["completed_by"], json!("bo"));

    // ---- And the downstream step ran, exactly once -----------------------
    let invocations = harness.function_invocations.lock().unwrap();
    let publishes = invocations
        .iter()
        .filter(|(f, _)| f == "/lib/publish")
        .count();
    assert_eq!(publishes, 1, "publish runs once, after the join");
}

/// `all_success` must make the join AUTHORITATIVE: one branch failing has to
/// fail the container, rather than the flow continuing and leaving the
/// application to notice from child state.
#[tokio::test]
async fn fan_out_all_success_fails_the_join_when_a_branch_fails() {
    let harness = Harness::new();

    harness.seed_node(
        "functions",
        "/flows/process-item",
        json!({
            "properties": {
                "workflow_data": {
                    "nodes": [
                        { "id": "start", "step_type": "start", "next_node": "work" },
                        {
                            "id": "work",
                            "step_type": "function_step",
                            "properties": {
                                "function_ref": "/lib/process",
                                // Runtime format takes retry config as FLAT
                                // properties (and defaults to 3 retries);
                                // this test asserts a SINGLE failure reaches
                                // the join.
                                "max_retries": 0
                            },
                            "next_node": "end"
                        },
                        { "id": "end", "step_type": "end" }
                    ]
                }
            }
        }),
    );

    // First item succeeds, second fails.
    harness.script_function(
        "/lib/process",
        json!({"success": true, "result": {"ok": 1}}),
    );
    harness.script_function(
        "/lib/process",
        json!({"success": false, "error": "processing blew up"}),
    );
    harness.script_function(
        "/lib/after",
        json!({"success": true, "result": {"ran": true}}),
    );

    let flow = json!({
        "nodes": [
            { "id": "start", "step_type": "start", "next_node": "fan" },
            {
                "id": "fan",
                "step_type": "parallel",
                "properties": {
                    "for_each": "${input.rows}",
                    "branch": {
                        "flow_path": "/flows/process-item",
                        "input_mapping": { "row": "${item}" }
                    },
                    "merge_strategy": "all_success"
                },
                "next_node": "after"
            },
            {
                "id": "after",
                "step_type": "function_step",
                "properties": { "function_ref": "/lib/after" },
                "next_node": "end"
            },
            { "id": "end", "step_type": "end" }
        ]
    });

    let id = run_to_quiescence(&harness, flow, json!({"rows": ["a", "b"]})).await;

    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Failed,
        "all_success must fail the container when a branch failed"
    );
    assert!(
        harness
            .function_invocations
            .lock()
            .unwrap()
            .iter()
            .all(|(f, _)| f != "/lib/after"),
        "the step after a failed join must not run"
    );
}

/// An empty collection is a no-op fan-out that flows straight through — not a
/// wait that never resolves.
#[tokio::test]
async fn fan_out_over_empty_collection_continues_immediately() {
    let harness = Harness::new();
    harness.script_function(
        "/lib/plan",
        json!({"success": true, "result": {"items": []}}),
    );
    harness.script_function(
        "/lib/after",
        json!({"success": true, "result": {"ran": true}}),
    );

    let flow = json!({
        "nodes": [
            { "id": "start", "step_type": "start", "next_node": "plan" },
            {
                "id": "plan",
                "step_type": "function_step",
                "properties": { "function_ref": "/lib/plan" },
                "next_node": "fan"
            },
            {
                "id": "fan",
                "step_type": "parallel",
                "properties": {
                    "for_each": "${steps.plan.items}",
                    "branch": { "flow_path": "/flows/never-used" }
                },
                "next_node": "after"
            },
            {
                "id": "after",
                "step_type": "function_step",
                "properties": { "function_ref": "/lib/after" },
                "next_node": "end"
            },
            { "id": "end", "step_type": "end" }
        ]
    });

    let id = run_to_quiescence(&harness, flow, json!({})).await;
    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "error: {:?}",
        instance.error
    );
    assert!(harness
        .function_invocations
        .lock()
        .unwrap()
        .iter()
        .any(|(f, _)| f == "/lib/after"));
}

// ===========================================================================
// 21. Templated human-task deadline drives the REAL timeout
// ===========================================================================

/// The point of the deadline fix: a `${...}` deadline must not merely be
/// stored, it must actually arm the wait so `timeout_edge` fires. Before, the
/// property was dropped and the flow waited forever.
#[tokio::test]
async fn templated_due_in_seconds_arms_the_wait_and_times_out() {
    let harness = Harness::new();
    harness.script_function(
        "/lib/escalate",
        json!({"success": true, "result": {"escalated": true}}),
    );

    let flow = json!({
        "nodes": [
            { "id": "start", "step_type": "start", "next_node": "approve" },
            {
                "id": "approve",
                "step_type": "human_task",
                "properties": {
                    "task_type": "approval",
                    "title": "Approve",
                    "assignee": "/users/manager",
                    // Deadline comes from the run's input, not the flow
                    "due_in_seconds": "${input.sla_seconds}",
                    "timeout_edge": "escalate"
                },
                "next_node": "end"
            },
            {
                "id": "escalate",
                "step_type": "function_step",
                "properties": { "function_ref": "/lib/escalate" },
                "next_node": "end"
            },
            { "id": "end", "step_type": "end" }
        ]
    });

    // Real-time pump: a future deadline must stay pending, so the armed
    // state is observable before it fires.
    let id = harness.seed_instance(make_instance(flow, json!({"sla_seconds": 3600})));
    let _ = execute_flow(&id, &harness).await;
    harness.pump_real_time().await;

    // The wait carries a real deadline derived from the template
    let instance = harness.instance(&id);
    assert_eq!(instance.status, FlowStatus::Waiting);
    let wait = instance.wait_info.as_ref().unwrap();
    assert_eq!(wait.wait_type, WaitType::HumanTask);
    assert!(
        wait.timeout_at.is_some(),
        "a templated deadline must arm the wait, not be dropped"
    );

    // Let the deadline pass, then run the timeout check the scheduler would
    {
        let mut map = harness.instances.lock().unwrap();
        let inst = map.get_mut(&id).unwrap();
        inst.wait_info.as_mut().unwrap().timeout_at =
            Some(chrono::Utc::now() - chrono::Duration::seconds(1));
    }
    let _ = check_flow_timeout(&id, &harness).await;
    harness.pump_real_time().await;

    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "timeout_edge should have routed to escalation; error: {:?}",
        instance.error
    );
    assert!(
        harness
            .function_invocations
            .lock()
            .unwrap()
            .iter()
            .any(|(f, _)| f == "/lib/escalate"),
        "the escalation step must have run"
    );

    // And the task itself was marked expired
    let (_, task) = harness
        .find_node_by_prefix("raisin:access_control", "/users/manager/inbox/")
        .expect("task node exists");
    assert_eq!(task["properties"]["status"], json!("expired"));
}

// ===========================================================================
// 22. Designer-format wait / sub_flow / decision run end to end
// ===========================================================================

/// These three step types used to be runtime-format only, so needing one
/// forced the whole flow out of the designer format. Prove a single
/// designer-format flow using all three actually runs.
///
/// Note the shape of the decision: designer siblings chain in array order, so
/// a free-standing decision is a **guard** that skips forward over steps.
/// Mutually exclusive branches are what an `or` container is for.
#[tokio::test]
async fn designer_wait_sub_flow_and_decision_run_end_to_end() {
    let harness = Harness::new();

    harness.seed_node(
        "functions",
        "/flows/settle",
        json!({
            "properties": {
                "workflow_data": {
                    "nodes": [
                        { "id": "start", "step_type": "start", "next_node": "work" },
                        {
                            "id": "work",
                            "step_type": "function_step",
                            "properties": { "function_ref": "/lib/settle" },
                            "next_node": "end"
                        },
                        { "id": "end", "step_type": "end" }
                    ]
                }
            }
        }),
    );
    harness.script_function(
        "/lib/settle",
        json!({"success": true, "result": {"settled": true}}),
    );
    harness.script_function(
        "/lib/discount",
        json!({"success": true, "result": {"discounted": true}}),
    );
    harness.script_function(
        "/lib/invoice",
        json!({"success": true, "result": {"invoiced": true}}),
    );

    // Designer format: no start/end, no next_node, array order is the chain.
    let flow = json!({
        "version": 1,
        "error_strategy": "fail_fast",
        "nodes": [
            {
                "id": "cool_off",
                "node_type": "raisin:FlowStep",
                "properties": { "action": "Cool off", "step_type": "wait", "duration": "1s" }
            },
            {
                "id": "settle",
                "node_type": "raisin:FlowStep",
                "properties": {
                    "action": "Settle",
                    "step_type": "sub_flow",
                    "flow_ref": "/flows/settle",
                    "input_mapping": { "order_id": "${input.order_id}" }
                }
            },
            {
                "id": "big_order",
                "node_type": "raisin:FlowStep",
                "properties": {
                    "action": "Big order? skip the discount",
                    "step_type": "decision",
                    "condition": "input.amount > 500",
                    // Guard: on true, jump PAST the discount step.
                    "yes_branch": "invoice"
                }
            },
            {
                "id": "discount",
                "node_type": "raisin:FlowStep",
                "properties": { "action": "Apply discount", "function_ref": "/lib/discount" }
            },
            {
                "id": "invoice",
                "node_type": "raisin:FlowStep",
                "properties": { "action": "Invoice", "function_ref": "/lib/invoice" }
            }
        ]
    });

    // amount > 500 -> the guard skips the discount step
    let id = harness.seed_instance(make_instance(
        flow.clone(),
        json!({"order_id": "ORD-9", "amount": 900}),
    ));
    let _ = execute_flow(&id, &harness).await;
    // Virtual time: advances the 1s scheduled wake-up deterministically.
    harness.pump().await;

    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "error: {:?}",
        instance.error
    );

    let called: Vec<String> = harness
        .function_invocations
        .lock()
        .unwrap()
        .iter()
        .map(|(f, _)| f.clone())
        .collect();
    // The wait elapsed, the sub-flow ran through its own child instance...
    assert!(
        called.contains(&"/lib/settle".to_string()),
        "the sub-flow must have run; called: {:?}",
        called
    );
    // ...and the guard skipped the discount, landing on invoice
    assert!(
        !called.contains(&"/lib/discount".to_string()),
        "the guard should have skipped the discount; called: {:?}",
        called
    );
    assert!(
        called.contains(&"/lib/invoice".to_string()),
        "called: {:?}",
        called
    );
}

/// The same flow with a small amount: the UNNAMED `no_branch` falls through to
/// the next sibling, so the guarded step runs. Before the fix this lowered
/// with no `no_branch` at all and the decision handler rejected it at run
/// time for a missing property.
#[tokio::test]
async fn designer_decision_unnamed_arm_falls_through_to_next_sibling() {
    let harness = Harness::new();
    harness.script_function(
        "/lib/discount",
        json!({"success": true, "result": {"discounted": true}}),
    );
    harness.script_function(
        "/lib/invoice",
        json!({"success": true, "result": {"invoiced": true}}),
    );

    let flow = json!({
        "version": 1,
        "error_strategy": "fail_fast",
        "nodes": [
            {
                "id": "big_order",
                "node_type": "raisin:FlowStep",
                "properties": {
                    "action": "Big order? skip the discount",
                    "step_type": "decision",
                    "condition": "input.amount > 500",
                    "yes_branch": "invoice"
                }
            },
            {
                "id": "discount",
                "node_type": "raisin:FlowStep",
                "properties": { "action": "Apply discount", "function_ref": "/lib/discount" }
            },
            {
                "id": "invoice",
                "node_type": "raisin:FlowStep",
                "properties": { "action": "Invoice", "function_ref": "/lib/invoice" }
            }
        ]
    });

    let id = run_to_quiescence(&harness, flow, json!({"amount": 100})).await;

    let instance = harness.instance(&id);
    assert_eq!(
        instance.status,
        FlowStatus::Completed,
        "error: {:?}",
        instance.error
    );

    let called: Vec<String> = harness
        .function_invocations
        .lock()
        .unwrap()
        .iter()
        .map(|(f, _)| f.clone())
        .collect();
    assert_eq!(
        called,
        vec!["/lib/discount".to_string(), "/lib/invoice".to_string()],
        "the false arm falls through to the next sibling"
    );
}

// ===========================================================================
// 23. A timed-out CHILD must release its parent's join
// ===========================================================================

/// Regression: `fail_timed_out` transitioned a child to Failed but never
/// notified the parent, so a parent parked on a join waited FOREVER the
/// moment one child's wait expired - the child was Failed, the parent never
/// learned, and no error surfaced anywhere. A fan-out with per-item deadlines
/// has one chance to hit this per item.
#[tokio::test]
async fn timed_out_child_notifies_parent_and_releases_the_join() {
    let harness = Harness::new();

    // Per-item flow that parks on a human task with a deadline nobody meets.
    harness.seed_node(
        "functions",
        "/flows/ask-owner",
        json!({
            "properties": {
                "workflow_data": {
                    "nodes": [
                        { "id": "start", "step_type": "start", "next_node": "ask" },
                        {
                            "id": "ask",
                            "step_type": "human_task",
                            "properties": {
                                "task_type": "approval",
                                "title": "Approve {{ input.item }}",
                                "assignee": "/users/owner",
                                "due_in_seconds": 1
                            },
                            "next_node": "end"
                        },
                        { "id": "end", "step_type": "end" }
                    ]
                }
            }
        }),
    );

    let flow = json!({
        "nodes": [
            { "id": "start", "step_type": "start", "next_node": "fan" },
            {
                "id": "fan",
                "step_type": "parallel",
                "properties": {
                    "for_each": "${input.items}",
                    "branch": {
                        "flow_path": "/flows/ask-owner",
                        "input_mapping": { "item": "${item}" }
                    },
                    "merge_strategy": "merge_all"
                },
                "next_node": "end"
            },
            { "id": "end", "step_type": "end" }
        ]
    });

    // The virtual-time pump forces the children's deadlines to fire.
    let id = run_to_quiescence(&harness, flow, json!({"items": ["a", "b"]})).await;

    // Both children timed out...
    let statuses: Vec<FlowStatus> = {
        let map = harness.instances.lock().unwrap();
        map.values()
            .filter(|i| i.parent_instance_ref.is_some())
            .map(|i| i.status)
            .collect()
    };
    assert_eq!(statuses.len(), 2, "two children were forked");
    assert!(
        statuses.iter().all(|s| *s == FlowStatus::Failed),
        "both children timed out: {:?}",
        statuses
    );

    // ...and the parent was RELEASED rather than stranded in Waiting.
    let parent = harness.instance(&id);
    assert_ne!(
        parent.status,
        FlowStatus::Waiting,
        "a timed-out child must notify its parent; the join must not hang"
    );

    // merge_all continues with the failed branch results recorded.
    assert_eq!(
        parent.status,
        FlowStatus::Completed,
        "error: {:?}",
        parent.error
    );
    let joined = &parent.variables["step_outputs"]["fan"];
    let branches = joined["branches"].as_array().expect("ordered branch array");
    assert_eq!(branches.len(), 2);
    assert!(
        branches.iter().all(|b| b["status"] == json!("failed")),
        "both branches recorded as failed: {}",
        joined
    );
    assert!(
        branches[0]["error"]
            .as_str()
            .unwrap_or_default()
            .contains("timed out"),
        "the timeout reason reaches the parent: {}",
        branches[0]
    );
}

/// The deadline fix, exercised through a DESIGNER-format flow.
///
/// The earlier coverage used runtime format, which hid a real gap: the
/// designer types declared `due_in_seconds` as `i64`, so a `${...}` there
/// failed to DESERIALIZE and took the whole flow definition down with it.
/// Designer format is the canonical authoring format, so that meant per-run
/// deadlines were unavailable exactly where flows are actually written.
#[tokio::test]
async fn designer_human_task_templated_deadline_arms_the_wait() {
    let harness = Harness::new();
    harness.script_function(
        "/lib/escalate",
        json!({"success": true, "result": {"escalated": true}}),
    );

    let flow = json!({
        "version": 1,
        "error_strategy": "fail_fast",
        "nodes": [
            {
                "id": "approve",
                "node_type": "raisin:FlowStep",
                "properties": {
                    "action": "Approve {{ input.order_id }}",
                    "step_type": "human_task",
                    "task_type": "approval",
                    "assignee": "/users/manager",
                    // Both from data, not authored into the flow
                    "due_in_seconds": "${input.sla_seconds}",
                    "priority": "${input.tier}",
                    "timeout_edge": "escalate"
                }
            },
            {
                "id": "escalate",
                "node_type": "raisin:FlowStep",
                "properties": { "action": "Escalate", "function_ref": "/lib/escalate" }
            }
        ]
    });

    let id = harness.seed_instance(make_instance(
        flow,
        json!({"order_id": "ORD-3", "sla_seconds": 3600, "tier": 5}),
    ));
    let _ = execute_flow(&id, &harness).await;
    harness.pump_real_time().await;

    // The task carries the RESOLVED deadline and priority...
    let (_, task) = harness
        .find_node_by_prefix("raisin:access_control", "/users/manager/inbox/")
        .expect("inbox task created");
    assert_eq!(task["properties"]["title"], json!("Approve ORD-3"));
    assert_eq!(task["properties"]["due_in_seconds"], json!(3600));
    assert_eq!(task["properties"]["priority"], json!(5));
    assert!(task["properties"]["due_at"].is_string());

    // ...and the wait is actually armed with it, so timeout_edge can fire.
    let instance = harness.instance(&id);
    assert_eq!(instance.status, FlowStatus::Waiting);
    let wait = instance.wait_info.as_ref().unwrap();
    assert_eq!(wait.wait_type, WaitType::HumanTask);
    assert!(
        wait.timeout_at.is_some(),
        "a templated deadline must arm the wait"
    );

    // Expire it and confirm the escalation branch runs.
    {
        let mut map = harness.instances.lock().unwrap();
        let inst = map.get_mut(&id).unwrap();
        inst.wait_info.as_mut().unwrap().timeout_at =
            Some(chrono::Utc::now() - chrono::Duration::seconds(1));
    }
    let _ = check_flow_timeout(&id, &harness).await;
    harness.pump_real_time().await;

    assert!(
        harness
            .function_invocations
            .lock()
            .unwrap()
            .iter()
            .any(|(f, _)| f == "/lib/escalate"),
        "timeout_edge must route to the escalation step"
    );
}
