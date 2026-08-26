// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Main flow execution loop with hybrid batching and OCC
//!
//! Implements the core execution loop that:
//! - Loads flow instances with version for optimistic concurrency
//! - Executes steps continuously until an async boundary
//! - Handles idempotency (skips already completed flows)
//! - Retries on version conflicts
//! - Manages compensation stack for rollback

use crate::types::{
    FlowCallbacks, FlowDefinition, FlowError, FlowExecutionEvent, FlowResult, FlowStatus,
    StepResult,
};
use chrono::Utc;
use serde_json::Value;
use std::time::Instant;
use tracing::info;

use super::helpers::{
    bump_visit_count, extract_token_usage, project_output_key, record_step_output,
};
use super::isolated_branch::execute_step;
use super::result_handlers::{
    fail_flow_terminally, handle_complete_result, handle_error_result, handle_wait_result,
};

/// Main flow execution function with hybrid batching.
///
/// This function implements the core execution loop:
/// 1. Loads the flow instance with version for OCC
/// 2. Checks idempotency (skips if already completed/cancelled)
/// 3. Executes steps in a loop until:
///    - An async boundary is reached (Wait result)
///    - Flow completes
///    - An error occurs
/// 4. Saves state with version check (OCC)
/// 5. Retries on version conflict (with limit)
///
/// # Arguments
///
/// * `instance_id` - The flow instance ID to execute
/// * `callbacks` - Callbacks for node operations, AI calls, etc.
///
/// # Returns
///
/// Returns `Ok(())` if execution succeeds or pauses at async boundary.
/// Returns `Err` if execution fails after retries.
pub async fn execute_flow(instance_id: &str, callbacks: &dyn FlowCallbacks) -> FlowResult<()> {
    execute_flow_with_retry(instance_id, callbacks, 0).await
}

/// Internal flow execution with version conflict retry tracking
pub(super) async fn execute_flow_with_retry(
    instance_id: &str,
    callbacks: &dyn FlowCallbacks,
    version_conflict_retries: u32,
) -> FlowResult<()> {
    info!("Starting flow execution for instance: {}", instance_id);

    // 1. Load Instance with version for OCC
    let mut instance = callbacks
        .load_instance(&format!("/flows/instances/{}", instance_id))
        .await?;

    let expected_version = instance.version;

    // 2. Skip if already completed (idempotency check)
    if instance.is_terminated() {
        info!(
            "Flow {} already in terminal state: {:?}, skipping",
            instance_id, instance.status
        );
        return Ok(());
    }

    // Mark as running if pending
    if instance.status == FlowStatus::Pending {
        instance.status = FlowStatus::Running;
        instance.started_at = Utc::now();
    }

    // Parse flow definition from snapshot
    // Supports both runtime format (step_type) and designer format (node_type)
    let flow_def = FlowDefinition::from_workflow_data(instance.flow_definition_snapshot.clone())?;

    // VALIDATE BEFORE THE FIRST STEP, not on the way into a broken edge.
    //
    // Only on a fresh run: a resumed instance is already mid-flight and has
    // side effects behind it, so refusing it here would strand work that a
    // human task was about to release. A definition is immutable per run
    // (`flow_definition_snapshot`), so if it was valid at the start it is valid
    // now.
    if instance.metrics.step_count == 0 {
        if let Err(e) = flow_def.validate() {
            tracing::error!(instance_id, error = %e, "Flow definition is invalid, refusing to start");
            instance.status = FlowStatus::Failed;
            instance.error = Some(e.to_string());
            instance.completed_at = Some(Utc::now());
            callbacks.save_instance(&instance).await?;
            return Err(e);
        }
    }

    // Track flow start time for duration calculation
    let flow_start = Instant::now();

    // Opt-in per flow; 0 (the default) means no history is kept.
    let checkpoint_limit =
        crate::types::FlowInstance::checkpoint_limit(&instance.flow_definition_snapshot);

    // Guard against unbounded SameStep re-execution
    let mut same_step_count: u32 = 0;
    const MAX_SAME_STEP_ITERATIONS: u32 = 100;

    // Guard against an unbounded CYCLE.
    //
    // The cursor below is assigned whatever `next_node_id` a handler returns,
    // with no visited set — which is deliberate, and is what lets a graph loop
    // back (`decision.no_branch` → an earlier node) the way a draft/critique/
    // revise cycle needs to. `same_step_count` does NOT cover it: that only
    // catches a handler re-entering ITSELF, and it resets on every transition,
    // so `A → B → A` slips past it forever.
    //
    // Unbounded matters more here than it looks. Persistence happens only at
    // async boundaries, so a cycle of purely synchronous steps never commits:
    // it is a hot loop inside one job execution, burning a worker with no
    // durable state and nothing to observe. This bounds the number of
    // TRANSITIONS in a single execution batch, which is exactly the quantity
    // that is unbounded — it is not a limit on how long a flow may live, since
    // any step that waits resets the batch.
    //
    // Generous on purpose: a legitimate fan-out or a long for_each does many
    // transitions, and this is a backstop against a mis-authored graph, not a
    // budget an author should ever feel. A bounded cycle is expressed by the
    // author with `visits.<step_id>` (below), not by this number.
    let mut steps_this_execution: u32 = 0;
    const MAX_STEPS_PER_EXECUTION: u32 = 10_000;

    // 3. Main execution loop - continue until async boundary or completion
    loop {
        let current_step = flow_def
            .find_node(&instance.current_node_id)
            .ok_or_else(|| {
                crate::types::FlowError::StepNotFound(instance.current_node_id.clone())
            })?;

        info!(
            "Executing step {} of type {:?}",
            current_step.id, current_step.step_type
        );

        steps_this_execution += 1;
        if steps_this_execution > MAX_STEPS_PER_EXECUTION {
            tracing::error!(
                step_id = %current_step.id,
                transitions = steps_this_execution,
                "Step budget exceeded in one execution - probable unbounded cycle"
            );
            // TERMINAL, deliberately skipping retry / error_edge /
            // continue_on_fail. Every one of those is wrong for a runaway: a
            // retry re-enters the same non-terminating cycle (three more times,
            // by default), and an error edge lets the flow continue from a state
            // the engine has just said it cannot bound. It still fails the flow
            // PROPERLY — error recorded, compensation run, parent released —
            // rather than returning a raw Err that would strand the instance in
            // `Running`.
            let step_id = current_step.id.clone();
            fail_flow_terminally(
                instance_id,
                &mut instance,
                &step_id,
                crate::types::FlowError::MaxIterationsExceeded {
                    limit: MAX_STEPS_PER_EXECUTION,
                },
                &flow_start,
                callbacks,
            )
            .await?;
            return Ok(());
        }

        // COUNT THE VISIT, before the step runs, so a step can read its own
        // attempt number as `visits.<its own id>` (1 on the first pass).
        //
        // This is the primitive a bounded cycle is built from. The engine will
        // happily run `critic --fail--> draft` forever; what stops it is the
        // AUTHOR writing `visits.draft < 3` on that edge, and until now there
        // was nothing to write. `retry_count` is not it — that is flow-scoped,
        // resets on any successful transition, and is never exposed to
        // expressions at all.
        //
        // Lives under `__visits` in the variable bag so it persists and resumes
        // with everything else; `__`-prefixed keys are already protected from
        // being clobbered by a step output. `FlowContext::to_json` republishes
        // it as the `visits` namespace.
        let visit_count = bump_visit_count(&mut instance, &current_step.id);
        tracing::debug!(step_id = %current_step.id, visit = visit_count, "Step visit");

        // Record where we are, if this flow asked for a replayable history.
        // Taken BEFORE the step runs, so a checkpoint is a position you can
        // start from rather than one you have already moved past — the useful
        // question is "run it again from here", and answering it needs the state
        // the step saw, not the state it left.
        instance.record_checkpoint(checkpoint_limit, visit_count);

        // Emit StepStarted event
        let step_start = Instant::now();
        let step_name = current_step
            .properties
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let _ = callbacks
            .emit_event(
                instance_id,
                FlowExecutionEvent::step_started(
                    &current_step.id,
                    step_name,
                    format!("{:?}", current_step.step_type),
                ),
            )
            .await;

        // Execute step based on type
        let result = execute_step(current_step, &mut instance, &flow_def, callbacks).await;

        // Calculate step duration
        let step_duration_ms = step_start.elapsed().as_millis() as u64;

        // Handle step execution errors with event emission
        let result = match result {
            Ok(r) => r,
            Err(e) => {
                // Emit StepFailed event
                let _ = callbacks
                    .emit_event(
                        instance_id,
                        FlowExecutionEvent::step_failed(
                            &current_step.id,
                            e.to_string(),
                            step_duration_ms,
                        ),
                    )
                    .await;

                // Optimistic-concurrency conflicts are INFRASTRUCTURAL: the
                // job system must see them and redeliver, and the instance
                // must be left alone for the winning writer. Everything else
                // is a step failure.
                if e.is_infrastructural() {
                    return Err(e);
                }

                // A handler that returns `Err` and one that returns
                // `StepResult::Error` describe the SAME event, so they must
                // take the same path: retry, error edge, continue-on-fail,
                // else fail the flow (which records the error, runs
                // compensation, and notifies a waiting parent). Returning
                // the error raw here used to leave the instance stuck in
                // `Running` forever - not failed, not waiting, no error
                // recorded, and a parent parked on a join never released.
                // `parallel`'s `all_success` / `first_success` merges report
                // failure exactly this way.
                let should_return = handle_error_result(
                    instance_id,
                    &mut instance,
                    current_step,
                    &flow_def,
                    e,
                    expected_version,
                    step_duration_ms,
                    &flow_start,
                    callbacks,
                )
                .await?;
                if should_return {
                    return Ok(());
                }
                continue;
            }
        };

        // Increment step counter
        instance.metrics.step_count += 1;

        match result {
            // 4. Sync Success -> Move to next step immediately (Hot Path)
            StepResult::Continue {
                next_node_id,
                output,
            } => {
                info!(
                    "Step {} completed, continuing to {}",
                    current_step.id, next_node_id
                );

                // Emit StepCompleted event
                let _ = callbacks
                    .emit_event(
                        instance_id,
                        FlowExecutionEvent::step_completed(
                            &current_step.id,
                            output.clone(),
                            step_duration_ms,
                        ),
                    )
                    .await;

                // Record the output: latest under `steps.<id>`, one entry per
                // visit under `history.<id>`, and the legacy flat merge into the
                // shared variable bag — with THIS STEP'S PREVIOUS flat keys
                // retracted first, so a re-visited node cannot leave stale values
                // standing under names that read like current data.
                //
                // The `steps.<id>` write is UNCONDITIONAL, including a null
                // output. It used to be guarded by `if !output.is_null()`, which
                // meant a step that produced nothing left its PREVIOUS
                // execution's value in place — so the next reader got stale data
                // with no way to tell. In a loop that is a silent wrong-data bug,
                // not a missing-data one: a translation flow whose per-locale
                // agent call came back empty re-read the previous locale's output
                // and wrote GERMAN text into the French and Italian overlays,
                // reporting success for both.
                // Republish the primary value under the step's own
                // `output_key`, if the author named one, BEFORE recording — so
                // `steps.<id>`, `history.<id>` and the flat variable bag all
                // agree on the shape.
                let output = project_output_key(current_step, output);
                if let Value::Object(ref mut vars) = instance.variables {
                    record_step_output(vars, &current_step.id, &output);
                }

                // Roll model usage up onto the flow. `FlowMetrics` tracked
                // duration, steps, retries and AI iterations but not TOKENS —
                // the one number anyone asks about an agent workflow — and usage
                // was left as an opaque blob inside an agent step's output for a
                // caller to dig out. An AI container reports its own cumulative
                // total once, at the end, so a container's many turns are counted
                // exactly once here rather than per re-entry.
                if let Some((input, output_tokens)) = extract_token_usage(&output) {
                    instance.metrics.ai_call_count += 1;
                    instance.metrics.total_input_tokens =
                        instance.metrics.total_input_tokens.saturating_add(input);
                    instance.metrics.total_output_tokens = instance
                        .metrics
                        .total_output_tokens
                        .saturating_add(output_tokens);
                }

                instance.current_node_id = next_node_id;
                instance.retry_count = 0; // Step succeeded - reset retry budget
                same_step_count = 0; // Reset SameStep guard on step transition
                                     // OPTIMIZATION: Don't persist to DB yet if next step is also sync
                                     // Only persist at async boundaries
            }

            // 5. Async Boundary -> Persist and Exit
            StepResult::Wait { reason, metadata } => {
                handle_wait_result(
                    instance_id,
                    &mut instance,
                    &current_step.id,
                    &reason,
                    &metadata,
                    expected_version,
                    version_conflict_retries,
                    callbacks,
                )
                .await?;
                return Ok(());
            }

            // 6. Flow completed
            StepResult::Complete { output } => {
                handle_complete_result(
                    instance_id,
                    &mut instance,
                    &current_step.id,
                    output,
                    expected_version,
                    step_duration_ms,
                    &flow_start,
                    callbacks,
                )
                .await?;
                return Ok(());
            }

            // 7. Error handling with retry, error edges, and continue-on-fail
            StepResult::Error { error } => {
                let should_return = handle_error_result(
                    instance_id,
                    &mut instance,
                    current_step,
                    &flow_def,
                    error,
                    expected_version,
                    step_duration_ms,
                    &flow_start,
                    callbacks,
                )
                .await?;
                if should_return {
                    return Ok(());
                }
                // else continue the loop (error edge or continue_on_fail)
            }

            // 8. Re-execute same step (for internal loops like AI agent iterations)
            StepResult::SameStep { metadata } => {
                same_step_count += 1;
                if same_step_count > MAX_SAME_STEP_ITERATIONS {
                    tracing::error!(
                        step_id = %current_step.id,
                        iterations = same_step_count,
                        "SameStep loop guard exceeded, failing step"
                    );
                    // Through the normal error path. This used to `return Err`
                    // raw, which is the exact shape the error arm above was
                    // fixed for: MaxIterationsExceeded is not infrastructural,
                    // so nothing downstream turned it into a flow failure — the
                    // instance stayed `Running` forever with no error recorded,
                    // no compensation, and any parent parked on a join never
                    // released. An AI container hitting its iteration ceiling is
                    // a STEP failure, and the flow's own error handling
                    // (retry / error_edge / continue_on_fail) should get to
                    // decide what that means.
                    let should_return = handle_error_result(
                        instance_id,
                        &mut instance,
                        current_step,
                        &flow_def,
                        crate::types::FlowError::MaxIterationsExceeded {
                            limit: MAX_SAME_STEP_ITERATIONS,
                        },
                        expected_version,
                        step_duration_ms,
                        &flow_start,
                        callbacks,
                    )
                    .await?;
                    if should_return {
                        return Ok(());
                    }
                    same_step_count = 0; // error edge / continue-on-fail moved us on
                    continue;
                }
                tracing::debug!(
                    step_id = %current_step.id,
                    iteration = same_step_count,
                    ?metadata,
                    "Step requesting re-execution"
                );
                // Continue the loop - the step will be re-executed with updated state
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use serde_json::{json, Value};

    use crate::types::{FlowCallbacks, FlowExecutionEvent, FlowInstance, FlowResult, FlowStatus};

    // -----------------------------------------------------------------------
    // Mock FlowCallbacks
    // -----------------------------------------------------------------------

    struct MockLoopCallbacks {
        instance: Arc<Mutex<FlowInstance>>,
        events: Arc<Mutex<Vec<FlowExecutionEvent>>>,
        save_count: Arc<Mutex<u32>>,
    }

    impl MockLoopCallbacks {
        fn new(instance: FlowInstance) -> Self {
            Self {
                instance: Arc::new(Mutex::new(instance)),
                events: Arc::new(Mutex::new(Vec::new())),
                save_count: Arc::new(Mutex::new(0)),
            }
        }

        fn saved_instance(&self) -> FlowInstance {
            self.instance.lock().unwrap().clone()
        }

        fn emitted_events(&self) -> Vec<FlowExecutionEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl FlowCallbacks for MockLoopCallbacks {
        async fn load_instance(&self, _path: &str) -> FlowResult<FlowInstance> {
            Ok(self.instance.lock().unwrap().clone())
        }

        async fn save_instance(&self, instance: &FlowInstance) -> FlowResult<()> {
            *self.instance.lock().unwrap() = instance.clone();
            Ok(())
        }

        async fn save_instance_with_version(
            &self,
            instance: &FlowInstance,
            _expected_version: i32,
        ) -> FlowResult<()> {
            *self.instance.lock().unwrap() = instance.clone();
            *self.save_count.lock().unwrap() += 1;
            Ok(())
        }

        async fn create_node(
            &self,
            _node_type: &str,
            _path: &str,
            _properties: Value,
        ) -> FlowResult<Value> {
            Ok(json!({}))
        }

        async fn update_node(&self, _path: &str, _properties: Value) -> FlowResult<Value> {
            Ok(json!({}))
        }

        async fn get_node(&self, _path: &str) -> FlowResult<Option<Value>> {
            Ok(None)
        }

        async fn queue_job(&self, _job_type: &str, _payload: Value) -> FlowResult<String> {
            Ok("mock-job-id".to_string())
        }

        async fn call_ai(
            &self,
            _agent_workspace: &str,
            _agent_ref: &str,
            _messages: Vec<Value>,
            _response_format: Option<Value>,
        ) -> FlowResult<Value> {
            Ok(json!({ "content": "AI response" }))
        }

        async fn execute_function(&self, _function_ref: &str, _input: Value) -> FlowResult<Value> {
            Ok(json!({ "status": "ok" }))
        }

        async fn emit_event(
            &self,
            _instance_id: &str,
            event: FlowExecutionEvent,
        ) -> FlowResult<()> {
            self.events.lock().unwrap().push(event);
            Ok(())
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Minimal flow: Start -> End (no intermediate steps)
    fn start_end_flow() -> Value {
        json!({
            "nodes": [
                { "id": "start", "step_type": "start", "next_node": "end" },
                { "id": "end", "step_type": "end" }
            ]
        })
    }

    /// Flow with a function step: Start -> FunctionStep -> End
    /// FunctionStepHandler queues a job and returns Wait, so the flow
    /// pauses at the function step (async boundary).
    fn function_step_flow(func_ref: &str) -> Value {
        json!({
            "nodes": [
                { "id": "start", "step_type": "start", "next_node": "step1" },
                {
                    "id": "step1",
                    "step_type": "function_step",
                    "properties": { "function_ref": func_ref, "action": "test-action" },
                    "next_node": "end"
                },
                { "id": "end", "step_type": "end" }
            ]
        })
    }

    /// Flow that simulates resuming a function step with a pre-seeded result.
    /// The instance starts at `step1` (already past start) and has
    /// `__function_result` in variables so the FunctionStepHandler returns Continue.
    fn resumed_function_flow(func_ref: &str) -> Value {
        json!({
            "nodes": [
                { "id": "start", "step_type": "start", "next_node": "step1" },
                {
                    "id": "step1",
                    "step_type": "function_step",
                    "properties": { "function_ref": func_ref, "action": "test-action" },
                    "next_node": "end"
                },
                { "id": "end", "step_type": "end" }
            ]
        })
    }

    fn make_instance(flow_def_snapshot: Value) -> FlowInstance {
        FlowInstance {
            id: "test-instance-1".to_string(),
            version: 1,
            flow_ref: "/flows/test".to_string(),
            flow_version: 1,
            flow_definition_snapshot: flow_def_snapshot,
            status: FlowStatus::Pending,
            current_node_id: "start".to_string(),
            wait_info: None,
            variables: json!({}),
            input: json!({}),
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

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    /// A TWO-NODE CYCLE, bounded by the author with `visits`.
    ///
    /// `spin` loops back through `hop` until it has been entered `limit` times.
    /// Deliberately two nodes rather than a self-loop: the pre-existing
    /// `same_step_count` guard only catches a handler re-entering ITSELF and
    /// resets on every transition, so a two-node cycle is precisely the shape it
    /// cannot see.
    fn bounded_cycle_flow(limit: u32) -> Value {
        json!({
            "nodes": [
                { "id": "start", "step_type": "start", "next_node": "spin" },
                {
                    "id": "spin",
                    "step_type": "decision",
                    "properties": {
                        "condition": format!("visits.spin < {}", limit),
                        "yes_branch": "hop",
                        "no_branch": "end",
                        "max_retries": 0
                    }
                },
                {
                    "id": "hop",
                    "step_type": "decision",
                    "properties": {
                        "condition": "true",
                        "yes_branch": "spin",
                        "no_branch": "end",
                        "max_retries": 0
                    }
                },
                { "id": "end", "step_type": "end" }
            ]
        })
    }

    /// The same cycle with NO guard - it never terminates on its own.
    fn unbounded_cycle_flow() -> Value {
        json!({
            "nodes": [
                { "id": "start", "step_type": "start", "next_node": "spin" },
                {
                    "id": "spin",
                    "step_type": "decision",
                    "properties": { "condition": "true", "yes_branch": "hop", "no_branch": "end" }
                },
                {
                    "id": "hop",
                    "step_type": "decision",
                    "properties": { "condition": "true", "yes_branch": "spin", "no_branch": "end" }
                },
                { "id": "end", "step_type": "end" }
            ]
        })
    }

    /// Start -> End completes immediately with no async boundaries.
    #[tokio::test]
    async fn test_execute_start_to_end_completes() {
        let flow_def = start_end_flow();
        let instance = make_instance(flow_def);
        let callbacks = MockLoopCallbacks::new(instance);

        let result = super::execute_flow("test-instance-1", &callbacks).await;
        assert!(result.is_ok(), "Flow should complete successfully");

        let saved = callbacks.saved_instance();
        assert_eq!(saved.status, FlowStatus::Completed);
        assert!(saved.output.is_some());
        assert!(saved.completed_at.is_some());
        assert!(saved.metrics.step_count >= 2); // start + end
    }

    /// Function step causes the flow to pause at an async boundary (Wait state).
    #[tokio::test]
    async fn test_execute_function_step_pauses_at_wait() {
        let flow_def = function_step_flow("/functions/greet");
        let instance = make_instance(flow_def);
        let callbacks = MockLoopCallbacks::new(instance);

        let result = super::execute_flow("test-instance-1", &callbacks).await;
        assert!(result.is_ok(), "Flow should pause without error");

        let saved = callbacks.saved_instance();
        assert_eq!(saved.status, FlowStatus::Waiting);
        assert!(saved.wait_info.is_some());
        let wait_info = saved.wait_info.as_ref().unwrap();
        assert_eq!(wait_info.wait_type, crate::types::WaitType::FunctionCall);
    }

    /// Resuming a function step with __function_result completes the flow.
    #[tokio::test]
    async fn test_execute_resumed_function_step_completes() {
        let flow_def = resumed_function_flow("/functions/greet");
        let mut instance = make_instance(flow_def);
        // Simulate resume: instance is already at step1, status Running
        instance.current_node_id = "step1".to_string();
        instance.status = FlowStatus::Running;
        // Pre-seed the function result so FunctionStepHandler returns Continue
        instance.variables = json!({
            "__function_result": {
                "success": true,
                "result": { "greeting": "hello" }
            }
        });

        let callbacks = MockLoopCallbacks::new(instance);

        let result = super::execute_flow("test-instance-1", &callbacks).await;
        assert!(result.is_ok(), "Flow should complete after resume");

        let saved = callbacks.saved_instance();
        assert_eq!(saved.status, FlowStatus::Completed);
        assert!(saved.output.is_some());

        // The greeting output should be in the final variables
        let output = saved.output.as_ref().unwrap();
        assert_eq!(output.get("greeting"), Some(&json!("hello")));
    }

    /// Already-completed flows are skipped (idempotency).
    #[tokio::test]
    async fn test_execute_already_completed_is_noop() {
        let flow_def = start_end_flow();
        let mut instance = make_instance(flow_def);
        instance.status = FlowStatus::Completed;
        instance.output = Some(json!({ "done": true }));

        let callbacks = MockLoopCallbacks::new(instance);

        let result = super::execute_flow("test-instance-1", &callbacks).await;
        assert!(result.is_ok());

        // save should NOT have been called since flow was already completed
        assert_eq!(*callbacks.save_count.lock().unwrap(), 0);
    }

    /// Events are emitted for step lifecycle (Start -> End flow).
    #[tokio::test]
    async fn test_execute_emits_lifecycle_events() {
        let flow_def = start_end_flow();
        let instance = make_instance(flow_def);
        let callbacks = MockLoopCallbacks::new(instance);

        let result = super::execute_flow("test-instance-1", &callbacks).await;
        assert!(result.is_ok());

        let events = callbacks.emitted_events();
        // Start -> End: step_started(start), step_completed(start),
        //   step_started(end), step_completed(end), flow_completed
        assert!(
            events.len() >= 4,
            "Expected at least 4 events, got {}",
            events.len()
        );
    }

    /// Pending instance transitions to Running before step execution.
    #[tokio::test]
    async fn test_execute_pending_becomes_running() {
        let flow_def = function_step_flow("/functions/greet");
        let instance = make_instance(flow_def);
        assert_eq!(instance.status, FlowStatus::Pending);

        let callbacks = MockLoopCallbacks::new(instance);

        let _ = super::execute_flow("test-instance-1", &callbacks).await;

        // After execution (paused at Wait), status should be Waiting
        // but it went through Running first
        let saved = callbacks.saved_instance();
        assert_eq!(saved.status, FlowStatus::Waiting);
    }

    // -----------------------------------------------------------------------
    // Cyclic graphs: visit counting, and the runaway backstop
    // -----------------------------------------------------------------------

    /// A backward edge runs, and `visits.<step_id>` is what lets the AUTHOR
    /// stop it. This is the draft/critique/revise shape the graph editor needs.
    #[tokio::test]
    async fn test_bounded_cycle_terminates_on_visit_count() {
        let instance = make_instance(bounded_cycle_flow(3));
        let callbacks = MockLoopCallbacks::new(instance);

        let result = super::execute_flow("test-instance-1", &callbacks).await;
        assert!(result.is_ok(), "bounded cycle should complete: {result:?}");

        let saved = callbacks.saved_instance();
        assert_eq!(
            saved.status,
            FlowStatus::Completed,
            "cycle should exit through the guard, not run away"
        );

        // `spin` is entered 3 times: visits 1 and 2 loop, visit 3 fails
        // `visits.spin < 3` and falls through to end.
        let visits = saved
            .variables
            .get("__visits")
            .expect("visit counts should be recorded");
        assert_eq!(visits.get("spin").and_then(|v| v.as_u64()), Some(3));
        assert_eq!(visits.get("hop").and_then(|v| v.as_u64()), Some(2));
    }

    /// The counter is readable by the step being counted, from its first visit.
    /// Without this an author cannot write "attempt 2 of 3" at all.
    #[tokio::test]
    async fn test_visit_count_is_one_on_first_entry() {
        // `visits.spin < 1` is false on the very first entry, so the flow must
        // take the NO branch immediately - proving the count is 1, not 0.
        let instance = make_instance(bounded_cycle_flow(1));
        let callbacks = MockLoopCallbacks::new(instance);

        let result = super::execute_flow("test-instance-1", &callbacks).await;
        assert!(result.is_ok(), "flow should complete: {result:?}");

        let saved = callbacks.saved_instance();
        assert_eq!(saved.status, FlowStatus::Completed);
        let visits = saved.variables.get("__visits").expect("visit counts");
        assert_eq!(visits.get("spin").and_then(|v| v.as_u64()), Some(1));
        assert!(
            visits.get("hop").is_none(),
            "the loop body must never run when the guard is false on entry"
        );
    }

    /// An UNGUARDED cycle must terminate as a failed flow rather than spinning
    /// forever. Persistence only happens at async boundaries, so before this
    /// backstop existed a synchronous cycle was an unbounded hot loop inside one
    /// job execution - no durable state, nothing to observe, a burned worker.
    #[tokio::test]
    async fn test_unbounded_cycle_fails_instead_of_hanging() {
        let instance = make_instance(unbounded_cycle_flow());
        let callbacks = MockLoopCallbacks::new(instance);

        let result = super::execute_flow("test-instance-1", &callbacks).await;
        assert!(
            matches!(
                result,
                Err(crate::types::FlowError::MaxIterationsExceeded { .. })
            ),
            "runaway cycle should surface as MaxIterationsExceeded, got {result:?}"
        );

        // Failed PROPERLY: an earlier version of this guard returned a raw Err,
        // which left the instance in `Running` forever with no error recorded
        // and any parent parked on a join never released.
        let saved = callbacks.saved_instance();
        assert_eq!(saved.status, FlowStatus::Failed);
        assert!(saved.error.is_some(), "the failure must be recorded");
        assert!(saved.completed_at.is_some());
    }

    /// Visit counts must survive an async boundary, or a cycle that parks on a
    /// human task resets its own guard on every resume and never terminates -
    /// which is exactly the shape of a review/revise loop.
    #[tokio::test]
    async fn test_visit_counts_persist_across_a_wait() {
        let instance = make_instance(function_step_flow("/lib/test/fn"));
        let callbacks = MockLoopCallbacks::new(instance);

        let result = super::execute_flow("test-instance-1", &callbacks).await;
        assert!(result.is_ok(), "flow should pause cleanly: {result:?}");

        let saved = callbacks.saved_instance();
        assert_eq!(saved.status, FlowStatus::Waiting, "should be parked");

        let visits = saved
            .variables
            .get("__visits")
            .expect("visit counts must be persisted with the instance, not held in memory");
        assert_eq!(visits.get("start").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(visits.get("step1").and_then(|v| v.as_u64()), Some(1));
    }

    // -----------------------------------------------------------------------
    // Checkpoints: a replayable history, opt-in
    // -----------------------------------------------------------------------

    fn checkpointing_flow(limit: u32) -> Value {
        json!({
            "metadata": { "custom": { "checkpoints": limit } },
            "nodes": [
                { "id": "start", "step_type": "start", "next_node": "a" },
                { "id": "a", "step_type": "decision",
                  "properties": { "condition": "true", "yes_branch": "end", "no_branch": "end" } },
                { "id": "end", "step_type": "end" }
            ]
        })
    }

    /// OFF unless the flow asks. A snapshot copies the whole variable bag, and a
    /// flow moving large payloads should not pay that on every step just to
    /// exist.
    #[tokio::test]
    async fn test_checkpoints_are_off_by_default() {
        let instance = make_instance(bounded_cycle_flow(3));
        let callbacks = MockLoopCallbacks::new(instance);

        super::execute_flow("test-instance-1", &callbacks)
            .await
            .unwrap();

        assert!(
            callbacks.saved_instance().checkpoints.is_empty(),
            "a flow that did not ask for checkpoints must not accumulate them"
        );
    }

    /// Each step records where it was ABOUT to run, so a checkpoint is a
    /// position you can start from rather than one already moved past.
    #[tokio::test]
    async fn test_checkpoints_record_each_position() {
        let instance = make_instance(checkpointing_flow(10));
        let callbacks = MockLoopCallbacks::new(instance);

        super::execute_flow("test-instance-1", &callbacks)
            .await
            .unwrap();

        let saved = callbacks.saved_instance();
        let ids: Vec<&str> = saved
            .checkpoints
            .iter()
            .map(|c| c.node_id.as_str())
            .collect();
        assert_eq!(ids, vec!["start", "a", "end"]);
        assert_eq!(saved.checkpoints[0].seq, 0);
        assert_eq!(saved.checkpoints[0].visit, 1, "first visit of `start`");
    }

    /// Bounded, dropping the OLDEST - a debugger wants the recent past, and an
    /// unbounded history over a long loop would grow one persisted node forever.
    #[tokio::test]
    async fn test_checkpoints_are_bounded_keeping_the_recent_past() {
        let instance = make_instance(checkpointing_flow(2));
        let callbacks = MockLoopCallbacks::new(instance);

        super::execute_flow("test-instance-1", &callbacks)
            .await
            .unwrap();

        let saved = callbacks.saved_instance();
        assert_eq!(saved.checkpoints.len(), 2);
        let ids: Vec<&str> = saved
            .checkpoints
            .iter()
            .map(|c| c.node_id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec!["a", "end"],
            "the earliest is dropped, not the latest"
        );
    }

    /// Replay is a FORK, never a rewind: the original keeps its history and its
    /// terminal state, because its side effects already happened.
    #[tokio::test]
    async fn test_fork_from_checkpoint_leaves_the_original_alone() {
        let instance = make_instance(checkpointing_flow(10));
        let callbacks = MockLoopCallbacks::new(instance);
        super::execute_flow("test-instance-1", &callbacks)
            .await
            .unwrap();

        let original = callbacks.saved_instance();
        assert_eq!(original.status, FlowStatus::Completed);

        let fork = original
            .fork_from_checkpoint(1, "forked-instance")
            .expect("checkpoint 1 exists");

        assert_eq!(fork.id, "forked-instance");
        assert_eq!(fork.current_node_id, "a", "positioned at the checkpoint");
        assert_eq!(fork.status, FlowStatus::Pending, "a fork has not run yet");
        assert!(
            fork.checkpoints.is_empty(),
            "the fork starts its own history"
        );
        assert!(fork.completed_at.is_none());
        assert_eq!(
            fork.flow_definition_snapshot, original.flow_definition_snapshot,
            "a fork is the same flow"
        );

        // The original is untouched.
        assert_eq!(original.status, FlowStatus::Completed);
        assert_eq!(original.checkpoints.len(), 3);
    }

    /// An unknown sequence number is `None`, not a silently mispositioned fork.
    #[tokio::test]
    async fn test_fork_from_unknown_checkpoint_is_none() {
        let instance = make_instance(checkpointing_flow(10));
        let callbacks = MockLoopCallbacks::new(instance);
        super::execute_flow("test-instance-1", &callbacks)
            .await
            .unwrap();

        assert!(callbacks
            .saved_instance()
            .fork_from_checkpoint(99, "nope")
            .is_none());
    }

    // -----------------------------------------------------------------------
    // step_outputs staleness
    // -----------------------------------------------------------------------

    /// A step that produces NOTHING must overwrite its previous output, not
    /// leave it standing.
    ///
    /// This is the bug that shipped: a per-locale translation loop whose agent
    /// call came back empty re-read the PREVIOUS locale's `steps.<id>` and wrote
    /// German text into the French and Italian overlays, reporting success for
    /// both. The write used to be guarded by `if !output.is_null()`, which made
    /// "produced nothing" indistinguishable from "produced what it did last
    /// time". `start` is the convenient probe here because it returns
    /// `Value::Null`.
    #[tokio::test]
    async fn test_null_output_overwrites_stale_step_output() {
        let mut instance = make_instance(start_end_flow());
        instance.variables = json!({
            "step_outputs": { "start": { "locale": "de", "translations": ["stale"] } }
        });
        let callbacks = MockLoopCallbacks::new(instance);

        let result = super::execute_flow("test-instance-1", &callbacks).await;
        assert!(result.is_ok(), "flow should complete: {result:?}");

        let saved = callbacks.saved_instance();
        let recorded = saved
            .variables
            .get("step_outputs")
            .and_then(|o| o.get("start"))
            .expect("the step must record an entry even when it produced nothing");
        assert!(
            recorded.is_null(),
            "a step that produced nothing must read as nothing, not as its previous run; got {recorded:?}"
        );
    }
}
