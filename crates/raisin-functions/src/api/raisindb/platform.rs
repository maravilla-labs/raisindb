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

//! `raisin.platform.*` delegation.

use super::RaisinFunctionApi;
use raisin_error::Result;
use serde_json::Value;

impl RaisinFunctionApi {
    pub(crate) async fn impl_platform_hook(&self, name: &str, payload: Value) -> Result<Value> {
        let callback = self.callbacks.platform_hook.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Platform hook callback not configured".to_string())
        })?;
        callback(name.to_string(), payload).await
    }
}
