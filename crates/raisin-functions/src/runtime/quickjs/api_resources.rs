// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Resource API registration for the QuickJS runtime.
//!
//! Registers internal functions for resource operations:
//! binary retrieval, temp file management, image resize, PDF extraction,
//! OCR processing, and resource upload.

use rquickjs::{Ctx, Function, Object};
use std::sync::Arc;

use super::helpers::{json_error, run_async_blocking};
use crate::api::FunctionApi;

/// Register internal resource API functions.
pub(super) fn register_resources_internal<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    api: Arc<dyn FunctionApi>,
) -> std::result::Result<(), rquickjs::Error> {
    use crate::runtime::temp::{ResizeOptions, TempFileManager};
    use std::sync::Arc as StdArc;

    // Create a shared TempFileManager for this execution
    let exec_id = uuid::Uuid::new_v4().to_string();
    let temp_manager = StdArc::new(TempFileManager::new(&exec_id).unwrap_or_else(|e| {
        tracing::error!(error = %e, "Failed to create TempFileManager");
        panic!("Failed to create TempFileManager: {}", e);
    }));

    // resource_getBinary
    let api_get_binary = api.clone();
    let get_binary_fn = Function::new(ctx.clone(), move |storage_key: String| {
        let api = api_get_binary.clone();
        let result = run_async_blocking(async move { api.resource_get_binary(&storage_key).await });
        match result {
            Ok(base64_data) => base64_data,
            Err(e) => {
                tracing::error!(error = %e, "resource_get_binary failed");
                format!("error:{}", e)
            }
        }
    })?;
    internal.set("resource_getBinary", get_binary_fn)?;

    register_temp_file_ops(ctx, internal, &temp_manager)?;
    register_pdf_ops(ctx, internal, api.clone())?;

    // node_addResource
    let api_add_resource = api.clone();
    let add_resource_fn = Function::new(
        ctx.clone(),
        move |workspace: String,
              node_path: String,
              property_path: String,
              upload_data_json: String| {
            let api = api_add_resource.clone();
            let upload_data: serde_json::Value =
                serde_json::from_str(&upload_data_json).unwrap_or(serde_json::json!({}));
            let result = run_async_blocking(async move {
                api.node_add_resource(&workspace, &node_path, &property_path, upload_data)
                    .await
            });
            match result {
                Ok(v) => serde_json::to_string(&v)
                    .unwrap_or(r#"{"error":"serialization failed"}"#.to_string()),
                Err(e) => {
                    tracing::error!(error = %e, "node_add_resource failed");
                    json_error(&e)
                }
            }
        },
    )?;
    internal.set("node_addResource", add_resource_fn)?;

    Ok(())
}

/// Register temp file operations (create, resize, get binary/mime, PDF conversion).
fn register_temp_file_ops<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    temp_manager: &Arc<crate::runtime::temp::TempFileManager>,
) -> std::result::Result<(), rquickjs::Error> {
    use crate::runtime::temp::ResizeOptions;

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
            use crate::runtime::temp::PdfToImageOptions;
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

/// Register PDF-specific operations (text extraction, page count, OCR, storage processing).
fn register_pdf_ops<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    api: Arc<dyn FunctionApi>,
) -> std::result::Result<(), rquickjs::Error> {
    // pdf_extractText
    let pdf_extract_text_fn = Function::new(ctx.clone(), move |base64_data: String| {
        use base64::Engine;
        use raisin_ai::pdf::native::extract_text;

        let pdf_bytes = match base64::engine::general_purpose::STANDARD.decode(&base64_data) {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::error!(error = %e, "pdf_extractText: base64 decode failed");
                return serde_json::json!({
                    "error": format!("Failed to decode base64: {}", e)
                })
                .to_string();
            }
        };

        match extract_text(&pdf_bytes) {
            Ok(result) => {
                let pages: Vec<serde_json::Value> = result
                    .pages
                    .iter()
                    .map(|p| {
                        serde_json::json!({
                            "index": p.index,
                            "text": p.text,
                            "charCount": p.char_count
                        })
                    })
                    .collect();

                serde_json::json!({
                    "text": result.full_text,
                    "pages": pages,
                    "pageCount": result.page_count,
                    "isScanned": result.is_likely_scanned
                })
                .to_string()
            }
            Err(e) => {
                tracing::error!(error = %e, "pdf_extractText failed");
                serde_json::json!({
                    "error": format!("PDF extraction failed: {}", e)
                })
                .to_string()
            }
        }
    })?;
    internal.set("pdf_extractText", pdf_extract_text_fn)?;

    // pdf_getPageCount
    let pdf_get_page_count_fn = Function::new(ctx.clone(), move |base64_data: String| {
        use base64::Engine;
        use raisin_ai::pdf::native::get_page_count;

        let pdf_bytes = match base64::engine::general_purpose::STANDARD.decode(&base64_data) {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::error!(error = %e, "pdf_getPageCount: base64 decode failed");
                return -1;
            }
        };

        match get_page_count(&pdf_bytes) {
            Ok(count) => count as i32,
            Err(e) => {
                tracing::error!(error = %e, "pdf_getPageCount failed");
                -1
            }
        }
    })?;
    internal.set("pdf_getPageCount", pdf_get_page_count_fn)?;

    // pdf_ocr
    let pdf_ocr_fn = Function::new(
        ctx.clone(),
        move |base64_data: String, options_json: String| {
            use base64::Engine;
            use raisin_ai::pdf::{get_default_ocr_provider, OcrOptions};

            let options: serde_json::Value =
                serde_json::from_str(&options_json).unwrap_or(serde_json::json!({}));

            let ocr_options = OcrOptions {
                languages: options
                    .get("languages")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                dpi: options
                    .get("dpi")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32),
                preserve_layout: options
                    .get("preserveLayout")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            };

            let provider = get_default_ocr_provider();
            let is_available = run_async_blocking(async { provider.is_available().await });

            if !is_available {
                return serde_json::json!({
                    "text": "",
                    "available": false,
                    "error": "OCR is not available. Tesseract may not be installed."
                })
                .to_string();
            }

            let image_bytes = match base64::engine::general_purpose::STANDARD.decode(&base64_data) {
                Ok(bytes) => bytes,
                Err(e) => {
                    tracing::error!(error = %e, "pdf_ocr: base64 decode failed");
                    return serde_json::json!({
                        "error": format!("Failed to decode base64: {}", e)
                    })
                    .to_string();
                }
            };

            let result =
                run_async_blocking(
                    async move { provider.ocr_image(&image_bytes, &ocr_options).await },
                );

            match result {
                Ok(text) => serde_json::json!({
                    "text": text,
                    "available": true
                })
                .to_string(),
                Err(e) => {
                    tracing::error!(error = %e, "pdf_ocr failed");
                    serde_json::json!({
                        "error": format!("OCR failed: {}", e),
                        "available": true
                    })
                    .to_string()
                }
            }
        },
    )?;
    internal.set("pdf_ocr", pdf_ocr_fn)?;

    // pdf_processFromStorage
    let api_pdf_process = api.clone();
    let pdf_process_fn = Function::new(
        ctx.clone(),
        move |storage_key: String, options_json: String| {
            let api = api_pdf_process.clone();
            let options: serde_json::Value =
                serde_json::from_str(&options_json).unwrap_or(serde_json::json!({}));

            let result = run_async_blocking(async move {
                api.pdf_process_from_storage(&storage_key, options).await
            });

            match result {
                Ok(v) => serde_json::to_string(&v)
                    .unwrap_or(r#"{"error":"serialization failed"}"#.to_string()),
                Err(e) => {
                    tracing::error!(error = %e, "pdf_processFromStorage failed");
                    serde_json::json!({
                        "error": format!("PDF processing failed: {}", e)
                    })
                    .to_string()
                }
            }
        },
    )?;
    internal.set("pdf_processFromStorage", pdf_process_fn)?;

    Ok(())
}
