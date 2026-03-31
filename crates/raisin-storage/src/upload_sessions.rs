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

//! Upload session storage for resumable file uploads
//!
//! This module provides data structures for managing resumable chunked uploads.
//! Upload sessions track the state of multi-chunk uploads and enable
//! pause/resume functionality for large file transfers.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Status of an upload session
///
/// Tracks the lifecycle of a resumable upload from creation through completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadSessionStatus {
    /// Session created, waiting for chunks
    Pending,
    /// Chunks are being uploaded
    InProgress,
    /// All chunks received, finalizing
    Completing,
    /// Upload completed successfully
    Completed,
    /// Upload failed
    Failed,
    /// Upload cancelled by user
    Cancelled,
    /// Session expired
    Expired,
}

/// A resumable upload session
///
/// Represents a multi-chunk file upload in progress. Sessions are stored in RocksDB
/// and track metadata, progress, and temporary file locations for chunk reassembly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadSession {
    /// Unique session ID
    pub id: String,
    /// Tenant ID
    pub tenant_id: String,
    /// Target repository
    pub repository: String,
    /// Target branch
    pub branch: String,
    /// Target workspace
    pub workspace: String,
    /// Target path in workspace
    pub path: String,
    /// Original filename
    pub filename: String,
    /// Total file size in bytes
    pub file_size: u64,
    /// Content type (MIME)
    pub content_type: Option<String>,
    /// Node type to create (default: raisin:Asset)
    pub node_type: String,
    /// Chunk size in bytes (default: 10MB)
    pub chunk_size: u64,
    /// Bytes received so far
    pub bytes_received: u64,
    /// Number of completed chunks
    pub chunks_completed: u32,
    /// Total number of chunks
    pub total_chunks: u32,
    /// Current status
    pub status: UploadSessionStatus,
    /// Path to temp directory for chunks
    pub temp_dir: String,
    /// Additional metadata
    pub metadata: serde_json::Value,
    /// Created timestamp
    pub created_at: DateTime<Utc>,
    /// Last updated timestamp
    pub updated_at: DateTime<Utc>,
    /// Expiration timestamp (default: 24h from creation)
    pub expires_at: DateTime<Utc>,
    /// Error message if failed
    pub error: Option<String>,
    /// User who created the session
    pub created_by: Option<String>,
}

impl UploadSession {
    /// Create a new upload session
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repository` - Target repository name
    /// * `branch` - Target branch name
    /// * `workspace` - Target workspace name
    /// * `path` - Target path within workspace
    /// * `filename` - Original filename
    /// * `file_size` - Total file size in bytes
    /// * `content_type` - Optional MIME type
    /// * `node_type` - Optional node type (defaults to "raisin:Asset")
    /// * `chunk_size` - Optional chunk size (defaults to 10MB)
    /// * `metadata` - Optional additional metadata
    /// * `created_by` - Optional user identifier
    ///
    /// # Returns
    ///
    /// A new upload session with generated ID and calculated chunk count
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tenant_id: String,
        repository: String,
        branch: String,
        workspace: String,
        path: String,
        filename: String,
        file_size: u64,
        content_type: Option<String>,
        node_type: Option<String>,
        chunk_size: Option<u64>,
        metadata: Option<serde_json::Value>,
        created_by: Option<String>,
    ) -> Self {
        let id = nanoid::nanoid!();
        let now = Utc::now();
        let chunk_size = chunk_size.unwrap_or(10 * 1024 * 1024); // 10MB default
        let total_chunks = file_size.div_ceil(chunk_size) as u32;
        let temp_dir = format!("/tmp/raisin-uploads/{}", id);

        Self {
            id,
            tenant_id,
            repository,
            branch,
            workspace,
            path,
            filename,
            file_size,
            content_type,
            node_type: node_type.unwrap_or_else(|| "raisin:Asset".to_string()),
            chunk_size,
            bytes_received: 0,
            chunks_completed: 0,
            total_chunks,
            status: UploadSessionStatus::Pending,
            temp_dir,
            metadata: metadata.unwrap_or(serde_json::Value::Null),
            created_at: now,
            updated_at: now,
            expires_at: now + chrono::Duration::hours(24),
            error: None,
            created_by,
        }
    }

    /// Calculate progress as a ratio (0.0 - 1.0)
    ///
    /// Returns the fraction of bytes received out of total file size.
    /// Returns 1.0 for zero-byte files.
    pub fn progress(&self) -> f64 {
        if self.file_size == 0 {
            1.0
        } else {
            self.bytes_received as f64 / self.file_size as f64
        }
    }

    /// Check if session is expired
    ///
    /// Sessions expire after 24 hours by default (configurable via expires_at).
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    /// Get the expected offset for the next chunk
    ///
    /// Returns the byte offset where the next chunk should start.
    /// Used to validate chunk uploads are sequential and contiguous.
    pub fn expected_offset(&self) -> u64 {
        self.bytes_received
    }

    /// Get chunk filename for a given chunk number
    ///
    /// Returns the full path to the temporary file for a specific chunk.
    ///
    /// # Arguments
    ///
    /// * `chunk_num` - Zero-based chunk index
    ///
    /// # Returns
    ///
    /// Path to chunk file in format: `{temp_dir}/chunk_{chunk_num:04}`
    pub fn chunk_filename(&self, chunk_num: u32) -> String {
        format!("{}/chunk_{:04}", self.temp_dir, chunk_num)
    }
}
