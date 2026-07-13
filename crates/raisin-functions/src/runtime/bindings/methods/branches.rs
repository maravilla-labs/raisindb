// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Branch API bindings (`raisin.branches.*`)

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Get all branch method descriptors
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // branches.diff(branch, baseBranch) - per-node diff of `branch`
        // relative to `baseBranch`'s merge-base. Returns
        // { common_ancestor, added: [...], modified: [...], deleted: [...] }.
        ApiMethodDescriptor {
            internal_name: "branches_diff",
            js_name: "diff",
            py_name: "diff",
            category: "branches",
            args: vec![
                ArgSpec::new("branch", ArgType::String),
                ArgSpec::new("baseBranch", ArgType::String),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let branch = parser.string()?;
                    let base_branch = parser.string()?;

                    let result = api.branch_diff(&branch, &base_branch).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // branches.compare(branch, baseBranch) - divergence (commits
        // ahead/behind) of `branch` relative to `baseBranch`. Returns
        // { ahead, behind, common_ancestor }.
        ApiMethodDescriptor {
            internal_name: "branches_compare",
            js_name: "compare",
            py_name: "compare",
            category: "branches",
            args: vec![
                ArgSpec::new("branch", ArgType::String),
                ArgSpec::new("baseBranch", ArgType::String),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let branch = parser.string()?;
                    let base_branch = parser.string()?;

                    let result = api.branch_compare(&branch, &base_branch).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // branches.copyNodes(sourceBranch, targetBranch, opts) - copy a node
        // set across branches (ids preserved, one atomic commit on the
        // target). opts: { workspace, roots, recursive?, deleteMissing? }.
        // Returns { copied, deleted, revision, changes: [...] }.
        ApiMethodDescriptor {
            internal_name: "branches_copyNodes",
            js_name: "copyNodes",
            py_name: "copy_nodes",
            category: "branches",
            args: vec![
                ArgSpec::new("sourceBranch", ArgType::String),
                ArgSpec::new("targetBranch", ArgType::String),
                ArgSpec::new("opts", ArgType::Json),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let source_branch = parser.string()?;
                    let target_branch = parser.string()?;
                    let opts = parser.json()?;

                    let result = api
                        .branch_copy_nodes(&source_branch, &target_branch, opts)
                        .await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
    ]
}
