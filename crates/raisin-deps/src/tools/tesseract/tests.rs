// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Tests for Tesseract checker.

use super::*;
use crate::checker::DependencyChecker;

#[test]
fn test_parse_version() {
    let output = r#"tesseract 5.3.3
 leptonica-1.84.1
  libgif 5.2.2 : libjpeg 9e : libpng 1.6.43 : libtiff 4.6.0 : zlib 1.3.1 : libwebp 1.4.0 : libopenjp2 2.5.2
 Found NEON
 Found libarchive 3.7.4 zlib/1.3.1 liblzma/5.6.2 bz2lib/1.0.8 liblz4/1.9.4 libzstd/1.5.6
 Found libcurl/8.9.1 SecureTransport (LibreSSL/3.3.6) zlib/1.2.12 nghttp2/1.61.0"#;

    let version = TesseractChecker::parse_version(output);
    assert_eq!(version, Some("5.3.3".to_string()));
}

#[test]
fn test_parse_version_old_format() {
    let output = "tesseract 4.1.1";
    let version = TesseractChecker::parse_version(output);
    assert_eq!(version, Some("4.1.1".to_string()));
}

#[test]
fn test_checker_basic() {
    let checker = TesseractChecker;

    assert_eq!(checker.name(), "tesseract");
    assert_eq!(checker.display_name(), "Tesseract OCR");
    assert!(!checker.is_required());
    assert!(!checker.features_affected().is_empty());
}

#[test]
fn test_install_instructions_not_empty() {
    let checker = TesseractChecker;
    let instructions = checker.install_instructions();

    // Should always have a command
    assert!(!instructions.command.is_empty());
    // Should always have a manual URL
    assert!(instructions.manual_url.is_some());
    // Should list what it provides
    assert!(!instructions.provides.is_empty());
}
