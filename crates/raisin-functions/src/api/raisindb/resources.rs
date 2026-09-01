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

    /// Fetch a stored blob as raw bytes.
    ///
    /// Goes through the base64 callback and decodes, rather than adding a
    /// second byte-returning callback beside it. That costs one extra copy of
    /// the blob, which is bounded by the attachment limits — and it buys the
    /// thing that matters more: there is still exactly ONE storage-read
    /// callback to wire up. A parallel callback would be `None` in any
    /// construction path that forgot it, and the symptom would be attachments
    /// working over one transport and silently failing over another.
    pub(crate) async fn impl_resource_get_bytes(&self, storage_key: &str) -> Result<Vec<u8>> {
        use base64::Engine as _;
        let encoded = self.impl_resource_get_binary(storage_key).await?;
        base64::engine::general_purpose::STANDARD
            .decode(encoded.as_bytes())
            .map_err(|e| {
                raisin_error::Error::Backend(format!(
                    "stored object '{storage_key}' did not decode as base64: {e}"
                ))
            })
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

    /// Store an extraction artifact produced above this layer. See
    /// `crate::execution::callbacks::assets` for why the primitive exists.
    pub(crate) async fn impl_asset_set_extraction(
        &self,
        workspace: &str,
        node_ref: &str,
        text: &str,
        options: Value,
    ) -> Result<Value> {
        let callback = self
            .callbacks
            .asset_set_extraction
            .as_ref()
            .ok_or_else(|| {
                raisin_error::Error::Validation(
                    "Asset extraction writeback callback not configured".to_string(),
                )
            })?;

        callback(
            workspace.to_string(),
            node_ref.to_string(),
            text.to_string(),
            options,
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
