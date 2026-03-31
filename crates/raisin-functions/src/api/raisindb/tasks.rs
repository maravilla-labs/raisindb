// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Task operation implementations for RaisinFunctionApi

use raisin_error::Result;
use serde_json::Value;

use super::RaisinFunctionApi;

impl RaisinFunctionApi {
    pub(crate) async fn impl_task_create(&self, request: Value) -> Result<Value> {
        let callback = self.callbacks.task_create.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Task create callback not configured".to_string())
        })?;

        callback(request).await
    }

    pub(crate) async fn impl_task_update(&self, task_id: &str, updates: Value) -> Result<Value> {
        let callback = self.callbacks.task_update.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Task update callback not configured".to_string())
        })?;

        callback(task_id.to_string(), updates).await
    }

    pub(crate) async fn impl_task_complete(&self, task_id: &str, response: Value) -> Result<Value> {
        let callback = self.callbacks.task_complete.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Task complete callback not configured".to_string())
        })?;

        callback(task_id.to_string(), response).await
    }

    pub(crate) async fn impl_task_query(&self, query: Value) -> Result<Vec<Value>> {
        let callback = self.callbacks.task_query.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Task query callback not configured".to_string())
        })?;

        callback(query).await
    }
}
