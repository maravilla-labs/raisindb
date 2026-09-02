// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Asset API bindings — the extraction writeback.
//!
//! One method, and the boundary it sits on is the point of it. Core reads PDFs
//! and nothing else; Word, PowerPoint and Excel are converted by a media plugin
//! called from a function, because LibreOffice must not live inside this
//! process. That leaves plugin-produced text stranded in JavaScript: the
//! extraction properties are engine-owned and the write-path shield refuses
//! them to tenant code.
//!
//! `raisin.assets.setExtractedText` is the door. It is narrow deliberately —
//! it takes the text and nothing core already knows. See
//! `crate::execution::callbacks::assets` for why the fingerprint is computed
//! server-side and why chunking stays on the `node:updated` path.

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Get all asset operation method descriptors.
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // assets.setExtractedText(workspace, nodeIdOrPath, text, options)
        //   -> { status, source, chars, stored }
        ApiMethodDescriptor {
            internal_name: "asset_set_extraction",
            js_name: "setExtractedText",
            py_name: "set_extracted_text",
            category: "assets",
            args: vec![
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("nodeRef", ArgType::String),
                ArgSpec::new("text", ArgType::String),
                ArgSpec::new("options", ArgType::Json),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let workspace = parser.string()?;
                    let node_ref = parser.string()?;
                    let text = parser.string()?;
                    let options = parser.json()?;
                    let result = api
                        .asset_set_extraction(&workspace, &node_ref, &text, options)
                        .await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // assets.reextract(workspace, nodeRef)
        ApiMethodDescriptor {
            internal_name: "asset_reextract",
            js_name: "reextract",
            py_name: "reextract",
            category: "assets",
            args: vec![
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("nodeRef", ArgType::String),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let workspace = parser.string()?;
                    let node_ref = parser.string()?;
                    let result = api.asset_reextract(&workspace, &node_ref).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // assets.signedUrl(workspace, nodeRef, options)
        //   -> { url, expiresAt, expiresIn, command, property, path }
        //
        // The point of it is what does NOT happen: the bytes never enter the
        // isolate. `getBinary()` + base64 holds three copies of the file in the
        // JS heap and killed a production run on a 1.5 MB video.
        ApiMethodDescriptor {
            internal_name: "asset_signed_url",
            js_name: "signedUrl",
            py_name: "signed_url",
            category: "assets",
            args: vec![
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("nodeRef", ArgType::String),
                ArgSpec::new("options", ArgType::Json),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let workspace = parser.string()?;
                    let node_ref = parser.string()?;
                    let options = parser.json()?;
                    let result = api.asset_signed_url(&workspace, &node_ref, options).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // assets.ensureContent(workspace, nodeRef)
        //
        // Called by the Resource wrapper, not by function authors: it is what
        // makes `getBinary()` work on a mounted asset whose cache has expired.
        ApiMethodDescriptor {
            internal_name: "asset_ensure_content",
            js_name: "ensureContent",
            py_name: "ensure_content",
            category: "assets",
            args: vec![
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("nodeRef", ArgType::String),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let workspace = parser.string()?;
                    let node_ref = parser.string()?;
                    let result = api.asset_ensure_content(&workspace, &node_ref).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
    ]
}
