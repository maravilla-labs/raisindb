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
        // assets.publicUrl(workspace, path, options)
        //   -> { url, origin, repo, branch, workspace, path }
        //
        // The anonymous, query-less address of a PUBLISHED asset on this
        // tenant's public origin. Origin comes from operator config, never from
        // the caller or a request header. options: { branch?: "publish" }.
        ApiMethodDescriptor {
            internal_name: "asset_public_url",
            js_name: "publicUrl",
            py_name: "public_url",
            category: "assets",
            args: vec![
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("path", ArgType::String),
                ArgSpec::new("options", ArgType::Json),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let workspace = parser.string()?;
                    let path = parser.string()?;
                    let options = parser.json()?;
                    let result = api.asset_public_url(&workspace, &path, options).await?;
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
        // assets.readBytes(workspace, path, property?)
        //   -> { base64, mime_type, size, name }
        //
        // The bytes of an asset's resource property (default `file`), read
        // with the CALLER's rights: the node is read like `nodes.get` (a node
        // the caller may not read is not found), and only then are its bytes
        // fetched. A mounted asset whose local copy expired is fetched first,
        // like `node.getResource('file')` does. One call for what took
        // `nodes.get` + `resources.getBinary(storage_key)` — which also makes it
        // reachable from the Rust SDK (`raisin_sdk::assets::read_bytes`).
        ApiMethodDescriptor {
            internal_name: "asset_read_bytes",
            js_name: "readBytes",
            py_name: "read_bytes",
            category: "assets",
            args: vec![
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("path", ArgType::String),
                ArgSpec::new("property", ArgType::OptionalString),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let workspace = parser.string()?;
                    let path = parser.string()?;
                    let property = parser
                        .optional_string()?
                        .unwrap_or_else(|| "file".to_string());
                    let result = read_bytes(api, &workspace, &path, &property).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
    ]
}

/// `assets.readBytes`: the resource property of a node read with the caller's
/// rights, then its bytes.
async fn read_bytes(
    api: Arc<dyn FunctionApi>,
    workspace: &str,
    path: &str,
    property: &str,
) -> Result<Value> {
    let not_found = || raisin_error::Error::NotFound(format!("Asset not found: {path}"));
    let node = api.node_get(workspace, path).await?.ok_or_else(not_found)?;
    let mut resource = node
        .get("properties")
        .and_then(|p| p.get(property))
        .cloned();
    let is_virtual = node
        .get("properties")
        .and_then(|p| p.get("__virtual"))
        .and_then(Value::as_bool)
        == Some(true);
    if resource.is_none() && property == "file" && is_virtual {
        api.asset_ensure_content(workspace, path).await?;
        resource = api
            .node_get(workspace, path)
            .await?
            .and_then(|n| n.get("properties").and_then(|p| p.get(property)).cloned());
    }
    let resource = resource.ok_or_else(|| {
        raisin_error::Error::NotFound(format!("{path} has no resource property '{property}'"))
    })?;
    let storage_key = storage_key_of(&resource, path, property)?;
    let base64 = api.resource_get_binary(&storage_key).await?;
    Ok(serde_json::json!({
        "base64": base64,
        "mime_type": resource.get("mime_type").cloned().unwrap_or(Value::Null),
        "size": resource.get("size").cloned().unwrap_or(Value::Null),
        "name": resource.get("name").cloned().unwrap_or(Value::Null),
    }))
}

/// The blob key of a stored resource property.
fn storage_key_of(resource: &Value, path: &str, property: &str) -> Result<String> {
    resource
        .get("metadata")
        .and_then(|m| m.get("storage_key"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            raisin_error::Error::Validation(format!(
                "property '{property}' of {path} is not a stored resource (no storage_key)"
            ))
        })
}

#[cfg(test)]
mod read_bytes_tests {
    use super::{read_bytes, storage_key_of};
    use crate::api::FunctionApi;
    use crate::api::MockFunctionApi;
    use serde_json::json;
    use std::sync::Arc;

    #[test]
    fn the_storage_key_comes_from_the_resource_metadata() {
        let file = json!({ "name": "sig.png", "metadata": { "storage_key": "uploads/t/sig.png" } });
        assert_eq!(
            storage_key_of(&file, "/sig.png", "file").unwrap(),
            "uploads/t/sig.png"
        );
        let not_a_resource = json!("just text");
        assert!(storage_key_of(&not_a_resource, "/x", "title").is_err());
    }

    /// The node is read first (with the caller's rights, as `nodes.get`); a
    /// node without the property yields a clear error, never bytes.
    #[tokio::test]
    async fn a_node_without_the_resource_is_an_error() {
        let api: Arc<dyn FunctionApi> = Arc::new(MockFunctionApi::new(json!({})));
        let err = read_bytes(api, "assets", "/page", "file")
            .await
            .expect_err("no file property");
        assert!(err.to_string().contains("no resource property"), "{err}");
    }
}
