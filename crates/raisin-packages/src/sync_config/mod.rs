// SPDX-License-Identifier: BSL-1.1

//! Sync configuration for bidirectional package synchronization
//!
//! This module provides filter configuration
//! for fine-grained control over package synchronization between local development
//! directories and the RaisinDB server.

mod filter;
mod types;

#[cfg(test)]
mod tests;

pub use types::{
    ArrayMergeMode, ConflictOverride, ConflictStrategy, FilterType, ObjectMergeMode,
    PropertyFilter, PropertyMergeMode, PropertyMergeStrategy, RemoteConfig, ScalarMergeMode,
    SyncConfig, SyncDefaults, SyncDirection, SyncFilter, SyncHooks, SyncMode,
};
