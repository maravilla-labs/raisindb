//! Tenant resolution trait

use crate::TenantContext;

/// Trait for resolving tenant context from request information.
///
/// Implement this trait to define how your application extracts
/// tenant information from incoming requests. Common strategies:
///
/// - Subdomain extraction (`acme.yourapp.com` -> tenant: "acme")
/// - Header-based (`X-Tenant-ID: acme`)
/// - Path-based (`/tenants/acme/...`)
/// - JWT claims
///
/// # Examples
///
/// ```rust
/// use raisin_context::{TenantResolver, TenantContext};
///
/// struct HeaderResolver;
///
/// impl TenantResolver for HeaderResolver {
///     fn resolve(&self, input: &str) -> Option<TenantContext> {
///         // Parse tenant from header value
///         if input.is_empty() {
///             return None;
///         }
///         Some(TenantContext::new(input, "production"))
///     }
/// }
/// ```
pub trait TenantResolver: Send + Sync {
    /// Resolve tenant context from input (e.g., hostname, header value, etc.)
    ///
    /// Returns `None` if no tenant can be resolved, which typically means
    /// the system should operate in single-tenant mode or reject the request.
    fn resolve(&self, input: &str) -> Option<TenantContext>;
}

/// A simple resolver that always returns a fixed tenant context.
/// Useful for testing or single-tenant deployments with multi-tenant infrastructure.
#[allow(dead_code)]
pub struct FixedTenantResolver {
    context: TenantContext,
}

#[allow(dead_code)]
impl FixedTenantResolver {
    pub fn new(tenant_id: impl Into<String>, deployment: impl Into<String>) -> Self {
        Self {
            context: TenantContext::new(tenant_id, deployment),
        }
    }
}

impl TenantResolver for FixedTenantResolver {
    fn resolve(&self, _input: &str) -> Option<TenantContext> {
        Some(self.context.clone())
    }
}

/// A resolver that extracts tenant from subdomain
///
/// Example: `acme.example.com` -> tenant: "acme"
#[allow(dead_code)]
pub struct SubdomainResolver {
    default_deployment: String,
}

#[allow(dead_code)]
impl SubdomainResolver {
    pub fn new(default_deployment: impl Into<String>) -> Self {
        Self {
            default_deployment: default_deployment.into(),
        }
    }
}

impl TenantResolver for SubdomainResolver {
    fn resolve(&self, host: &str) -> Option<TenantContext> {
        let parts: Vec<&str> = host.split('.').collect();
        if parts.len() < 2 {
            return None;
        }

        let subdomain = parts[0];

        // Skip common non-tenant subdomains
        if subdomain == "www" || subdomain == "api" || subdomain.is_empty() {
            return None;
        }

        Some(TenantContext::new(subdomain, &self.default_deployment))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fixed_resolver() {
        let resolver = FixedTenantResolver::new("test-tenant", "dev");
        let ctx = resolver.resolve("anything").unwrap();
        assert_eq!(ctx.tenant_id(), "test-tenant");
        assert_eq!(ctx.deployment(), "dev");
    }

    #[test]
    fn test_subdomain_resolver() {
        let resolver = SubdomainResolver::new("production");

        let ctx = resolver.resolve("acme.example.com").unwrap();
        assert_eq!(ctx.tenant_id(), "acme");
        assert_eq!(ctx.deployment(), "production");

        // Skip www
        assert!(resolver.resolve("www.example.com").is_none());

        // Skip api
        assert!(resolver.resolve("api.example.com").is_none());

        // Invalid domains
        assert!(resolver.resolve("localhost").is_none());
    }
}
