// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! StepHandler trait implementation for ParallelHandler

use crate::handlers::StepHandler;
use crate::types::{FlowCallbacks, FlowContext, FlowNode, FlowResult, StepResult};
use async_trait::async_trait;
use tracing::{debug, instrument};

use super::handler::ParallelHandler;

#[async_trait]
impl StepHandler for ParallelHandler {
    #[instrument(skip(self, context, callbacks), fields(step_id = %step.id))]
    async fn execute(
        &self,
        step: &FlowNode,
        context: &mut FlowContext,
        callbacks: &dyn FlowCallbacks,
    ) -> FlowResult<StepResult> {
        debug!("Executing parallel container: {}", step.id);

        // Check if this is a fork (initial execution) or join (resuming)
        // This would be tracked in wait_info in real implementation
        // For now, we'll fork every time
        self.fork_branches(step, context, callbacks).await
    }
}
