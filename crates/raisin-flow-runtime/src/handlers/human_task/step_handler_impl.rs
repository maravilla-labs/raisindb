// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! StepHandler trait implementation for HumanTaskHandler

use crate::handlers::StepHandler;
use crate::types::{
    FlowCallbacks, FlowContext, FlowError, FlowNode, FlowResult, StepResult, TaskType,
};
use async_trait::async_trait;
use tracing::{debug, error, instrument};

use super::handler::{HumanTaskHandler, WAIT_REASON_HUMAN_TASK};

#[async_trait]
impl StepHandler for HumanTaskHandler {
    #[instrument(skip(self, context, callbacks), fields(step_id = %step.id))]
    async fn execute(
        &self,
        step: &FlowNode,
        context: &mut FlowContext,
        callbacks: &dyn FlowCallbacks,
    ) -> FlowResult<StepResult> {
        debug!("Executing human task step: {}", step.id);

        // Extract task configuration
        let task_type = self.get_task_type(step)?;
        let assignee = self.get_assignee(step)?;

        // Build task properties
        let task_properties = self.build_task_properties(step, context, task_type)?;

        // Generate task path
        let task_path = self.generate_task_path(&assignee, &step.id);
        debug!("Creating inbox task at: {}", task_path);

        // Create the inbox task node via callbacks
        match callbacks
            .create_node("inbox_task", &task_path, task_properties.clone())
            .await
        {
            Ok(created_node) => {
                debug!("Inbox task created successfully: {}", task_path);
                debug!("Created node: {}", created_node);

                // Return Wait result
                Ok(StepResult::Wait {
                    reason: WAIT_REASON_HUMAN_TASK.to_string(),
                    metadata: serde_json::json!({
                        "task_path": task_path,
                        "task_type": match task_type {
                            TaskType::Approval => "approval",
                            TaskType::Input => "input",
                            TaskType::Review => "review",
                            TaskType::Action => "action",
                        },
                        "step_id": step.id,
                        "assignee": assignee,
                    }),
                })
            }
            Err(e) => {
                error!("Failed to create inbox task: {}", e);
                Err(FlowError::FunctionExecution(format!(
                    "Failed to create inbox task at '{}': {}",
                    task_path, e
                )))
            }
        }
    }
}
