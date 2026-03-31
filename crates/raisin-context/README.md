# raisin-context

Multi-tenancy context and pluggable trait definitions for RaisinDB.

## Overview

This crate provides foundational types and traits for multi-tenant applications:

- **TenantContext** - Tenant and deployment context for request isolation
- **RepositoryContext** - Repository-first architecture scoping with storage key generation
- **TenantResolver** - Trait for extracting tenant info from requests (subdomain, header, path, JWT)
- **ServiceTier** - Free/Professional/Enterprise tier definitions with limits
- **TierProvider** - Trait for integrating with billing/subscription systems
- **RateLimiter** - Trait for rate limiting implementations
- **Branch/Tag/MergeResult** - Git-like versioning types

## Usage

```rust
use raisin_context::{TenantContext, RepositoryContext, TenantResolver};

// Basic tenant context
let ctx = TenantContext::new("customer-123", "production");
assert_eq!(ctx.storage_prefix(), "/customer-123/production");

// Repository context (preferred for repository-first architecture)
let repo = RepositoryContext::new("acme-corp", "website");
assert_eq!(repo.storage_prefix(), "/acme-corp/repo/website");

// Implement custom tenant resolution
struct SubdomainResolver;
impl TenantResolver for SubdomainResolver {
    fn resolve(&self, host: &str) -> Option<TenantContext> {
        let subdomain = host.split('.').next()?;
        Some(TenantContext::new(subdomain, "production"))
    }
}
```

## Isolation Modes

- **Single** - No isolation, direct storage access (embedded use)
- **Shared** - Logical isolation via tenant_id + deployment prefix
- **Dedicated** - Separate database instance per tenant

## License

BSL-1.1 (Business Source License 1.1)
