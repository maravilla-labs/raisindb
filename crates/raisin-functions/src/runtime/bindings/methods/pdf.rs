// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! PDF operation API bindings

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Get all PDF operation method descriptors
///
/// Note: Base64-based PDF operations (extractText, getPageCount, ocr)
/// are implemented locally in each runtime using temp file operations.
/// These bindings are for storage-key based operations that go through FunctionApi.
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // pdf.processFromStorage(storageKey, options)
        ApiMethodDescriptor {
            internal_name: "pdf_processFromStorage",
            js_name: "processFromStorage",
            py_name: "process_from_storage",
            category: "pdf",
            args: vec![
                ArgSpec::new("storageKey", ArgType::String),
                ArgSpec::new("options", ArgType::Json),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let storage_key = parser.string()?;
                    let options = parser.json()?;
                    let result = api.pdf_process_from_storage(&storage_key, options).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
    ]
}
