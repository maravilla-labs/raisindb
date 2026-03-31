// SPDX-License-Identifier: BSL-1.1

//! Sync status types for package synchronization
//!
//! This module provides types for tracking the synchronization status
//! between installed package content and the package source.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Sync status for a single file
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncFileStatus {
    /// File is identical on local and server
    Synced,
    /// File exists only locally (added after install)
    LocalOnly,
    /// File exists only in package (deleted locally)
    ServerOnly,
    /// File modified locally (different from package source)
    Modified,
    /// Both sides modified since last sync (conflict)
    Conflict,
}

impl SyncFileStatus {
    /// Check if this status requires attention
    pub fn needs_action(&self) -> bool {
        !matches!(self, SyncFileStatus::Synced)
    }

    /// Check if this is a conflict
    pub fn is_conflict(&self) -> bool {
        matches!(self, SyncFileStatus::Conflict)
    }
}

/// File sync information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncFileInfo {
    /// Path relative to package root
    pub path: String,

    /// Current sync status
    pub status: SyncFileStatus,

    /// Workspace containing this file
    pub workspace: String,

    /// Hash of local content (if exists)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_hash: Option<String>,

    /// Hash of server/package content (if exists)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_hash: Option<String>,

    /// Local modification timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_modified_at: Option<DateTime<Utc>>,

    /// Server modification timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_modified_at: Option<DateTime<Utc>>,

    /// Node type of the file (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_type: Option<String>,
}

/// Summary of sync status counts
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncSummary {
    /// Number of files in sync
    pub synced: u32,
    /// Number of locally modified files
    pub modified: u32,
    /// Number of files that exist only locally
    pub local_only: u32,
    /// Number of files that exist only on server
    pub server_only: u32,
    /// Number of files with conflicts
    pub conflict: u32,
}

impl SyncSummary {
    /// Create a new empty summary
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a file status to the summary
    pub fn add(&mut self, status: SyncFileStatus) {
        match status {
            SyncFileStatus::Synced => self.synced += 1,
            SyncFileStatus::Modified => self.modified += 1,
            SyncFileStatus::LocalOnly => self.local_only += 1,
            SyncFileStatus::ServerOnly => self.server_only += 1,
            SyncFileStatus::Conflict => self.conflict += 1,
        }
    }

    /// Get total number of files
    pub fn total(&self) -> u32 {
        self.synced + self.modified + self.local_only + self.server_only + self.conflict
    }

    /// Check if there are any issues requiring attention
    pub fn has_issues(&self) -> bool {
        self.modified > 0 || self.local_only > 0 || self.server_only > 0 || self.conflict > 0
    }

    /// Check if there are any conflicts
    pub fn has_conflicts(&self) -> bool {
        self.conflict > 0
    }
}

/// Overall package sync status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageSyncStatus {
    /// Package name
    pub package_name: String,

    /// Package version
    pub package_version: String,

    /// When the package was installed
    pub installed_at: DateTime<Utc>,

    /// When sync status was last checked
    pub last_sync_check: DateTime<Utc>,

    /// Overall status
    pub status: OverallSyncStatus,

    /// Per-file sync information
    pub files: Vec<SyncFileInfo>,

    /// Summary counts
    pub summary: SyncSummary,
}

/// Overall sync status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverallSyncStatus {
    /// All files are in sync
    Synced,
    /// Some files have been modified
    Modified,
    /// There are conflicts that need resolution
    Conflict,
}

impl From<&SyncSummary> for OverallSyncStatus {
    fn from(summary: &SyncSummary) -> Self {
        if summary.conflict > 0 {
            OverallSyncStatus::Conflict
        } else if summary.has_issues() {
            OverallSyncStatus::Modified
        } else {
            OverallSyncStatus::Synced
        }
    }
}

/// File difference for conflict resolution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    /// Path to the file
    pub path: String,

    /// Type of diff (text or binary)
    pub diff_type: DiffType,

    /// Local content (if text and readable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_content: Option<String>,

    /// Server content (if text and readable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_content: Option<String>,

    /// Unified diff string (if text)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unified_diff: Option<String>,
}

/// Type of diff content
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffType {
    /// Text content that can be diffed line by line
    Text,
    /// Binary content that cannot be diffed
    Binary,
}

/// Export options for package export
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportOptions {
    /// Export mode
    pub export_mode: ExportMode,

    /// Filter patterns to apply (glob patterns)
    #[serde(default)]
    pub filter_patterns: Vec<String>,

    /// Whether to include local modifications
    #[serde(default = "default_true")]
    pub include_modifications: bool,

    /// Optional new version for the exported package
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_version: Option<String>,
}

fn default_true() -> bool {
    true
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            export_mode: ExportMode::Filtered,
            filter_patterns: Vec::new(),
            include_modifications: true,
            new_version: None,
        }
    }
}

/// Export mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExportMode {
    /// Export all content
    All,
    /// Apply manifest filters
    #[default]
    Filtered,
}

/// Result of a sync operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResult {
    /// Files that were uploaded to server
    pub uploaded: Vec<String>,

    /// Files that were downloaded from server
    pub downloaded: Vec<String>,

    /// Files that were skipped due to conflicts
    pub conflicts: Vec<SyncFileInfo>,

    /// Files that were skipped for other reasons
    pub skipped: Vec<String>,

    /// Errors encountered during sync
    pub errors: Vec<SyncError>,
}

/// Error during sync operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncError {
    /// Path that caused the error
    pub path: String,

    /// Error message
    pub error: String,
}

impl SyncResult {
    /// Create a new empty sync result
    pub fn new() -> Self {
        Self {
            uploaded: Vec::new(),
            downloaded: Vec::new(),
            conflicts: Vec::new(),
            skipped: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// Check if sync was successful (no errors or conflicts)
    pub fn is_success(&self) -> bool {
        self.errors.is_empty() && self.conflicts.is_empty()
    }

    /// Get total number of files synced
    pub fn total_synced(&self) -> usize {
        self.uploaded.len() + self.downloaded.len()
    }
}

impl Default for SyncResult {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute SHA-256 hash of content
pub fn compute_hash(content: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content);
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_summary() {
        let mut summary = SyncSummary::new();

        summary.add(SyncFileStatus::Synced);
        summary.add(SyncFileStatus::Synced);
        summary.add(SyncFileStatus::Modified);
        summary.add(SyncFileStatus::Conflict);

        assert_eq!(summary.synced, 2);
        assert_eq!(summary.modified, 1);
        assert_eq!(summary.conflict, 1);
        assert_eq!(summary.total(), 4);
        assert!(summary.has_issues());
        assert!(summary.has_conflicts());
    }

    #[test]
    fn test_overall_status_from_summary() {
        let synced = SyncSummary {
            synced: 10,
            ..Default::default()
        };
        assert_eq!(OverallSyncStatus::from(&synced), OverallSyncStatus::Synced);

        let modified = SyncSummary {
            synced: 10,
            modified: 2,
            ..Default::default()
        };
        assert_eq!(
            OverallSyncStatus::from(&modified),
            OverallSyncStatus::Modified
        );

        let conflict = SyncSummary {
            synced: 10,
            modified: 2,
            conflict: 1,
            ..Default::default()
        };
        assert_eq!(
            OverallSyncStatus::from(&conflict),
            OverallSyncStatus::Conflict
        );
    }

    #[test]
    fn test_compute_hash() {
        let hash = compute_hash(b"hello world");
        assert!(hash.starts_with("sha256:"));
        assert_eq!(hash.len(), 7 + 64); // "sha256:" + 64 hex chars
    }
}
