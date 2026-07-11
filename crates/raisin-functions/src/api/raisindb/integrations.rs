// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Integration / mount operation implementations for RaisinFunctionApi.

use raisin_error::Result;
use serde_json::Value;

use super::RaisinFunctionApi;

impl RaisinFunctionApi {
    /// Back `raisin.integrations.sync_now(mountId, mode?)` by delegating to the
    /// wired job-queue callback. Fails cleanly when the job system is not
    /// configured for this runtime (e.g. the in-memory backend).
    pub(crate) async fn impl_integrations_sync_now(
        &self,
        mount_id: &str,
        mode: Option<&str>,
    ) -> Result<Value> {
        let callback = self
            .callbacks
            .integrations_sync_now
            .as_ref()
            .ok_or_else(|| {
                raisin_error::Error::Validation(
                    "Integration sync is not available in this runtime \
                     (job system not configured)."
                        .to_string(),
                )
            })?;
        callback(mount_id.to_string(), mode.map(|s| s.to_string())).await
    }
}
