// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Resource and PDF operation implementations for RaisinFunctionApi

use raisin_error::Result;
use serde_json::Value;

use super::RaisinFunctionApi;

impl RaisinFunctionApi {
    pub(crate) async fn impl_resource_get_binary(&self, storage_key: &str) -> Result<String> {
        let callback = self.callbacks.resource_get_binary.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation(
                "Resource get binary callback not configured".to_string(),
            )
        })?;

        callback(storage_key.to_string()).await
    }

    pub(crate) async fn impl_node_add_resource(
        &self,
        workspace: &str,
        node_path: &str,
        property_path: &str,
        upload_data: Value,
    ) -> Result<Value> {
        let callback = self.callbacks.node_add_resource.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Node add resource callback not configured".to_string())
        })?;

        callback(
            workspace.to_string(),
            node_path.to_string(),
            property_path.to_string(),
            upload_data,
        )
        .await
    }

    pub(crate) async fn impl_pdf_process_from_storage(
        &self,
        storage_key: &str,
        options: Value,
    ) -> Result<Value> {
        let callback = self
            .callbacks
            .pdf_process_from_storage
            .as_ref()
            .ok_or_else(|| {
                raisin_error::Error::Validation(
                    "PDF process from storage callback not configured".to_string(),
                )
            })?;

        callback(storage_key.to_string(), options).await
    }
}
