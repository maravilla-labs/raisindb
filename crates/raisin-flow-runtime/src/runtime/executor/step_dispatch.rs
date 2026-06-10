// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Step dispatch logic
//!
//! Routes each flow step to the appropriate handler based on its `StepType`.

use crate::handlers::StepHandler;
use crate::types::{
    FlowCallbacks, FlowDefinition, FlowError, FlowInstance, FlowNode, FlowResult, MockBehavior,
    StepResult, StepType,
};
use serde_json::Value;
use tracing::{info, warn};

use super::helpers::{build_context_from_instance, sync_context_to_instance};

/// Maximum artificial mock delay (guards against runaway test configs)
const MAX_MOCK_DELAY_MS: u64 = 30_000;

/// Test-run mock interception: when the instance carries a TestRunConfig,
/// work steps whose function/agent path has a non-Real mock skip their
/// handler and return the mock result directly.
///
/// Returns `None` when the step should execute normally.
async fn apply_test_mock(
    step: &FlowNode,
    instance: &FlowInstance,
    flow_def: &FlowDefinition,
) -> Option<FlowResult<StepResult>> {
    let test_config = instance.test_config.as_ref()?;
    if !test_config.is_test_run {
        return None;
    }

    let mock = match step.step_type {
        StepType::FunctionStep => {
            let path = step.get_string("function_ref")?;
            test_config.get_mock(&path)?
        }
        StepType::AgentStep | StepType::AIContainer | StepType::Chat => {
            let path = step.get_string("agent_ref").or_else(|| {
                step.properties
                    .get("ai_config")
                    .or_else(|| step.properties.get("chat_config"))
                    .and_then(|cfg| cfg.get("agent_ref"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })?;
            test_config.get_agent_mock(&path)?
        }
        _ => return None,
    };

    if mock.behavior == MockBehavior::Real {
        return None;
    }

    if let Some(delay_ms) = mock.mock_delay_ms {
        tokio::time::sleep(std::time::Duration::from_millis(
            delay_ms.min(MAX_MOCK_DELAY_MS),
        ))
        .await;
    }

    let output = match mock.behavior {
        MockBehavior::MockOutput => mock.mock_output.clone().unwrap_or(Value::Null),
        MockBehavior::Passthrough => step
            .properties
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| instance.input.clone()),
        MockBehavior::Real => unreachable!(),
    };

    info!("Test run: step {} mocked ({:?})", step.id, mock.behavior);

    let next_node_id = step
        .next_node
        .clone()
        .or_else(|| flow_def.next_node_id(&step.id))
        .unwrap_or_else(|| "end".to_string());

    Some(Ok(StepResult::Continue {
        next_node_id,
        output,
    }))
}

/// Internal step execution logic (without branch handling)
pub(crate) async fn execute_step_inner(
    step: &FlowNode,
    instance: &mut FlowInstance,
    flow_def: &FlowDefinition,
    callbacks: &dyn FlowCallbacks,
) -> FlowResult<StepResult> {
    // Test-run mocks bypass the real handler entirely
    if let Some(mock_result) = apply_test_mock(step, instance, flow_def).await {
        return mock_result;
    }

    // Build context from instance
    let mut context = build_context_from_instance(instance);

    let result = match step.step_type {
        StepType::Start => {
            // Start node (also used as a pass-through for lowered
            // containers) - continue to the next node. Prefer the explicit
            // next_node link; fall back to edge lookup.
            let next_node_id = step
                .next_node
                .clone()
                .or_else(|| flow_def.next_node_id(&step.id))
                .ok_or_else(|| {
                    FlowError::InvalidDefinition("Start node has no next node".to_string())
                })?;
            Ok(StepResult::Continue {
                next_node_id,
                output: Value::Null,
            })
        }
        StepType::End => {
            // End node - complete the flow. The output is the flow's
            // variables minus internal bookkeeping (step_outputs, __* keys),
            // so parents of sub-flows get a clean result object.
            let output = match &instance.variables {
                Value::Object(map) => Value::Object(
                    map.iter()
                        .filter(|(k, _)| !k.starts_with("__") && k.as_str() != "step_outputs")
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                ),
                other => other.clone(),
            };
            Ok(StepResult::Complete { output })
        }
        StepType::Decision => {
            // Decision node - use DecisionHandler
            use crate::handlers::DecisionHandler;
            let handler = DecisionHandler::new();
            handler.execute(step, &mut context, callbacks).await
        }
        StepType::FunctionStep => {
            // Function step - use FunctionStepHandler
            use crate::handlers::FunctionStepHandler;
            let handler = FunctionStepHandler::new();
            handler.execute(step, &mut context, callbacks).await
        }
        StepType::AgentStep => {
            // Single-shot AI agent call - use AgentStepHandler
            use crate::handlers::AgentStepHandler;
            let handler = AgentStepHandler::new();
            handler.execute(step, &mut context, callbacks).await
        }
        StepType::AgentDecision => {
            // AI-routed decision - an agent picks the next branch
            use crate::handlers::AgentDecisionHandler;
            let handler = AgentDecisionHandler::new();
            handler.execute(step, &mut context, callbacks).await
        }
        StepType::Competition => {
            // Competing agents judged by a referee
            use crate::handlers::AgentCompetitionHandler;
            let handler = AgentCompetitionHandler::new();
            handler.execute(step, &mut context, callbacks).await
        }
        StepType::AIContainer => {
            // AI container - use AiContainerHandler
            use crate::handlers::AiContainerHandler;
            let handler = AiContainerHandler::new();
            handler.execute(step, &mut context, callbacks).await
        }
        StepType::HumanTask => {
            // Human task - use HumanTaskHandler
            use crate::handlers::HumanTaskHandler;
            let handler = HumanTaskHandler::new();
            handler.execute(step, &mut context, callbacks).await
        }
        StepType::Chat => {
            // Chat session - use ChatStepHandler
            use crate::handlers::ChatStepHandler;
            let handler = ChatStepHandler::new();
            handler.execute(step, &mut context, callbacks).await
        }
        StepType::SubFlow => {
            // Sub-flow step - use SubFlowHandler
            use crate::handlers::SubFlowHandler;
            let handler = SubFlowHandler::new();
            handler.execute(step, &mut context, callbacks).await
        }
        StepType::Loop => {
            // Loop step - use LoopHandler
            use crate::handlers::LoopHandler;
            let handler = LoopHandler::new();
            handler.execute(step, &mut context, callbacks).await
        }
        StepType::Wait => {
            // Wait step - use WaitHandler
            use crate::handlers::WaitHandler;
            let handler = WaitHandler::new();
            handler.execute(step, &mut context, callbacks).await
        }
        StepType::Parallel => {
            // Parallel step - use ParallelHandler
            use crate::handlers::ParallelHandler;
            let handler = ParallelHandler::new();
            handler.execute(step, &mut context, callbacks).await
        }
        _ => {
            // Unknown step type - just continue
            warn!("Unknown step type: {:?}, continuing", step.step_type);
            let next_node_id = flow_def
                .next_node_id(&step.id)
                .unwrap_or_else(|| "end".to_string());
            Ok(StepResult::Continue {
                next_node_id,
                output: Value::Null,
            })
        }
    };

    // Sync context changes back to instance
    sync_context_to_instance(&context, instance);

    result
}
