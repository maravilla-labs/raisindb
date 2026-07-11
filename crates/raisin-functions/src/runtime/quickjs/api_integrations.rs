// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Integration / mount API registration for the QuickJS runtime.
//!
//! Registers the internal `__raisin_internal.integrations_sync_now` function
//! that backs `raisin.integrations.sync_now(mountId, mode?)`. It delegates to
//! the `FunctionApi` method, which enqueues a deduped `VirtualMountSync` job.
//!
//! NOTE: QuickJS uses this hand-written binding layer (not the shared bindings
//! registry, which drives the Starlark runtime). New `raisin.*` methods must be
//! registered here to be callable from JavaScript functions.

use rquickjs::{Ctx, Function, Object};
use std::sync::Arc;

use super::helpers::{json_error, run_async_blocking};
use crate::api::FunctionApi;

/// Register internal integration / mount API functions.
pub(super) fn register_integrations_internal<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    api: Arc<dyn FunctionApi>,
) -> std::result::Result<(), rquickjs::Error> {
    // integrations_sync_now(mountId, mode?) -> JSON string
    //   { job_id: String|null, status: "queued"|"already_running" } | { error }
    let api_sync = api.clone();
    let sync_fn = Function::new(
        ctx.clone(),
        move |mount_id: String, mode: Option<String>| {
            let api = api_sync.clone();
            let result = run_async_blocking(async move {
                api.integrations_sync_now(&mount_id, mode.as_deref()).await
            });
            match result {
                Ok(v) => serde_json::to_string(&v).unwrap_or_else(|_| "null".to_string()),
                Err(e) => {
                    tracing::error!(error = %e, "integrations_sync_now failed");
                    json_error(&e)
                }
            }
        },
    )?;
    internal.set("integrations_sync_now", sync_fn)?;

    Ok(())
}
