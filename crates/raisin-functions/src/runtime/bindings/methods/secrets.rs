// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Secret store API bindings.
//!
//! Exposes `raisin.secrets.*` (get / put / list / rotate / delete) to
//! server-side functions. One descriptor set reaches both runtimes: QuickJS via
//! the `secrets` block in `quickjs/api_wrapper.js`, Starlark auto-generated from
//! the `"secrets"` category.
//!
//! # Deny by default, and why that matters more here than for `http`
//!
//! Every method is gated by the function's
//! [`SecretPolicy`](crate::types::SecretPolicy) inside
//! `api/raisindb/secrets.rs`, BEFORE the store is touched — and the policy
//! denies everything unless the function's `.node.yaml` declares
//! `secret_policy.allowed_names`.
//!
//! That default is load-bearing. Virtual-node adapters run privileged, with
//! row-level security bypassed, AND they hold `raisin.http`. An unrestricted
//! secrets binding would therefore be a one-line exfiltration path for every
//! credential in the repo — read them all, POST them anywhere. The allow-list
//! is what confines an adapter to the credentials it was actually granted.
//!
//! # Deliberately NOT mirrored into `raisin.admin`
//!
//! The `admin_*` namespaces exist to bypass row-level security. Secrets are not
//! RLS-scoped — they are gated by the function's own declared policy — so an
//! `admin.secrets` would bypass nothing and could only serve as a second door
//! past the one gate that matters. There is one door.

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Get all secret method descriptors
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // secrets.get(nameOrRef, version?) -> plaintext string
        // `nameOrRef` is a bare name OR a full `secret://name@version` — the
        // latter is what a node property in an `encrypted: true` field holds.
        ApiMethodDescriptor {
            internal_name: "secrets_get",
            js_name: "get",
            py_name: "get",
            category: "secrets",
            args: vec![
                ArgSpec::new("name", ArgType::String),
                ArgSpec::new("version", ArgType::OptionalI64),
            ],
            return_type: ReturnType::String,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let name = parser.string()?;
                    let version = parser.optional_i64()?;
                    let value = api.secret_get(&name, version).await?;
                    Ok(InvokeResult::String(value))
                })
            },
        },
        // secrets.resolve(value) -> plaintext, or `value` unchanged
        ApiMethodDescriptor {
            internal_name: "secrets_resolve",
            js_name: "resolve",
            py_name: "resolve",
            category: "secrets",
            args: vec![ArgSpec::new("value", ArgType::String)],
            return_type: ReturnType::String,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let value = parser.string()?;
                    Ok(InvokeResult::String(api.secret_resolve(&value).await?))
                })
            },
        },
        // secrets.put(name, value) -> { name, version }
        ApiMethodDescriptor {
            internal_name: "secrets_put",
            js_name: "put",
            py_name: "put",
            category: "secrets",
            args: vec![
                ArgSpec::new("name", ArgType::String),
                ArgSpec::new("value", ArgType::String),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let name = parser.string()?;
                    let value = parser.string()?;
                    Ok(InvokeResult::Json(api.secret_put(&name, &value).await?))
                })
            },
        },
        // secrets.list() -> [ metadata ] (never ciphertext or plaintext)
        ApiMethodDescriptor {
            internal_name: "secrets_list",
            js_name: "list",
            py_name: "list",
            category: "secrets",
            args: vec![],
            return_type: ReturnType::JsonArray,
            invoker: |api: Arc<dyn FunctionApi>,
                      _args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move { Ok(InvokeResult::JsonArray(api.secret_list().await?)) })
            },
        },
        // secrets.rotate(name, newValue) -> { name, version }
        ApiMethodDescriptor {
            internal_name: "secrets_rotate",
            js_name: "rotate",
            py_name: "rotate",
            category: "secrets",
            args: vec![
                ArgSpec::new("name", ArgType::String),
                ArgSpec::new("value", ArgType::String),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let name = parser.string()?;
                    let value = parser.string()?;
                    Ok(InvokeResult::Json(api.secret_rotate(&name, &value).await?))
                })
            },
        },
        // secrets.delete(name) -> { name, version } (tombstone ordinal)
        ApiMethodDescriptor {
            internal_name: "secrets_delete",
            js_name: "delete",
            py_name: "delete",
            category: "secrets",
            args: vec![ArgSpec::new("name", ArgType::String)],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let name = parser.string()?;
                    Ok(InvokeResult::Json(api.secret_delete(&name).await?))
                })
            },
        },
    ]
}
