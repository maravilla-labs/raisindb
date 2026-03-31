// SPDX-License-Identifier: BSL-1.1

//! Sync configuration type definitions.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Complete sync configuration for a package
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SyncConfig {
    /// Remote repository configuration
    #[serde(default)]
    pub remote: Option<RemoteConfig>,

    /// Default sync settings
    #[serde(default)]
    pub defaults: SyncDefaults,

    /// Filter rules (ordered, last match wins)
    #[serde(default)]
    pub filters: Vec<SyncFilter>,

    /// Path-specific conflict resolution overrides
    #[serde(default)]
    pub conflicts: HashMap<String, ConflictOverride>,

    /// Sync lifecycle hooks
    #[serde(default)]
    pub hooks: Option<SyncHooks>,
}

/// Remote repository connection settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteConfig {
    /// URL of the RaisinDB server
    pub url: String,

    /// Repository identifier
    pub repo_id: String,

    /// Branch to sync with (default: "main")
    #[serde(default = "default_branch")]
    pub branch: String,

    /// Tenant ID for multi-tenant deployments
    #[serde(default = "default_tenant")]
    pub tenant_id: String,

    /// Reference to auth credentials profile
    #[serde(default)]
    pub auth_profile: Option<String>,

    /// Custom headers for authentication
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

fn default_branch() -> String {
    "main".to_string()
}

fn default_tenant() -> String {
    "default".to_string()
}

/// Default sync settings applied to all paths
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncDefaults {
    /// Default sync mode for all paths
    #[serde(default)]
    pub mode: SyncMode,

    /// Default conflict resolution strategy
    #[serde(default)]
    pub on_conflict: ConflictStrategy,

    /// Whether to sync deletions (remove files that no longer exist in source)
    #[serde(default = "default_true")]
    pub sync_deletions: bool,

    /// Property-level merge strategy
    #[serde(default)]
    pub property_merge: PropertyMergeMode,
}

fn default_true() -> bool {
    true
}

impl Default for SyncDefaults {
    fn default() -> Self {
        Self {
            mode: SyncMode::default(),
            on_conflict: ConflictStrategy::default(),
            sync_deletions: true,
            property_merge: PropertyMergeMode::default(),
        }
    }
}

/// Sync mode determining how content is synchronized
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    /// Full replacement of target with source
    #[default]
    Replace,
    /// Combine source and target, keeping both
    Merge,
    /// Only apply changes, preserve unmodified content
    Update,
}

/// Sync direction determining which way content flows
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncDirection {
    /// Sync in both directions (default)
    #[default]
    Bidirectional,
    /// Local only, never sync to server
    LocalOnly,
    /// Server only, pull but never push
    ServerOnly,
    /// Push only, never pull from server
    PushOnly,
}

impl SyncDirection {
    /// Check if this direction allows pushing to server
    pub fn allows_push(&self) -> bool {
        matches!(self, SyncDirection::Bidirectional | SyncDirection::PushOnly)
    }

    /// Check if this direction allows pulling from server
    pub fn allows_pull(&self) -> bool {
        matches!(
            self,
            SyncDirection::Bidirectional | SyncDirection::ServerOnly
        )
    }
}

/// Conflict resolution strategy
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConflictStrategy {
    /// Prompt user for each conflict (interactive)
    #[default]
    Ask,
    /// Always use local version
    PreferLocal,
    /// Always use server version
    PreferServer,
    /// Use version with most recent timestamp
    PreferNewer,
    /// Create both versions with suffix
    KeepBoth,
    /// Attempt property-level merge
    MergeProperties,
    /// Stop sync on first conflict
    Abort,
}

/// Property merge mode for merge operations
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PropertyMergeMode {
    /// Shallow merge (top-level properties only)
    #[default]
    Shallow,
    /// Deep merge (recursive for nested objects)
    Deep,
}

/// Filter type
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FilterType {
    /// Normal sync filter
    #[default]
    Normal,
    /// Cleanup filter (remove orphaned paths)
    Cleanup,
}

/// A single sync filter rule (evaluated in order, last match wins)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncFilter {
    /// Root path for this filter
    pub root: String,

    /// Override sync mode for this filter
    #[serde(default)]
    pub mode: Option<SyncMode>,

    /// Override sync direction for this filter
    #[serde(default)]
    pub direction: Option<SyncDirection>,

    /// Filter type (normal or cleanup)
    #[serde(default, rename = "type")]
    pub filter_type: FilterType,

    /// Include patterns (relative to root, glob syntax)
    #[serde(default)]
    pub include: Vec<String>,

    /// Exclude patterns (relative to root, glob syntax)
    #[serde(default)]
    pub exclude: Vec<String>,

    /// Override conflict strategy for this filter
    #[serde(default)]
    pub on_conflict: Option<ConflictStrategy>,

    /// Property filtering configuration
    #[serde(default)]
    pub properties: Option<PropertyFilter>,
}

/// Property-level filtering for merge operations
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PropertyFilter {
    /// Only sync these properties (whitelist)
    #[serde(default)]
    pub include: Vec<String>,

    /// Never sync these properties (blacklist)
    #[serde(default)]
    pub exclude: Vec<String>,

    /// Keep local values for these properties, ignore server
    #[serde(default)]
    pub preserve_local: Vec<String>,

    /// Keep server values for these properties, ignore local
    #[serde(default)]
    pub preserve_server: Vec<String>,

    /// Merge strategy configuration
    #[serde(default)]
    pub merge_strategy: Option<PropertyMergeStrategy>,

    /// Keys to use for array merging by ID
    #[serde(default)]
    pub merge_keys: HashMap<String, String>,
}

/// Detailed property merge strategy
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PropertyMergeStrategy {
    /// How to merge arrays
    #[serde(default)]
    pub arrays: ArrayMergeMode,

    /// How to merge objects
    #[serde(default)]
    pub objects: ObjectMergeMode,

    /// How to merge scalar values
    #[serde(default)]
    pub scalars: ScalarMergeMode,
}

/// Array merge mode
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArrayMergeMode {
    /// Concatenate arrays
    #[default]
    Concat,
    /// Replace target array with source
    Replace,
    /// Combine unique values
    Unique,
    /// Merge by key field
    MergeByKey,
}

/// Object merge mode
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObjectMergeMode {
    /// Shallow merge (top-level keys)
    #[default]
    Shallow,
    /// Deep recursive merge
    Deep,
    /// Replace entire object
    Replace,
}

/// Scalar value merge mode
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScalarMergeMode {
    /// Prefer local value
    #[default]
    PreferLocal,
    /// Prefer server value
    PreferServer,
    /// Prefer newer value by timestamp
    PreferNewer,
}

/// Path-specific conflict resolution override
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictOverride {
    /// Conflict resolution strategy for this path
    pub strategy: ConflictStrategy,

    /// Whether to create backup before overwriting
    #[serde(default)]
    pub backup: bool,

    /// Array merge mode for this path
    #[serde(default)]
    pub merge_arrays: Option<ArrayMergeMode>,
}

/// Sync lifecycle hooks
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SyncHooks {
    /// Hooks to run before sync starts
    #[serde(default)]
    pub before_sync: Vec<String>,

    /// Hooks to run after sync completes
    #[serde(default)]
    pub after_sync: Vec<String>,

    /// Hooks to run when a conflict is detected
    #[serde(default)]
    pub on_conflict: Vec<String>,
}
