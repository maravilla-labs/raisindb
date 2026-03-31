// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Tesseract checker implementation.

use crate::checker::{DependencyChecker, DependencyStatus, InstallInstructions};
use crate::platform::run_command;
use std::path::PathBuf;

use super::instructions::build_install_instructions;

/// Checker for Tesseract OCR installation.
///
/// Tesseract is used for OCR (Optical Character Recognition) to extract
/// text from scanned PDF documents and images.
pub struct TesseractChecker;

impl TesseractChecker {
    /// Minimum recommended version for full functionality.
    pub const MIN_VERSION: &'static str = "4.0.0";

    /// Try to find the tesseract executable path.
    pub(super) fn find_executable() -> Option<PathBuf> {
        which::which("tesseract").ok()
    }

    /// Parse version from tesseract --version output.
    ///
    /// Example output:
    /// ```text
    /// tesseract 5.3.3
    ///  leptonica-1.84.1
    ///   libgif 5.2.2 : libjpeg 9e : libpng 1.6.43 : libtiff 4.6.0 : zlib 1.3.1 : libwebp 1.4.0 : libopenjp2 2.5.2
    ///  Found NEON
    ///  Found libarchive 3.7.4 zlib/1.3.1 liblzma/5.6.2 bz2lib/1.0.8 liblz4/1.9.4 libzstd/1.5.6
    ///  Found libcurl/8.9.1 SecureTransport (LibreSSL/3.3.6) zlib/1.2.12 nghttp2/1.61.0
    /// ```
    pub(super) fn parse_version(output: &str) -> Option<String> {
        // Look for "tesseract X.Y.Z" pattern
        for line in output.lines() {
            let line = line.trim();
            if line.starts_with("tesseract ") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    return Some(parts[1].to_string());
                }
            }
        }
        None
    }

    /// Check if tessdata is available (language data files).
    pub(super) fn check_tessdata() -> bool {
        // Try TESSDATA_PREFIX environment variable
        if let Ok(prefix) = std::env::var("TESSDATA_PREFIX") {
            let path = PathBuf::from(prefix);
            if path.join("eng.traineddata").exists() {
                return true;
            }
        }

        // Common tessdata locations
        let common_paths = [
            "/usr/share/tesseract-ocr/5/tessdata",
            "/usr/share/tesseract-ocr/4.00/tessdata",
            "/usr/share/tessdata",
            "/usr/local/share/tessdata",
            "/opt/homebrew/share/tessdata",
            "/opt/local/share/tessdata",
            // Windows paths
            "C:\\Program Files\\Tesseract-OCR\\tessdata",
            "C:\\Program Files (x86)\\Tesseract-OCR\\tessdata",
        ];

        for path in &common_paths {
            let p = PathBuf::from(path);
            if p.join("eng.traineddata").exists() {
                return true;
            }
        }

        false
    }
}

impl DependencyChecker for TesseractChecker {
    fn name(&self) -> &str {
        "tesseract"
    }

    fn display_name(&self) -> &str {
        "Tesseract OCR"
    }

    fn description(&self) -> &str {
        "Open-source OCR engine for extracting text from scanned documents and images"
    }

    fn check(&self) -> DependencyStatus {
        // Check if executable exists
        let path = match Self::find_executable() {
            Some(p) => p,
            None => return DependencyStatus::NotInstalled,
        };

        // Get version
        let version = match run_command("tesseract", &["--version"]) {
            Ok(output) => match Self::parse_version(&output) {
                Some(v) => v,
                None => {
                    return DependencyStatus::Error("Could not parse tesseract version".to_string())
                }
            },
            Err(e) => return DependencyStatus::Error(format!("Failed to run tesseract: {}", e)),
        };

        // Check tessdata availability
        if !Self::check_tessdata() {
            return DependencyStatus::Error(
                "Tesseract is installed but language data (tessdata) not found. \
                 Set TESSDATA_PREFIX or install tesseract language packs."
                    .to_string(),
            );
        }

        DependencyStatus::Installed { version, path }
    }

    fn install_instructions(&self) -> InstallInstructions {
        build_install_instructions()
    }

    fn is_required(&self) -> bool {
        // Tesseract is optional - OCR features will be disabled if not present
        false
    }

    fn features_affected(&self) -> Vec<&str> {
        vec![
            "PDF OCR (text extraction from scanned documents)",
            "Image text extraction",
            "Automatic language detection in documents",
        ]
    }
}
