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
/// `processFromStorage` goes through `FunctionApi`. The base64-based
/// operations (`extractText`, `getPageCount`, `ocr`) are pure local compute
/// (raisin-ai) — their invokers ignore the `api` handle but live here so both
/// runtimes share the single definition.
///
/// Error shapes (load-bearing for the QuickJS wrapper):
/// - `pdf_extractText` / `pdf_ocr` hard failures return an
///   `Ok({"error":true,"message":"..."})` envelope (message texts match the
///   pre-registry host functions).
/// - `pdf_ocr` when OCR is UNAVAILABLE returns DATA
///   (`{"text":"","available":false,"error":"..."}`), not an error envelope —
///   callers branch on `available`.
/// - `pdf_getPageCount` returns `-1` on failure (never errors).
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
        // pdf.extractText(base64Data) -> { text, pages, pageCount, isScanned }
        ApiMethodDescriptor {
            internal_name: "pdf_extractText",
            js_name: "extractText",
            py_name: "extract_text",
            category: "pdf",
            args: vec![ArgSpec::new("base64Data", ArgType::String)],
            return_type: ReturnType::Json,
            invoker: |_api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let base64_data = parser.string()?;
                    Ok(InvokeResult::Json(extract_text_impl(&base64_data)))
                })
            },
        },
        // pdf.getPageCount(base64Data) -> page count (-1 on failure)
        ApiMethodDescriptor {
            internal_name: "pdf_getPageCount",
            js_name: "getPageCount",
            py_name: "get_page_count",
            category: "pdf",
            args: vec![ArgSpec::new("base64Data", ArgType::String)],
            return_type: ReturnType::I64,
            invoker: |_api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let base64_data = parser.string()?;
                    Ok(InvokeResult::I64(get_page_count_impl(&base64_data)))
                })
            },
        },
        // pdf.ocr(base64Data, options) -> { text, available } | availability data
        ApiMethodDescriptor {
            internal_name: "pdf_ocr",
            js_name: "ocr",
            py_name: "ocr",
            category: "pdf",
            args: vec![
                ArgSpec::new("base64Data", ArgType::String),
                ArgSpec::new("options", ArgType::Json),
            ],
            return_type: ReturnType::Json,
            invoker: |_api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let base64_data = parser.string()?;
                    let options = parser.json()?;
                    Ok(InvokeResult::Json(ocr_impl(&base64_data, options).await))
                })
            },
        },
    ]
}

/// Error envelope matching the gateway convention (`{"error":true,"message"}`).
fn error_value(message: String) -> Value {
    serde_json::json!({ "error": true, "message": message })
}

/// Extract text from a base64-encoded PDF (local compute, no FunctionApi).
fn extract_text_impl(base64_data: &str) -> Value {
    use base64::Engine;
    use raisin_ai::pdf::native::extract_text;

    let pdf_bytes = match base64::engine::general_purpose::STANDARD.decode(base64_data) {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!(error = %e, "pdf_extractText: base64 decode failed");
            return error_value(format!("Failed to decode base64: {}", e));
        }
    };

    match extract_text(&pdf_bytes) {
        Ok(result) => {
            let pages: Vec<Value> = result
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
        }
        Err(e) => {
            tracing::error!(error = %e, "pdf_extractText failed");
            error_value(format!("PDF extraction failed: {}", e))
        }
    }
}

/// Get the page count of a base64-encoded PDF; `-1` on any failure.
fn get_page_count_impl(base64_data: &str) -> i64 {
    use base64::Engine;
    use raisin_ai::pdf::native::get_page_count;

    let pdf_bytes = match base64::engine::general_purpose::STANDARD.decode(base64_data) {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!(error = %e, "pdf_getPageCount: base64 decode failed");
            return -1;
        }
    };

    match get_page_count(&pdf_bytes) {
        Ok(count) => count as i64,
        Err(e) => {
            tracing::error!(error = %e, "pdf_getPageCount failed");
            -1
        }
    }
}

/// OCR a base64-encoded image via the default provider (Tesseract).
async fn ocr_impl(base64_data: &str, options: Value) -> Value {
    use base64::Engine;
    use raisin_ai::pdf::{get_default_ocr_provider, OcrOptions};

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
    if !provider.is_available().await {
        // Availability is DATA, not an error: callers branch on `available`.
        return serde_json::json!({
            "text": "",
            "available": false,
            "error": "OCR is not available. Tesseract may not be installed."
        });
    }

    let image_bytes = match base64::engine::general_purpose::STANDARD.decode(base64_data) {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!(error = %e, "pdf_ocr: base64 decode failed");
            return error_value(format!("Failed to decode base64: {}", e));
        }
    };

    match provider.ocr_image(&image_bytes, &ocr_options).await {
        Ok(text) => serde_json::json!({
            "text": text,
            "available": true
        }),
        Err(e) => {
            tracing::error!(error = %e, "pdf_ocr failed");
            let mut envelope = error_value(format!("OCR failed: {}", e));
            if let Some(obj) = envelope.as_object_mut() {
                obj.insert("available".to_string(), Value::Bool(true));
            }
            envelope
        }
    }
}
