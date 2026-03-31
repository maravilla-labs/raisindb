// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Tesseract OCR dependency checker.
//!
//! This module provides functionality to check for Tesseract OCR installation
//! and generate platform-specific installation instructions.

mod checker;
mod instructions;

pub use checker::TesseractChecker;

#[cfg(test)]
mod tests;
