// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Branch operation implementations for RaisinFunctionApi

use raisin_error::Result;
use serde_json::Value;

use super::RaisinFunctionApi;

impl RaisinFunctionApi {
    pub(crate) async fn impl_branch_diff(&self, branch: &str, base_branch: &str) -> Result<Value> {
        let callback = self.callbacks.branch_diff.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Branch diff callback not configured".to_string())
        })?;

        callback(branch.to_string(), base_branch.to_string()).await
    }

    pub(crate) async fn impl_branch_compare(
        &self,
        branch: &str,
        base_branch: &str,
    ) -> Result<Value> {
        let callback = self.callbacks.branch_compare.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Branch compare callback not configured".to_string())
        })?;

        callback(branch.to_string(), base_branch.to_string()).await
    }

    pub(crate) async fn impl_branch_copy_nodes(
        &self,
        source_branch: &str,
        target_branch: &str,
        opts: Value,
    ) -> Result<Value> {
        let callback = self.callbacks.branch_copy_nodes.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Branch copy-nodes callback not configured".to_string())
        })?;

        callback(source_branch.to_string(), target_branch.to_string(), opts).await
    }
}
