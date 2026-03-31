// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! External dependency management for RaisinDB.
//!
//! This crate provides infrastructure for checking, installing, and managing
//! external system dependencies like Tesseract OCR, FFmpeg, etc.
//!
//! # Features
//!
//! - **Dependency checking**: Detect if tools are installed and get versions
//! - **Platform detection**: macOS, Windows, Linux (Debian/Fedora/Arch)
//! - **Interactive setup**: CLI prompts for installation guidance
//! - **State persistence**: Remember setup decisions across restarts
//!
//! # Usage
//!
//! ```rust,ignore
//! use raisin_deps::{ExternalDependencyManager, TesseractChecker};
//!
//! let manager = ExternalDependencyManager::new()
//!     .register(TesseractChecker);
//!
//! let results = manager.check_all();
//! for result in &results {
//!     println!("{}: {:?}", result.name, result.status);
//! }
//! ```

pub mod checker;
pub mod flags;
pub mod manager;
pub mod platform;
pub mod state;
pub mod tools;
pub mod ui;

// Re-export commonly used types
pub use checker::{DependencyChecker, DependencyResult, DependencyStatus, InstallInstructions};
pub use flags::DEPENDENCY_FLAGS;
pub use manager::{EnableResult, ExternalDependencyManager, SetupError, SetupResult};
pub use platform::{LinuxDistro, Platform};
pub use state::{DependencyRecord, DependencySetupState, SetupStatus};
pub use tools::TesseractChecker;
pub use ui::{DependencySetupUI, InstallAction};
