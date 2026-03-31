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
    /// * `node` - The node being audited
    /// * `action` - The action performed on the node
    /// * `details` - Optional additional details about the action
    fn write<'a>(
        &'a self,
        node: &'a models::nodes::Node,
        action: AuditLogAction,
        details: Option<String>,
    ) -> Pin<Box<dyn std::future::Future<Output = raisin_error::Result<()>> + Send + 'a>>;
}
