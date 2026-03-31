// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Function executor callback for flow execution

use super::types::FunctionExecutorCallback;
use crate::execution::ExecutionDependencies;
use raisin_binary::BinaryStorage;
use raisin_storage::{transactional::TransactionalStorage, Storage};
use std::sync::Arc;

/// Create function executor callback - executes serverless functions
pub(super) fn create_function_executor<S, B>(
    deps: &Arc<ExecutionDependencies<S, B>>,
) -> FunctionExecutorCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    let deps = deps.clone();
    Arc::new(
        move |function_ref, input, tenant_id, repo_id, branch, workspace| {
            let deps = deps.clone();
            Box::pin(async move {
                tracing::debug!(
                    function_ref = %function_ref,
                    tenant_id = %tenant_id,
                    repo_id = %repo_id,
                    branch = %branch,
                    workspace = %workspace,
                    "Flow function_executor callback"
                );

                // Use the existing function executor if available
                // This reuses the same execution logic as FunctionExecutionHandler
                use crate::execution::execute_function;
                use crate::execution::FunctionExecutionConfig;

                let execution_id = nanoid::nanoid!();
                let config = FunctionExecutionConfig::default();

                execute_function(
                    &deps,
                    &config,
                    &function_ref,
                    &execution_id,
                    input,
                    &tenant_id,
                    &repo_id,
                    &branch,
                    &workspace,
                    None, // auth context
                    None, // no real-time log streaming for flow functions
                )
                .await
                .map_err(|e| format!("Function execution failed: {}", e))
                .map(|result| result.result.unwrap_or(serde_json::json!(null)))
            })
        },
    )
}
