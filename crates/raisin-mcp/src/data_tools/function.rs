// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Custom function tools that invoke `raisin:Function` nodes over MCP.
//!
//! A [`FunctionTool`] maps a `customTools` entry of a `raisin:McpServer` onto a
//! named `raisin:Function`. On `tools/call` it runs the function through the
//! [`FunctionInvoker`](crate::services::FunctionInvoker) as the calling identity
//! and surfaces the real [`ExecutionResult`](raisin_functions::ExecutionResult):
//! the function's `output` on success, or its `error` as a tool error.

use serde_json::{json, Value};

use crate::error::{McpError, Result};
use crate::identity::McpIdentity;
use crate::registry::{Tool, ToolDescriptor, ToolKind};
use crate::server::CustomTool;
use crate::services::SharedFunctionInvoker;

/// A custom tool backed by a `raisin:Function`.
pub struct FunctionTool {
    spec: CustomTool,
    invoker: SharedFunctionInvoker,
}

impl FunctionTool {
    /// Build a function tool from its declaration and an invoker.
    pub fn new(spec: CustomTool, invoker: SharedFunctionInvoker) -> Self {
        Self { spec, invoker }
    }
}

impl Tool for FunctionTool {
    fn name(&self) -> &str {
        &self.spec.name
    }

    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            self.spec.name.clone(),
            self.spec.description.clone(),
            self.spec.input_schema.clone(),
            ToolKind::Function,
        )
        .with_scopes(self.spec.scopes.clone())
        .with_output_schema(self.spec.output_schema.clone())
    }

    async fn call(&self, identity: &McpIdentity, args: Value) -> Result<Value> {
        let result = self
            .invoker
            .invoke(identity, &self.spec.function, args)
            .await?;

        if result.success {
            let output = result.output.unwrap_or(Value::Null);
            // When the tool declares an `outputSchema`, return the function's
            // output directly so it conforms to that schema (the dispatcher then
            // surfaces it as `structuredContent`). Otherwise keep the wrapped
            // `{ output, logs }` shape.
            if self.spec.output_schema.is_some() {
                Ok(output)
            } else {
                Ok(json!({ "output": output, "logs": result.logs }))
            }
        } else {
            let message = result
                .error
                .map(|e| e.to_string())
                .unwrap_or_else(|| "function reported failure".to_string());
            Err(McpError::function_failed(message))
        }
    }
}
