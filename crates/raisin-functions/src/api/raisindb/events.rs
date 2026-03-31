// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Event emission operation implementation for RaisinFunctionApi

use raisin_error::Result;
use serde_json::Value;

use super::RaisinFunctionApi;

impl RaisinFunctionApi {
    pub(crate) async fn impl_emit_event(&self, event_type: &str, data: Value) -> Result<()> {
        let callback = self.callbacks.emit_event.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Emit event callback not configured".to_string())
        })?;

        callback(event_type.to_string(), data).await
    }
}
