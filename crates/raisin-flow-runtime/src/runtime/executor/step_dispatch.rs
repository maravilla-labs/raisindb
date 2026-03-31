// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Step dispatch logic
//!
//! Routes each flow step to the appropriate handler based on its `StepType`.

use crate::handlers::StepHandler;
use crate::types::{
    FlowCallbacks, FlowDefinition, FlowError, FlowInstance, FlowNode, FlowResult, StepResult,
    StepType,
};
use serde_json::Value;
use tracing::warn;

use super::helpers::{build_context_from_instance, sync_context_to_instance};

/// Internal step execution logic (without branch handling)
pub(crate) async fn execute_step_inner(
    step: &FlowNode,
    instance: &mut FlowInstance,
    flow_def: &FlowDefinition,
    callbacks: &dyn FlowCallbacks,
) -> FlowResult<StepResult> {
    // Build context from instance
    let mut context = build_context_from_instance(instance);

    let result = match step.step_type {
        StepType::Start => {
            // Start node - just continue to next
            let next_node_id = flow_def.next_node_id(&step.id).ok_or_else(|| {
                FlowError::InvalidDefinition("Start node has no next node".to_string())
            })?;
            Ok(StepResult::Continue {
                next_node_id,
                output: Value::Null,
            })
        }
        StepType::End => {
            // End node - complete the flow
            Ok(StepResult::Complete {
                output: instance.variables.clone(),
            })
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
