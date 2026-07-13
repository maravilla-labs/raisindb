// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Scheduled invocation operation implementations for RaisinFunctionApi

use raisin_error::Result;
use serde_json::Value;

use super::RaisinFunctionApi;

impl RaisinFunctionApi {
    pub(crate) async fn impl_scheduler_schedule(&self, request: Value) -> Result<Value> {
        let callback = self.callbacks.scheduler_schedule.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation(
                "Scheduler schedule callback not configured".to_string(),
            )
        })?;

        callback(request).await
    }

    pub(crate) async fn impl_scheduler_cancel(&self, job_id_or_key: &str) -> Result<Value> {
        let callback = self.callbacks.scheduler_cancel.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Scheduler cancel callback not configured".to_string())
        })?;

        callback(job_id_or_key.to_string()).await
    }

    pub(crate) async fn impl_scheduler_list(&self, filter: Value) -> Result<Value> {
        let callback = self.callbacks.scheduler_list.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Scheduler list callback not configured".to_string())
        })?;

        callback(filter).await
    }

    pub(crate) async fn impl_scheduler_get(&self, job_id_or_key: &str) -> Result<Value> {
        let callback = self.callbacks.scheduler_get.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Scheduler get callback not configured".to_string())
        })?;

        callback(job_id_or_key.to_string()).await
    }
}
