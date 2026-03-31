// SPDX-License-Identifier: BSL-1.1

//! Upload session types, constants, and in-memory session store.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Default chunk size: 10MB
pub(super) const DEFAULT_CHUNK_SIZE: u64 = 10 * 1024 * 1024;

/// Default session expiration: 24 hours
pub(super) const DEFAULT_SESSION_EXPIRATION_HOURS: i64 = 24;

/// Temporary directory for upload chunks
pub(super) const UPLOAD_TEMP_DIR: &str = "/tmp/raisin-uploads";

/// Upload session status
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UploadSessionStatus {
    Pending,
    InProgress,
    Completing,
    Completed,
    Failed,
    Cancelled,
    Expired,
}

/// Upload session data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadSession {
    pub id: String,
    pub tenant_id: String,
    pub repository: String,
    pub branch: String,
    pub workspace: String,
    pub path: String,
    pub filename: String,
    pub file_size: u64,
    pub content_type: Option<String>,
    pub node_type: String,
    pub chunk_size: u64,
    pub bytes_received: u64,
    pub chunks_completed: u32,
    pub total_chunks: u32,
    pub status: UploadSessionStatus,
    pub temp_dir: String,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// In-memory upload session store
/// TODO: Replace with RocksDB storage for persistence
#[derive(Clone)]
pub struct UploadSessionStore {
    sessions: Arc<RwLock<HashMap<String, UploadSession>>>,
}

impl UploadSessionStore {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn get(&self, upload_id: &str) -> Option<UploadSession> {
        let sessions = self.sessions.read().await;
        sessions.get(upload_id).cloned()
    }

    pub async fn put(&self, session: UploadSession) {
        let mut sessions = self.sessions.write().await;
        sessions.insert(session.id.clone(), session);
    }

    pub async fn delete(&self, upload_id: &str) {
        let mut sessions = self.sessions.write().await;
        sessions.remove(upload_id);
    }
}

/// Global upload session store
/// In production, this should be persisted in RocksDB
pub(super) static UPLOAD_STORE: once_cell::sync::Lazy<UploadSessionStore> =
    once_cell::sync::Lazy::new(UploadSessionStore::new);

/// Create upload session request
#[derive(Debug, Deserialize)]
pub struct CreateUploadRequest {
    pub repository: String,
    #[serde(default = "default_branch")]
    pub branch: String,
    pub workspace: String,
    pub path: String,
    pub filename: String,
    pub file_size: u64,
    pub content_type: Option<String>,
    #[serde(default = "default_node_type")]
    pub node_type: String,
    #[serde(default = "default_chunk_size")]
    pub chunk_size: u64,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

fn default_branch() -> String {
    "main".to_string()
}

fn default_node_type() -> String {
    "raisin:Asset".to_string()
}

fn default_chunk_size() -> u64 {
    DEFAULT_CHUNK_SIZE
}

/// Create upload session response
#[derive(Debug, Serialize)]
pub struct CreateUploadResponse {
    pub upload_id: String,
    pub upload_url: String,
    pub chunk_size: u64,
    pub total_chunks: u32,
    pub expires_at: DateTime<Utc>,
}

/// Chunk upload response
#[derive(Debug, Serialize)]
pub struct ChunkUploadResponse {
    pub upload_id: String,
    pub bytes_received: u64,
    pub bytes_total: u64,
    pub chunks_completed: u32,
    pub chunks_total: u32,
    pub progress: f64,
}

/// Complete upload request
#[derive(Debug, Deserialize)]
pub struct CompleteUploadRequest {
    pub commit_message: Option<String>,
    pub commit_actor: Option<String>,
}

/// Complete upload response
#[derive(Debug, Serialize)]
pub struct CompleteUploadResponse {
    pub upload_id: String,
    pub job_id: String,
    pub status: String,
}
