// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Tenant-identity API bindings.
//!
//! Exposes `raisin.identities.findByEmail(email)` and
//! `raisin.identities.update(id, patch)` to server-side functions. One
//! descriptor set reaches both runtimes: QuickJS via the `identities` block in
//! `quickjs/api_wrapper.js`, Starlark auto-generated from the `"identities"`
//! category.
//!
//! Every method is gated by the function's
//! [`IdentityPolicy`](crate::types::IdentityPolicy) inside
//! `api/raisindb/identities.rs`, which denies unless the function's
//! `.node.yaml` declares `identity_policy: { enabled: true }`. These are the
//! tenant's AUTH records — renaming or re-passwording one is an account
//! takeover if it lands in the wrong function — so the grant must be typed.
//!
//! Deliberately NOT mirrored into `raisin.admin`: identities are not
//! RLS-scoped, so there is nothing for an admin variant to bypass.

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Get all identity method descriptors.
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // identities.findByEmail(email) -> { id, email, email_verified, display_name, has_password } | null
        ApiMethodDescriptor {
            internal_name: "identities_findByEmail",
            js_name: "findByEmail",
            py_name: "find_by_email",
            category: "identities",
            args: vec![ArgSpec::new("email", ArgType::String)],
            return_type: ReturnType::OptionalJson,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let email = parser.string()?;
                    Ok(InvokeResult::OptionalJson(
                        api.identity_find_by_email(&email).await?,
                    ))
                })
            },
        },
        // identities.update(id, { email?, password?, display_name? }) -> same shape
        ApiMethodDescriptor {
            internal_name: "identities_update",
            js_name: "update",
            py_name: "update",
            category: "identities",
            args: vec![
                ArgSpec::new("id", ArgType::String),
                ArgSpec::new("patch", ArgType::Json),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let id = parser.string()?;
                    let patch = parser.json()?;
                    Ok(InvokeResult::Json(api.identity_update(&id, patch).await?))
                })
            },
        },
    ]
}
