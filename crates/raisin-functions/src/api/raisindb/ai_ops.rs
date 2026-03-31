// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! AI operation implementations for RaisinFunctionApi

use raisin_error::Result;
use serde_json::Value;

use super::RaisinFunctionApi;

impl RaisinFunctionApi {
    pub(crate) async fn impl_ai_completion(&self, request: Value) -> Result<Value> {
        let callback = self.callbacks.ai_completion.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("AI completion callback not configured".to_string())
        })?;

        callback(request).await
    }

    pub(crate) async fn impl_ai_list_models(&self) -> Result<Vec<Value>> {
        let callback = self.callbacks.ai_list_models.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("AI list models callback not configured".to_string())
        })?;

        callback().await
    }

    pub(crate) async fn impl_ai_get_default_model(&self, use_case: &str) -> Result<Option<String>> {
        let callback = self
            .callbacks
            .ai_get_default_model
            .as_ref()
            .ok_or_else(|| {
                raisin_error::Error::Validation(
                    "AI get default model callback not configured".to_string(),
                )
            })?;

        callback(use_case.to_string()).await
    }

    pub(crate) async fn impl_ai_embed(&self, request: Value) -> Result<Value> {
        let callback = self.callbacks.ai_embed.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("AI embed callback not configured".to_string())
        })?;

        callback(request).await
    }
}
