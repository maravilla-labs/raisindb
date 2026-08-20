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

//! `raisin.platform.*` — operator-configured platform hooks.

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![ApiMethodDescriptor {
        internal_name: "platform_hook",
        js_name: "hook",
        py_name: "hook",
        category: "platform",
        args: vec![
            ArgSpec::new("name", ArgType::String),
            ArgSpec::new("payload", ArgType::Json),
        ],
        return_type: ReturnType::Json,
        invoker: |api: Arc<dyn FunctionApi>,
                  args: Vec<Value>|
         -> BoxFuture<'static, Result<InvokeResult>> {
            Box::pin(async move {
                let mut parser = ArgParser::new(&args);
                let name = parser.string()?;
                let payload = parser.json()?;
                let result = api.platform_hook(&name, payload).await?;
                Ok(InvokeResult::Json(result))
            })
        },
    }]
}
