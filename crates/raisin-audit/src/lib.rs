//! Audit-log storage.
//!
//! An [`AuditRepository`] persists [`AuditLog`] entries and serves them back per
//! node. Two implementations exist: [`InMemoryAuditRepo`] (dev / tests / the
//! non-RocksDB backend, lost on restart) and `RocksDBAuditRepo` in
//! `raisin-rocksdb` (persistent).
//!
//! # Scoping
//!
//! The original two trait methods (`write_log` / `get_logs_by_node_id`) key on
//! `node_id` alone, which is fine for a process-local map but cannot isolate
//! tenants in a shared store. The `*_scoped` variants carry the full
//! (tenant, repo, branch, workspace) scope every caller already has, so a
//! persistent implementation can key by it — and so a tenant wipe can actually
//! find the rows.
//!
//! They are **defaulted** to delegate to the unscoped pair, so existing
//! implementations keep working unchanged; only stores that can honour the scope
//! override them.

use std::collections::HashMap;
use tokio::sync::RwLock;

use raisin_error::Result;
use raisin_models::nodes::audit_log::{AuditLog, AuditLogAction};

/// The (tenant, repo, branch, workspace) an audit entry belongs to.
///
/// Borrowed rather than owned because every call site — the event-bus writer and
/// all four readers — already holds these as `&str` on the request/event.
#[derive(Debug, Clone, Copy)]
pub struct AuditScope<'a> {
    pub tenant_id: &'a str,
    pub repo_id: &'a str,
    pub branch: &'a str,
    pub workspace: &'a str,
}

impl<'a> AuditScope<'a> {
    pub fn new(tenant_id: &'a str, repo_id: &'a str, branch: &'a str, workspace: &'a str) -> Self {
        Self {
            tenant_id,
            repo_id,
            branch,
            workspace,
        }
    }
}

/// Trait for audit log repositories
pub trait AuditRepository: Send + Sync {
    fn write_log(&self, log: AuditLog) -> impl std::future::Future<Output = Result<()>> + Send;
    fn get_logs_by_node_id(
        &self,
        node_id: &str,
    ) -> impl std::future::Future<Output = Result<Vec<AuditLog>>> + Send;

    /// Write an entry into a specific tenant/repo/branch/workspace.
    ///
    /// Default: ignore the scope and delegate, preserving the global-bucket
    /// semantics of stores that cannot key by it.
    fn write_log_scoped(
        &self,
        scope: AuditScope<'_>,
        log: AuditLog,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let _ = scope;
        self.write_log(log)
    }

    /// Read a node's entries within a scope, newest first, capped at `limit`.
    ///
    /// Default: delegate to the unscoped read, then sort and truncate so every
    /// implementation presents the same ordering contract to callers.
    fn get_logs_scoped(
        &self,
        scope: AuditScope<'_>,
        node_id: &str,
        limit: Option<usize>,
    ) -> impl std::future::Future<Output = Result<Vec<AuditLog>>> + Send {
        let _ = scope;
        async move {
            let mut logs = self.get_logs_by_node_id(node_id).await?;
            logs.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
            if let Some(limit) = limit {
                logs.truncate(limit);
            }
            Ok(logs)
        }
    }
}

#[derive(Default)]
pub struct InMemoryAuditRepo {
    logs_by_node: RwLock<HashMap<String, Vec<AuditLog>>>,
}

impl AuditRepository for InMemoryAuditRepo {
    async fn write_log(&self, log: AuditLog) -> Result<()> {
        let mut map = self.logs_by_node.write().await;
        map.entry(log.node_id.clone()).or_default().push(log);
        Ok(())
    }

    async fn get_logs_by_node_id(&self, node_id: &str) -> Result<Vec<AuditLog>> {
        let map = self.logs_by_node.read().await;
        Ok(map.get(node_id).cloned().unwrap_or_default())
    }
}

/// Mint an [`AuditLog`] for a committed change.
///
/// `agent` is the optional non-human principal the write came through (see
/// [`AuditLog::agent`]); pass `None` for a direct human or system write.
pub fn make_log(
    node_id: String,
    path: String,
    workspace: String,
    user_id: Option<String>,
    action: AuditLogAction,
    details: Option<String>,
    agent: Option<String>,
) -> AuditLog {
    AuditLog {
        id: format!("log:{}:{}", &node_id, chrono::Utc::now().timestamp_millis()),
        node_id,
        path,
        workspace,
        user_id,
        action,
        timestamp: chrono::Utc::now(),
        details,
        agent,
    }
}
