// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Lock / inventory API registration for the QuickJS runtime.
//!
//! Registers the internal `__raisin_internal.{locks_*,inventory_*}` functions
//! that back `raisin.locks.*` / `raisin.inventory.*`. These delegate to the
//! `FunctionApi` lock methods, which in turn call the shared `LockManager`.
//!
//! NOTE: QuickJS uses this hand-written binding layer (not the shared bindings
//! registry, which currently only drives the Starlark runtime). New `raisin.*`
//! methods must be registered here to be callable from JavaScript functions.

use rquickjs::{Ctx, Function, Object};
use std::sync::Arc;

use super::helpers::{json_error, run_async_blocking};
use crate::api::FunctionApi;

/// Register internal lock / inventory API functions.
pub(super) fn register_locks_internal<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    api: Arc<dyn FunctionApi>,
) -> std::result::Result<(), rquickjs::Error> {
    // locks_acquire(key, ttl_ms, owner?) -> JSON string {acquired, ...} | {error}
    let api_acquire = api.clone();
    let acquire_fn = Function::new(
        ctx.clone(),
        move |key: String, ttl_ms: i64, owner: Option<String>| {
            let api = api_acquire.clone();
            let result =
                run_async_blocking(async move { api.lock_acquire(&key, ttl_ms, owner).await });
            // Discriminated shape, consistent across all surfaces (functions, REST,
            // WS, SQL): {acquired:true, key, token, expires_at_ms} on success,
            // {acquired:false} on a lost tie-breaker.
            match result {
                Ok(Some(mut v)) => {
                    if let Some(obj) = v.as_object_mut() {
                        obj.insert("acquired".to_string(), serde_json::Value::Bool(true));
                    }
                    serde_json::to_string(&v).unwrap_or_else(|_| "null".to_string())
                }
                Ok(None) => serde_json::json!({ "acquired": false }).to_string(),
                Err(e) => {
                    tracing::error!(error = %e, "lock_acquire failed");
                    json_error(&e)
                }
            }
        },
    )?;
    internal.set("locks_acquire", acquire_fn)?;

    // locks_release(key, token) -> bool
    let api_release = api.clone();
    let release_fn = Function::new(ctx.clone(), move |key: String, token: i64| {
        let api = api_release.clone();
        let result = run_async_blocking(async move { api.lock_release(&key, token).await });
        match result {
            Ok(b) => b,
            Err(e) => {
                tracing::error!(error = %e, "lock_release failed");
                false
            }
        }
    })?;
    internal.set("locks_release", release_fn)?;

    // locks_renew(key, token, ttl_ms) -> bool
    let api_renew = api.clone();
    let renew_fn = Function::new(ctx.clone(), move |key: String, token: i64, ttl_ms: i64| {
        let api = api_renew.clone();
        let result = run_async_blocking(async move { api.lock_renew(&key, token, ttl_ms).await });
        match result {
            Ok(b) => b,
            Err(e) => {
                tracing::error!(error = %e, "lock_renew failed");
                false
            }
        }
    })?;
    internal.set("locks_renew", renew_fn)?;

    // inventory_claim(pool, n, capacity) -> JSON string {claimed, ...} | {error}
    let api_claim = api.clone();
    let claim_fn = Function::new(ctx.clone(), move |pool: String, n: i64, capacity: i64| {
        let api = api_claim.clone();
        let result =
            run_async_blocking(async move { api.inventory_claim(&pool, n, capacity).await });
        // Discriminated shape, consistent across all surfaces: {claimed:true,
        // remaining} on success, {claimed:false} when sold out.
        match result {
            Ok(Some(mut v)) => {
                if let Some(obj) = v.as_object_mut() {
                    obj.insert("claimed".to_string(), serde_json::Value::Bool(true));
                }
                serde_json::to_string(&v).unwrap_or_else(|_| "null".to_string())
            }
            Ok(None) => serde_json::json!({ "claimed": false }).to_string(),
            Err(e) => {
                tracing::error!(error = %e, "inventory_claim failed");
                json_error(&e)
            }
        }
    })?;
    internal.set("inventory_claim", claim_fn)?;

    // inventory_release(pool, n) -> i64 (new remaining; -1 on error)
    let api_release_claim = api.clone();
    let release_claim_fn = Function::new(ctx.clone(), move |pool: String, n: i64| {
        let api = api_release_claim.clone();
        let result = run_async_blocking(async move { api.inventory_release(&pool, n).await });
        match result {
            Ok(remaining) => remaining,
            Err(e) => {
                tracing::error!(error = %e, "inventory_release failed");
                -1
            }
        }
    })?;
    internal.set("inventory_release", release_claim_fn)?;

    Ok(())
}
