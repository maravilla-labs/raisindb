// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Lock / inventory operation implementations for RaisinFunctionApi

use raisin_error::Result;
use serde_json::Value;

use super::RaisinFunctionApi;

fn disabled(op: &str) -> raisin_error::Error {
    raisin_error::Error::Validation(format!(
        "Locks subsystem not configured (cannot {op}). Enable [locks] in server config."
    ))
}

impl RaisinFunctionApi {
    pub(crate) async fn impl_lock_acquire(
        &self,
        key: &str,
        ttl_ms: i64,
        owner: Option<String>,
    ) -> Result<Option<Value>> {
        let callback = self
            .callbacks
            .lock_acquire
            .as_ref()
            .ok_or_else(|| disabled("acquire lock"))?;
        callback(key.to_string(), ttl_ms, owner).await
    }

    pub(crate) async fn impl_lock_release(&self, key: &str, token: i64) -> Result<bool> {
        let callback = self
            .callbacks
            .lock_release
            .as_ref()
            .ok_or_else(|| disabled("release lock"))?;
        callback(key.to_string(), token).await
    }

    pub(crate) async fn impl_lock_renew(&self, key: &str, token: i64, ttl_ms: i64) -> Result<bool> {
        let callback = self
            .callbacks
            .lock_renew
            .as_ref()
            .ok_or_else(|| disabled("renew lock"))?;
        callback(key.to_string(), token, ttl_ms).await
    }

    pub(crate) async fn impl_inventory_claim(
        &self,
        pool: &str,
        n: i64,
        capacity: i64,
    ) -> Result<Option<Value>> {
        let callback = self
            .callbacks
            .inventory_claim
            .as_ref()
            .ok_or_else(|| disabled("claim inventory"))?;
        callback(pool.to_string(), n, capacity).await
    }

    pub(crate) async fn impl_inventory_release(&self, pool: &str, n: i64) -> Result<i64> {
        let callback = self
            .callbacks
            .inventory_release
            .as_ref()
            .ok_or_else(|| disabled("release inventory"))?;
        callback(pool.to_string(), n).await
    }
}
