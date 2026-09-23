// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Node-development bindings (`raisin.node_dev.*`, `raisin.nodeDev.*` in
//! JavaScript), for every runtime. Each takes one request object and returns
//! the typed result — or, in tool mode (`__raisin_context` present or
//! `envelope: true`), a `raisin.tool-result/1` envelope.

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

macro_rules! dev_method {
    ($internal:literal, $name:literal) => {
        ApiMethodDescriptor {
            internal_name: $internal,
            js_name: $name,
            py_name: $name,
            category: "node_dev",
            args: vec![ArgSpec::new("request", ArgType::Json)],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let request = parser.json()?;
                    let result = api.node_dev_call($name, request).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        }
    };
}

/// All node-development method descriptors.
///
/// Common request fields: `roots: [{ workspace, path?, ops? }]` (or
/// `workspace`), `branch?`, and in tool mode `__raisin_context`.
///
/// - reads: `stat({ target })`, `read({ target, keys? })`,
///   `list({ target?, limit? })`, `diff({ target, from?, to? })`,
///   `watch({ since?, limit? })`
/// - changesets: `dry_run({ ops })`, `propose({ ops, idempotency_key? })`,
///   `get_changeset({ changeset_id })`, `list_changesets({ status? })`,
///   `commit({ changeset_id, expected_digest? })`,
///   `discard({ changeset_id })`, `apply({ ops, idempotency_key? })`
/// - branches: `fork_branch({ name })`, `diff_branch({ base? })`,
///   `merge_branch({ target, dry_run?, message? })`, `discard_branch({})`
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        dev_method!("node_dev_stat", "stat"),
        dev_method!("node_dev_read", "read"),
        dev_method!("node_dev_list", "list"),
        dev_method!("node_dev_diff", "diff"),
        dev_method!("node_dev_watch", "watch"),
        dev_method!("node_dev_dry_run", "dry_run"),
        dev_method!("node_dev_propose", "propose"),
        dev_method!("node_dev_get_changeset", "get_changeset"),
        dev_method!("node_dev_list_changesets", "list_changesets"),
        dev_method!("node_dev_commit", "commit"),
        dev_method!("node_dev_discard", "discard"),
        dev_method!("node_dev_apply", "apply"),
        dev_method!("node_dev_fork_branch", "fork_branch"),
        dev_method!("node_dev_diff_branch", "diff_branch"),
        dev_method!("node_dev_merge_branch", "merge_branch"),
        dev_method!("node_dev_discard_branch", "discard_branch"),
    ]
}
