// TODO(v0.2): Constants reserved for future operations
#![allow(dead_code)]

//! Common constants used throughout the raisin-rocksdb crate
//!
//! This module provides centralized string constants to avoid repeated
//! String allocations via `.to_string()` calls.

/// Root-level parent ID for top-level nodes in the hierarchy
pub const ROOT_PARENT_ID: &str = "/";

/// System actor identifier for automated/background operations
pub const SYSTEM_ACTOR: &str = "system";

/// Main branch identifier for system operations
pub const MAIN_BRANCH: &str = "main";
