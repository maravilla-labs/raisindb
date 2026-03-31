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

//! Temporary file manager for function execution.
//!
//! This module provides temporary file management for image processing operations
//! during function execution. It handles:
//! - Creating temp files from base64 data
//! - Resizing images using the `image` crate (pure Rust)
//! - Getting binary data from temp handles
//! - Cleanup when execution ends

use base64::Engine;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::webp::WebPEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat};
use raisin_error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Options for image resize operations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResizeOptions {
    /// Maximum width (maintains aspect ratio if only width specified)
    pub max_width: Option<u32>,
    /// Maximum height (maintains aspect ratio if only height specified)
    pub max_height: Option<u32>,
    /// Output quality (1-100, for JPEG/WebP)
    pub quality: Option<u32>,
    /// Output format (e.g., "jpeg", "png", "webp")
    pub format: Option<String>,
}

/// Options for PDF to image conversion
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfToImageOptions {
    /// Page index to render (0-based, default: 0)
    pub page: Option<usize>,
    /// Maximum width in pixels (maintains aspect ratio)
    pub max_width: Option<u32>,
    /// Maximum height in pixels (maintains aspect ratio)
    pub max_height: Option<u32>,
    /// Output format ("jpeg", "png", "webp", default: "jpeg")
    pub format: Option<String>,
    /// JPEG quality 1-100 (default: 85)
    pub quality: Option<u32>,
}

/// Information about a temp file
#[derive(Debug, Clone)]
struct TempFileInfo {
    path: PathBuf,
    mime_type: String,
    original_name: Option<String>,
}

/// Manages temporary files for a single function execution.
///
/// Each execution gets its own temp directory that is cleaned up
/// when the manager is dropped.
pub struct TempFileManager {
    /// Unique execution ID
    exec_id: String,
    /// Base temp directory for this execution
    temp_dir: PathBuf,
    /// Map of handle -> file info
    handles: Mutex<HashMap<String, TempFileInfo>>,
}

impl TempFileManager {
    /// Create a new TempFileManager for an execution.
    pub fn new(exec_id: &str) -> Result<Self> {
        let temp_base = std::env::temp_dir().join("raisin-functions");
        let temp_dir = temp_base.join(exec_id);

        // Create the temp directory
        std::fs::create_dir_all(&temp_dir)
            .map_err(|e| Error::Backend(format!("Failed to create temp directory: {}", e)))?;

        Ok(Self {
            exec_id: exec_id.to_string(),
            temp_dir,
            handles: Mutex::new(HashMap::new()),
        })
    }

    /// Create a temp file from base64 data, return handle.
    pub fn create_from_base64(
        &self,
        data: &str,
        mime_type: &str,
        original_name: Option<&str>,
    ) -> Result<String> {
        // Decode base64
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| Error::Validation(format!("Invalid base64 data: {}", e)))?;

        // Generate handle and filename
        let handle = format!("temp-{}", Uuid::new_v4());
        let ext = mime_type_to_extension(mime_type);
        let filename = format!("{}.{}", handle, ext);
        let path = self.temp_dir.join(&filename);

        // Write file
        std::fs::write(&path, &bytes)
            .map_err(|e| Error::Backend(format!("Failed to write temp file: {}", e)))?;

        // Store info
        let mut handles = self.handles.lock().unwrap();
        handles.insert(
            handle.clone(),
            TempFileInfo {
                path,
                mime_type: mime_type.to_string(),
                original_name: original_name.map(|s| s.to_string()),
            },
        );

        Ok(handle)
    }

    /// Resize an image using the `image` crate (pure Rust), return new handle.
    pub fn resize_image(&self, handle: &str, options: &ResizeOptions) -> Result<String> {
        let handles = self.handles.lock().unwrap();
        let info = handles
            .get(handle)
            .ok_or_else(|| Error::NotFound(format!("Temp handle not found: {}", handle)))?;

        let input_path = info.path.clone();
        let original_name = info.original_name.clone();
        let input_mime = info.mime_type.clone();
        drop(handles); // Release lock before IO

        // Load the image
        let img = image::open(&input_path)
            .map_err(|e| Error::Backend(format!("Failed to load image: {}", e)))?;

        // Calculate new dimensions
        let (orig_width, orig_height) = (img.width(), img.height());
        let (new_width, new_height) = calculate_dimensions(
            orig_width,
            orig_height,
            options.max_width,
            options.max_height,
        );

        // Resize if dimensions changed
        let resized = if new_width != orig_width || new_height != orig_height {
            img.resize(new_width, new_height, FilterType::Lanczos3)
        } else {
            img
        };

        // Determine output format
        let output_format = options
            .format
            .as_deref()
            .unwrap_or_else(|| mime_to_format(&input_mime));
        let output_ext = output_format;
        let output_mime = extension_to_mime_type(output_ext);

        // Generate new handle and path
        let new_handle = format!("temp-{}", Uuid::new_v4());
        let output_filename = format!("{}.{}", new_handle, output_ext);
        let output_path = self.temp_dir.join(&output_filename);

        // Encode and save the image
        let quality = options.quality.unwrap_or(85);
        encode_image(&resized, &output_path, output_format, quality)?;

        // Store new handle
        let mut handles = self.handles.lock().unwrap();
        handles.insert(
            new_handle.clone(),
            TempFileInfo {
                path: output_path,
                mime_type: output_mime.to_string(),
                original_name,
            },
        );

        Ok(new_handle)
    }

    /// Get base64 data from a temp handle.
    pub fn get_binary(&self, handle: &str) -> Result<String> {
        let handles = self.handles.lock().unwrap();
        let info = handles
            .get(handle)
            .ok_or_else(|| Error::NotFound(format!("Temp handle not found: {}", handle)))?;

        let path = info.path.clone();
        drop(handles);

        let bytes = std::fs::read(&path)
            .map_err(|e| Error::Backend(format!("Failed to read temp file: {}", e)))?;

        Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
    }

    /// Get the mime type for a temp handle.
    pub fn get_mime_type(&self, handle: &str) -> Result<String> {
        let handles = self.handles.lock().unwrap();
        let info = handles
            .get(handle)
            .ok_or_else(|| Error::NotFound(format!("Temp handle not found: {}", handle)))?;
        Ok(info.mime_type.clone())
    }

    /// Get the original name for a temp handle.
    pub fn get_original_name(&self, handle: &str) -> Result<Option<String>> {
        let handles = self.handles.lock().unwrap();
        let info = handles
            .get(handle)
            .ok_or_else(|| Error::NotFound(format!("Temp handle not found: {}", handle)))?;
        Ok(info.original_name.clone())
    }

    /// Convert a PDF page to an image, return new handle.
    ///
    /// NOTE: PDF rendering has been removed. Use PDF text extraction via
    /// `pdf_page_count` and text extraction instead.
    ///
    /// Options:
    /// - `page`: Page index (0-based, default: 0)
    /// - `max_width`: Maximum width in pixels
    /// - `format`: Output format ("jpeg", "png", default: "jpeg")
    /// - `quality`: JPEG quality 1-100 (default: 85)
    pub fn pdf_to_image(&self, handle: &str, _options: &PdfToImageOptions) -> Result<String> {
        let handles = self.handles.lock().unwrap();
        let info = handles
            .get(handle)
            .ok_or_else(|| Error::NotFound(format!("Temp handle not found: {}", handle)))?;

        // Verify it's a PDF
        if info.mime_type != "application/pdf" {
            return Err(Error::Validation(format!(
                "Expected PDF, got: {}",
                info.mime_type
            )));
        }

        drop(handles);

        // PDF rendering has been removed - we now use pdf_oxide for text extraction only
        Err(Error::Backend(
            "PDF to image conversion is not available. Use text extraction instead.".to_string(),
        ))
    }

    /// Get page count from a PDF.
    #[cfg(feature = "pdf-markdown")]
    pub fn pdf_page_count(&self, handle: &str) -> Result<usize> {
        use pdf_oxide::PdfDocument;

        let handles = self.handles.lock().unwrap();
        let info = handles
            .get(handle)
            .ok_or_else(|| Error::NotFound(format!("Temp handle not found: {}", handle)))?;

        // Verify it's a PDF
        if info.mime_type != "application/pdf" {
            return Err(Error::Validation(format!(
                "Expected PDF, got: {}",
                info.mime_type
            )));
        }

        let input_path = info.path.clone();
        drop(handles);

        // Open PDF with pdf_oxide and get page count
        let doc = PdfDocument::open(&input_path)
            .map_err(|e| Error::Backend(format!("Failed to open PDF: {}", e)))?;

        doc.page_count()
            .map_err(|e| Error::Backend(format!("Failed to get PDF page count: {}", e)))
    }

    /// Stub when pdf-markdown feature is not enabled.
    #[cfg(not(feature = "pdf-markdown"))]
    pub fn pdf_page_count(&self, _handle: &str) -> Result<usize> {
        Err(Error::Backend(
            "PDF processing requires pdf-markdown feature".to_string(),
        ))
    }

    /// Cleanup all temp files.
    pub fn cleanup(&self) {
        if let Err(e) = std::fs::remove_dir_all(&self.temp_dir) {
            tracing::warn!(
                exec_id = %self.exec_id,
                error = %e,
                "Failed to cleanup temp directory"
            );
        }
    }
}

impl Drop for TempFileManager {
    fn drop(&mut self) {
        self.cleanup();
    }
}

/// Thread-safe wrapper for TempFileManager
pub type SharedTempFileManager = Arc<TempFileManager>;

/// Convert MIME type to file extension
fn mime_type_to_extension(mime_type: &str) -> &str {
    match mime_type {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "image/bmp" => "bmp",
        "image/tiff" => "tiff",
        "application/pdf" => "pdf",
        _ => "bin",
    }
}

/// Convert file extension to MIME type
fn extension_to_mime_type(ext: &str) -> &str {
    match ext.to_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "tiff" | "tif" => "image/tiff",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// Convert MIME type to format string for output
fn mime_to_format(mime: &str) -> &str {
    match mime {
        "image/jpeg" => "jpeg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        _ => "jpeg", // Default to JPEG for unknown formats
    }
}

/// Calculate new dimensions maintaining aspect ratio
fn calculate_dimensions(
    orig_width: u32,
    orig_height: u32,
    max_width: Option<u32>,
    max_height: Option<u32>,
) -> (u32, u32) {
    match (max_width, max_height) {
        (Some(max_w), Some(max_h)) => {
            // Fit within both constraints
            let width_ratio = max_w as f64 / orig_width as f64;
            let height_ratio = max_h as f64 / orig_height as f64;
            let ratio = width_ratio.min(height_ratio).min(1.0); // Don't upscale
            (
                (orig_width as f64 * ratio) as u32,
                (orig_height as f64 * ratio) as u32,
            )
        }
        (Some(max_w), None) => {
            // Only width constraint
            if orig_width <= max_w {
                (orig_width, orig_height)
            } else {
                let ratio = max_w as f64 / orig_width as f64;
                (max_w, (orig_height as f64 * ratio) as u32)
            }
        }
        (None, Some(max_h)) => {
            // Only height constraint
            if orig_height <= max_h {
                (orig_width, orig_height)
            } else {
                let ratio = max_h as f64 / orig_height as f64;
                ((orig_width as f64 * ratio) as u32, max_h)
            }
        }
        (None, None) => (orig_width, orig_height),
    }
}

/// Encode and save image to file
fn encode_image(
    img: &DynamicImage,
    output_path: &PathBuf,
    format: &str,
    quality: u32,
) -> Result<()> {
    let mut output_file = std::fs::File::create(output_path)
        .map_err(|e| Error::Backend(format!("Failed to create output file: {}", e)))?;

    match format.to_lowercase().as_str() {
        "jpeg" | "jpg" => {
            let encoder = JpegEncoder::new_with_quality(&mut output_file, quality as u8);
            img.to_rgb8()
                .write_with_encoder(encoder)
                .map_err(|e| Error::Backend(format!("Failed to encode JPEG: {}", e)))?;
        }
        "png" => {
            let encoder = PngEncoder::new(&mut output_file);
            img.to_rgba8()
                .write_with_encoder(encoder)
                .map_err(|e| Error::Backend(format!("Failed to encode PNG: {}", e)))?;
        }
        "webp" => {
            let encoder = WebPEncoder::new_lossless(&mut output_file);
            img.to_rgba8()
                .write_with_encoder(encoder)
                .map_err(|e| Error::Backend(format!("Failed to encode WebP: {}", e)))?;
        }
        "gif" => {
            img.save_with_format(output_path, ImageFormat::Gif)
                .map_err(|e| Error::Backend(format!("Failed to encode GIF: {}", e)))?;
        }
        "bmp" => {
            img.save_with_format(output_path, ImageFormat::Bmp)
                .map_err(|e| Error::Backend(format!("Failed to encode BMP: {}", e)))?;
        }
        _ => {
            // Default to JPEG
            let encoder = JpegEncoder::new_with_quality(&mut output_file, quality as u8);
            img.to_rgb8()
                .write_with_encoder(encoder)
                .map_err(|e| Error::Backend(format!("Failed to encode image: {}", e)))?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_temp_file_manager_create_and_read() {
        let manager = TempFileManager::new("test-exec-1").unwrap();

        // Create a temp file from base64
        let original_data = b"Hello, World!";
        let base64_data = base64::engine::general_purpose::STANDARD.encode(original_data);

        let handle = manager
            .create_from_base64(&base64_data, "text/plain", Some("hello.txt"))
            .unwrap();

        assert!(handle.starts_with("temp-"));

        // Read it back
        let read_back = manager.get_binary(&handle).unwrap();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&read_back)
            .unwrap();

        assert_eq!(decoded, original_data);

        // Check mime type
        assert_eq!(manager.get_mime_type(&handle).unwrap(), "text/plain");

        // Cleanup happens on drop
    }

    #[test]
    fn test_mime_type_to_extension() {
        assert_eq!(mime_type_to_extension("image/jpeg"), "jpg");
        assert_eq!(mime_type_to_extension("image/png"), "png");
        assert_eq!(mime_type_to_extension("image/webp"), "webp");
        assert_eq!(mime_type_to_extension("unknown/type"), "bin");
    }
}
