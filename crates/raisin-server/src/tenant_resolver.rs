//! Tenant resolver for raisin-server
//!
//! Provides default implementation that uses a fixed tenant ID.
//! Production deployments can use more sophisticated resolvers from raisin-context.

use raisin_context::{TenantContext, TenantResolver};

/// Default tenant resolver that always returns a fixed tenant ID
///
/// This is used by raisin-server for single-tenant or development deployments.
/// For multi-tenant production deployments, use SubdomainResolver, HeaderResolver,
/// or implement a custom TenantResolver.
pub struct DefaultTenantResolver {
    tenant_id: String,
}

impl DefaultTenantResolver {
    pub fn new(tenant_id: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
        }
    }
}

impl TenantResolver for DefaultTenantResolver {
    fn resolve(&self, _input: &str) -> Option<TenantContext> {
        Some(TenantContext::new(&self.tenant_id, "production"))
    }
}
