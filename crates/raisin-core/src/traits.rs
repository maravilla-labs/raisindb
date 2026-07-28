use std::pin::Pin;

use raisin_models as models;
use raisin_models::nodes::audit_log::AuditLogAction;

/// Trait for auditing node operations
///
/// Provides functionality to write audit log entries for node changes.
///
/// Uses pinned boxed futures to maintain dyn-compatibility, allowing it to be used
/// with `Arc<dyn Audit>` for optional/pluggable auditing implementations.
pub trait Audit: Send + Sync {
    /// Write an audit log entry
    ///
    /// # Arguments
    /// * `scope` - The tenant / repo / branch / workspace the write happened in.
    ///   Carried explicitly because a `Node` does not know its branch or tenant,
    ///   and a persistent audit store must key by them to isolate tenants.
    /// * `node` - The node being audited
    /// * `action` - The action performed on the node
    /// * `details` - Optional additional details about the action
    /// * `agent` - Optional non-human principal the write came through (see
    ///   [`raisin_models::nodes::audit_log::AuditLog::agent`])
    fn write<'a>(
        &'a self,
        scope: raisin_audit::AuditScope<'a>,
        node: &'a models::nodes::Node,
        action: AuditLogAction,
        details: Option<String>,
        agent: Option<&'a str>,
    ) -> Pin<Box<dyn std::future::Future<Output = raisin_error::Result<()>> + Send + 'a>>;
}
