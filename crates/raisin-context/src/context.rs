//! Tenant context types

use serde::{Deserialize, Serialize};

/// Represents the tenant and deployment context for a request.
///
/// This is the core type that flows through the system to provide
/// multi-tenant isolation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TenantContext {
    /// Unique identifier for the tenant (e.g., "customer-123", "acme-corp")
    tenant_id: String,

    /// Deployment environment (e.g., "production", "preview", "staging")
    deployment: String,
}

impl TenantContext {
    /// Create a new tenant context
    pub fn new(tenant_id: impl Into<String>, deployment: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            deployment: deployment.into(),
        }
    }

    /// Get the tenant ID
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    /// Get the deployment name
    pub fn deployment(&self) -> &str {
        &self.deployment
    }

    /// Generate a storage key prefix for this context
    ///
    /// Format: `/{tenant_id}/{deployment}`
    pub fn storage_prefix(&self) -> String {
        format!("/{}/{}", self.tenant_id, self.deployment)
    }

    /// Check if this context matches given tenant and deployment
    pub fn matches(&self, tenant_id: &str, deployment: &str) -> bool {
        self.tenant_id == tenant_id && self.deployment == deployment
    }
}

/// Defines the isolation mode for the database
#[derive(Debug, Clone, Default)]
pub enum IsolationMode {
    /// Single-tenant mode - no isolation, direct storage access
    /// This is the default for embedded use cases
    #[default]
    Single,

    /// Shared database with logical isolation via tenant_id + deployment
    /// Keys/paths are prefixed with tenant context
    Shared(TenantContext),

    /// Dedicated database for premium customers
    /// Connection string points to a separate database instance
    Dedicated {
        context: TenantContext,
        connection_string: String,
    },
}

impl IsolationMode {
    /// Check if this is single-tenant mode
    pub fn is_single(&self) -> bool {
        matches!(self, IsolationMode::Single)
    }

    /// Get the tenant context if in multi-tenant mode
    pub fn context(&self) -> Option<&TenantContext> {
        match self {
            IsolationMode::Single => None,
            IsolationMode::Shared(ctx) => Some(ctx),
            IsolationMode::Dedicated { context, .. } => Some(context),
        }
    }

    /// Get the storage prefix for this isolation mode
    pub fn storage_prefix(&self) -> Option<String> {
        self.context().map(|ctx| ctx.storage_prefix())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tenant_context_creation() {
        let ctx = TenantContext::new("tenant-1", "prod");
        assert_eq!(ctx.tenant_id(), "tenant-1");
        assert_eq!(ctx.deployment(), "prod");
    }

    #[test]
    fn test_storage_prefix() {
        let ctx = TenantContext::new("acme", "staging");
        assert_eq!(ctx.storage_prefix(), "/acme/staging");
    }

    #[test]
    fn test_isolation_mode_single() {
        let mode = IsolationMode::Single;
        assert!(mode.is_single());
        assert!(mode.context().is_none());
        assert!(mode.storage_prefix().is_none());
    }

    #[test]
    fn test_isolation_mode_shared() {
        let ctx = TenantContext::new("tenant-2", "preview");
        let mode = IsolationMode::Shared(ctx.clone());
        assert!(!mode.is_single());
        assert_eq!(mode.context(), Some(&ctx));
        assert_eq!(mode.storage_prefix(), Some("/tenant-2/preview".to_string()));
    }
}
