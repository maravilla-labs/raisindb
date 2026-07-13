// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Temp-file host bindings for the QuickJS runtime.
//!
//! These are the only `__raisin_internal.*` functions that CANNOT live in the
//! shared bindings registry: they bind a per-execution `TempFileManager`
//! (registry invokers only receive `Arc<dyn FunctionApi>`). They back the JS
//! `Resource` class (resize / toImage / getPageCount) in `api_wrapper.js`.
//!
//! Error convention (consumed by the `Resource` class): a failing call returns
//! an `"error:<message>"` prefixed string (`temp_pdfPageCount` returns `-1`).

use rquickjs::{Ctx, Function, Object};
use std::sync::Arc;

/// Register the per-execution temp-file API functions.
pub(super) fn register_temp_internal<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
) -> std::result::Result<(), rquickjs::Error> {
    use crate::runtime::temp::{PdfToImageOptions, ResizeOptions, TempFileManager};

    // Create a shared TempFileManager for this execution
    let exec_id = uuid::Uuid::new_v4().to_string();
    let temp_manager = Arc::new(TempFileManager::new(&exec_id).unwrap_or_else(|e| {
        tracing::error!(error = %e, "Failed to create TempFileManager");
        panic!("Failed to create TempFileManager: {}", e);
    }));

    // temp_createFromBase64
    let temp_create = temp_manager.clone();
    let temp_create_fn = Function::new(
        ctx.clone(),
        move |base64_data: String, mime_type: String, name: Option<String>| match temp_create
            .create_from_base64(&base64_data, &mime_type, name.as_deref())
        {
            Ok(handle) => handle,
            Err(e) => {
                tracing::error!(error = %e, "temp_createFromBase64 failed");
                format!("error:{}", e)
            }
        },
    )?;
    internal.set("temp_createFromBase64", temp_create_fn)?;

    // temp_resize
    let temp_resize = temp_manager.clone();
    let temp_resize_fn =
        Function::new(ctx.clone(), move |handle: String, options_json: String| {
            let options: ResizeOptions =
                serde_json::from_str(&options_json).unwrap_or(ResizeOptions {
                    max_width: None,
                    max_height: None,
                    quality: None,
                    format: None,
                });
            match temp_resize.resize_image(&handle, &options) {
                Ok(new_handle) => new_handle,
                Err(e) => {
                    tracing::error!(error = %e, "temp_resize failed");
                    format!("error:{}", e)
                }
            }
        })?;
    internal.set("temp_resize", temp_resize_fn)?;

    // temp_getBinary
    let temp_get = temp_manager.clone();
    let temp_get_fn = Function::new(ctx.clone(), move |handle: String| {
        match temp_get.get_binary(&handle) {
            Ok(base64_data) => base64_data,
            Err(e) => {
                tracing::error!(error = %e, "temp_getBinary failed");
                format!("error:{}", e)
            }
        }
    })?;
    internal.set("temp_getBinary", temp_get_fn)?;

    // temp_getMimeType
    let temp_mime = temp_manager.clone();
    let temp_mime_fn = Function::new(ctx.clone(), move |handle: String| {
        match temp_mime.get_mime_type(&handle) {
            Ok(mime_type) => mime_type,
            Err(e) => {
                tracing::error!(error = %e, "temp_getMimeType failed");
                format!("error:{}", e)
            }
        }
    })?;
    internal.set("temp_getMimeType", temp_mime_fn)?;

    // temp_pdfToImage
    let temp_pdf_to_image = temp_manager.clone();
    let temp_pdf_to_image_fn =
        Function::new(ctx.clone(), move |handle: String, options_json: String| {
            let options: PdfToImageOptions =
                serde_json::from_str(&options_json).unwrap_or_default();
            match temp_pdf_to_image.pdf_to_image(&handle, &options) {
                Ok(new_handle) => new_handle,
                Err(e) => {
                    tracing::error!(error = %e, "temp_pdfToImage failed");
                    format!("error:{}", e)
                }
            }
        })?;
    internal.set("temp_pdfToImage", temp_pdf_to_image_fn)?;

    // temp_pdfPageCount
    let temp_pdf_pages = temp_manager.clone();
    let temp_pdf_pages_fn = Function::new(ctx.clone(), move |handle: String| match temp_pdf_pages
        .pdf_page_count(&handle)
    {
        Ok(count) => count as i32,
        Err(e) => {
            tracing::error!(error = %e, "temp_pdfPageCount failed");
            -1
        }
    })?;
    internal.set("temp_pdfPageCount", temp_pdf_pages_fn)?;

    Ok(())
}
