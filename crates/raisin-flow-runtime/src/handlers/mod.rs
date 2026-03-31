// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Step handlers for flow execution
//!
//! Each step type in a flow has a corresponding handler that implements the
//! StepHandler trait. Handlers are responsible for executing the step logic
//! and returning the appropriate result.

use crate::types::{FlowCallbacks, FlowContext, FlowNode, FlowResult, StepResult};
use async_trait::async_trait;

pub mod agent_step;
pub mod ai_container;
pub mod ai_tool_loop;
pub mod chat_step;
pub mod conversation_persistence;
pub mod decision;
pub mod error;
pub mod function_step;
pub mod human_task;
pub mod loop_step;
pub mod parallel;
pub mod sub_flow;
pub mod wait;

// Re-exports
pub use agent_step::AgentStepHandler;
pub use ai_container::AiContainerHandler;
pub use chat_step::ChatStepHandler;
pub use decision::DecisionHandler;
pub use error::{ErrorClass, OnErrorBehavior, StepError};
pub use function_step::FunctionStepHandler;
pub use human_task::HumanTaskHandler;
pub use loop_step::LoopHandler;
pub use parallel::ParallelHandler;
pub use sub_flow::SubFlowHandler;
pub use wait::WaitHandler;

/// Step handler trait
///
/// Each step type implements this trait to provide its execution logic.
#[async_trait]
pub trait StepHandler: Send + Sync {
    /// Execute the step
    ///
    /// # Arguments
    /// * `step` - The flow node to execute
    /// * `context` - The current flow execution context
    /// * `callbacks` - Callbacks for interacting with external services
    ///
    /// # Returns
    /// The result of step execution
    async fn execute(
        &self,
        step: &FlowNode,
        context: &mut FlowContext,
        callbacks: &dyn FlowCallbacks,
    ) -> FlowResult<StepResult>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::collections::HashMap;

    #[test]
    fn test_flow_node_property_access() {
        use crate::types::{FlowNode, StepType};

        let mut properties = HashMap::new();
        properties.insert(
            "condition".to_string(),
            Value::String("input.value > 10".to_string()),
        );

        let node = FlowNode {
            id: "test-node".to_string(),
            step_type: StepType::Decision,
            properties,
            children: vec![],
            next_node: None,
        };

        assert_eq!(
            node.get_string_property("condition"),
            Some("input.value > 10".to_string())
        );
    }
}
