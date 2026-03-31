// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Function-to-function call implementations for RaisinFunctionApi

use raisin_error::Result;
use serde_json::Value;

use super::RaisinFunctionApi;
use crate::api::FunctionExecuteContext;

impl RaisinFunctionApi {
    pub(crate) async fn impl_function_execute(
        &self,
        function_path: &str,
        arguments: Value,
        context: FunctionExecuteContext,
    ) -> Result<Value> {
        let callback = self.callbacks.function_execute.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Function execute callback not configured".to_string())
        })?;

        callback(function_path.to_string(), arguments, context).await
    }

    pub(crate) async fn impl_function_call(
        &self,
        function_path: &str,
        arguments: Value,
    ) -> Result<Value> {
        let callback = self.callbacks.function_call.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Function call callback not configured".to_string())
        })?;

        callback(function_path.to_string(), arguments).await
    }
}
