use std::collections::HashMap;
use tokio::sync::RwLock;

use raisin_error::Result;
use raisin_models::nodes::audit_log::{AuditLog, AuditLogAction};

/// Trait for audit log repositories
pub trait AuditRepository: Send + Sync {
    fn write_log(&self, log: AuditLog) -> impl std::future::Future<Output = Result<()>> + Send;
    fn get_logs_by_node_id(
        &self,
        node_id: &str,
    ) -> impl std::future::Future<Output = Result<Vec<AuditLog>>> + Send;
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

pub fn make_log(
    node_id: String,
    path: String,
    workspace: String,
    user_id: Option<String>,
    action: AuditLogAction,
    details: Option<String>,
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
    }
}
