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

//! Tests for authentication job handlers

use super::*;
use async_trait::async_trait;
use raisin_auth::jobs::{
    AccessNotificationJobData, AccessNotificationType, MagicLinkJobData, SessionCleanupConfig,
    TokenCleanupConfig,
};
use raisin_error::Result;
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

// Mock email sender for testing
struct MockEmailSender {
    sent_magic_links: Mutex<Vec<MagicLinkJobData>>,
    sent_notifications: Mutex<Vec<AccessNotificationJobData>>,
}

impl MockEmailSender {
    fn new() -> Self {
        Self {
            sent_magic_links: Mutex::new(Vec::new()),
            sent_notifications: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl MagicLinkEmailSender for MockEmailSender {
    async fn send_magic_link(&self, data: &MagicLinkJobData) -> Result<()> {
        self.sent_magic_links.lock().await.push(data.clone());
        Ok(())
    }
}

#[async_trait]
impl AccessNotificationEmailSender for MockEmailSender {
    async fn send_access_notification(&self, data: &AccessNotificationJobData) -> Result<()> {
        self.sent_notifications.lock().await.push(data.clone());
        Ok(())
    }
}

// Mock session store for testing
struct MockSessionStore {
    expired_sessions: Vec<String>,
    deleted_count: AtomicUsize,
    invalidated_count: AtomicUsize,
}

impl MockSessionStore {
    fn new(expired_sessions: Vec<String>) -> Self {
        Self {
            expired_sessions,
            deleted_count: AtomicUsize::new(0),
            invalidated_count: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl SessionCleanupStore for MockSessionStore {
    async fn find_expired_sessions(
        &self,
        _tenant_id: Option<&str>,
        _max_idle_seconds: Option<u64>,
        _batch_size: usize,
    ) -> Result<Vec<String>> {
        Ok(self.expired_sessions.clone())
    }

    async fn delete_session(&self, _session_id: &str) -> Result<()> {
        self.deleted_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn invalidate_cache(&self, _session_id: &str) -> Result<()> {
        self.invalidated_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

// Mock token store for testing
struct MockTokenStore {
    expired_tokens: Vec<(String, String)>,
    deleted_count: AtomicUsize,
}

impl MockTokenStore {
    fn new(expired_tokens: Vec<(String, String)>) -> Self {
        Self {
            expired_tokens,
            deleted_count: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl TokenCleanupStore for MockTokenStore {
    async fn find_expired_tokens(
        &self,
        _tenant_id: Option<&str>,
        _token_types: &[String],
        _grace_period_seconds: u64,
        _batch_size: usize,
    ) -> Result<Vec<(String, String)>> {
        Ok(self.expired_tokens.clone())
    }

    async fn delete_token(&self, _token_hash: &str) -> Result<()> {
        self.deleted_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn create_job_info(job_type: JobType) -> JobInfo {
    use raisin_storage::jobs::JobId;
    JobInfo {
        id: JobId("test-job-123".to_string()),
        job_type,
        status: raisin_storage::jobs::JobStatus::Scheduled,
        tenant: Some("tenant-1".to_string()),
        started_at: chrono::Utc::now(),
        completed_at: None,
        progress: None,
        error: None,
        result: None,
        retry_count: 0,
        max_retries: 3,
        last_heartbeat: None,
        timeout_seconds: 300,
        next_retry_at: None,
    }
}

fn create_job_context(metadata: HashMap<String, serde_json::Value>) -> JobContext {
    JobContext {
        tenant_id: "tenant-1".to_string(),
        repo_id: "repo-1".to_string(),
        branch: "main".to_string(),
        workspace_id: "content".to_string(),
        revision: raisin_hlc::HLC::new(1, 0),
        metadata,
    }
}

#[tokio::test]
async fn test_magic_link_handler() {
    let sender = Arc::new(MockEmailSender::new());
    let handler = AuthMagicLinkSendHandler::new(sender.clone());

    let data = MagicLinkJobData::new(
        "identity-123",
        "user@example.com",
        "token-id-456",
        "abc123def456",
        "https://app.example.com",
        15,
    );

    let job = create_job_info(JobType::AuthMagicLinkSend {
        identity_id: "identity-123".to_string(),
        email: "user@example.com".to_string(),
        token_id: "token-id-456".to_string(),
    });

    let context = create_job_context(data.to_metadata());

    handler.handle(&job, &context).await.unwrap();

    let sent = sender.sent_magic_links.lock().await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].email, "user@example.com");
}

#[tokio::test]
async fn test_session_cleanup_handler() {
    let store = Arc::new(MockSessionStore::new(vec![
        "session-1".to_string(),
        "session-2".to_string(),
        "session-3".to_string(),
    ]));
    let handler = AuthSessionCleanupHandler::new(store.clone());

    let config = SessionCleanupConfig::default();

    let job = create_job_info(JobType::AuthSessionCleanup {
        tenant_id: Some("tenant-1".to_string()),
        batch_size: 100,
    });

    let context = create_job_context(config.to_metadata());

    let result = handler.handle(&job, &context).await.unwrap();

    assert_eq!(result.sessions_scanned, 3);
    assert_eq!(result.sessions_deleted, 3);
    assert_eq!(result.cache_entries_invalidated, 3);
    assert!(!result.has_more);
    assert!(result.errors.is_empty());

    assert_eq!(store.deleted_count.load(Ordering::SeqCst), 3);
    assert_eq!(store.invalidated_count.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn test_token_cleanup_handler() {
    let store = Arc::new(MockTokenStore::new(vec![
        ("hash-1".to_string(), "magic_link".to_string()),
        ("hash-2".to_string(), "magic_link".to_string()),
        ("hash-3".to_string(), "invite".to_string()),
    ]));
    let handler = AuthTokenCleanupHandler::new(store.clone());

    let config = TokenCleanupConfig::default();

    let job = create_job_info(JobType::AuthTokenCleanup {
        tenant_id: Some("tenant-1".to_string()),
        token_types: vec!["all".to_string()],
    });

    let context = create_job_context(config.to_metadata());

    let result = handler.handle(&job, &context).await.unwrap();

    assert_eq!(result.tokens_scanned, 3);
    assert_eq!(result.tokens_deleted, 3);
    assert_eq!(result.deleted_by_type.get("magic_link"), Some(&2));
    assert_eq!(result.deleted_by_type.get("invite"), Some(&1));
    assert!(!result.has_more);
    assert!(result.errors.is_empty());

    assert_eq!(store.deleted_count.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn test_access_notification_handler() {
    let sender = Arc::new(MockEmailSender::new());
    let handler = AuthAccessNotificationHandler::new(sender.clone());

    let data = AccessNotificationJobData::new(
        "identity-123",
        "user@example.com",
        "repo-456",
        "My Workspace",
        AccessNotificationType::Granted,
        "https://app.example.com",
    )
    .with_actor("admin-789", "Admin User")
    .with_roles(vec!["editor".to_string()]);

    let job = create_job_info(JobType::AuthAccessNotification {
        identity_id: "identity-123".to_string(),
        repo_id: "repo-456".to_string(),
        notification_type: "granted".to_string(),
    });

    let context = create_job_context(data.to_metadata());

    handler.handle(&job, &context).await.unwrap();

    let sent = sender.sent_notifications.lock().await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].email, "user@example.com");
    assert_eq!(sent[0].notification_type, "granted");
}

#[tokio::test]
async fn test_wrong_job_type_errors() {
    let sender = Arc::new(MockEmailSender::new());
    let magic_link_handler = AuthMagicLinkSendHandler::new(sender.clone());

    // Try to handle a session cleanup job with magic link handler
    let job = create_job_info(JobType::AuthSessionCleanup {
        tenant_id: None,
        batch_size: 100,
    });
    let context = create_job_context(HashMap::new());

    let result = magic_link_handler.handle(&job, &context).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Expected AuthMagicLinkSend"));
}
