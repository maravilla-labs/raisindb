// SPDX-License-Identifier: BSL-1.1

//! OCR bindings — read the text out of an image.
//!
//! # Why this exists next to `raisin.pdf.ocr`
//!
//! The same primitive has been reachable as `raisin.pdf.ocr` since before there
//! was an OCR feature to speak of, and it has never been PDF-specific: it sniffs
//! the format and takes PNG, JPEG, TIFF, BMP, WebP and GIF. A function author
//! looking for "OCR an image" had no reason to look under `pdf`, so the
//! capability was effectively undiscoverable.
//!
//! This is the same call under the name it should have had. `raisin.pdf.ocr`
//! keeps working and is not deprecated in code, because a rename would break
//! every tenant function already calling it — silently, since a missing binding
//! is a `TypeError` at the moment it runs, in a background job.
//!
//! # Local compute, so no `FunctionApi` hop
//!
//! Tesseract runs in this process. There is no storage read, no tenant scoping
//! and nothing to authorise, so the descriptor ignores the `api` handle
//! entirely and no trait method, callback, impl or mock is needed —
//! `pdf_ocr` is the precedent for the shape.

use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};

/// Get all OCR operation method descriptors
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // ocr.image(base64Data, options)
        ApiMethodDescriptor {
            internal_name: "ocr_image",
            js_name: "image",
            py_name: "image",
            category: "ocr",
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
                    Ok(InvokeResult::Json(
                        super::pdf::ocr_impl(&base64_data, options).await,
                    ))
                })
            },
        },
    ]
}
