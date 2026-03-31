//! Multi-tenancy context and pluggable trait definitions for RaisinDB
//!
//! This crate provides the foundational types and traits for implementing
//! multi-tenant applications with RaisinDB. It's designed to be:
//!
//! - **Pluggable**: Implement traits for your specific tenant resolution logic
//! - **Optional**: Single-tenant apps don't need this
//! - **Flexible**: Works with any authentication/billing system
//!
//! # Examples
//!
//! ## Basic Tenant Context
//!
//! ```rust
//! use raisin_context::TenantContext;
//!
//! let ctx = TenantContext::new("customer-123", "production");
//! assert_eq!(ctx.tenant_id(), "customer-123");
//! assert_eq!(ctx.deployment(), "production");
//! ```
//!
//! ## Implementing Tenant Resolution
//!
//! ```rust
//! use raisin_context::{TenantResolver, TenantContext};
//!
//! struct SubdomainResolver;
//!
//! impl TenantResolver for SubdomainResolver {
//!     fn resolve(&self, host: &str) -> Option<TenantContext> {
//!         let subdomain = host.split('.').next()?;
//!         if subdomain == "www" || subdomain.is_empty() {
//!             return None;
//!         }
//!         Some(TenantContext::new(subdomain, "production"))
//!     }
//! }
//! ```

mod context;
mod rate_limit;
mod repository;
mod resolver;
mod tier;

pub use context::{IsolationMode, TenantContext};
pub use rate_limit::{RateLimitInfo, RateLimiter};
pub use repository::{
    Branch, BranchDivergence, ConflictResolution, ConflictType, MergeConflict, MergeResult,
    MergeStrategy, RepositoryConfig, RepositoryContext, RepositoryInfo, ResolutionType, Tag,
    WorkspaceConfig, WorkspaceScope,
};
pub use resolver::TenantResolver;
pub use tier::{Operation, ServiceTier, TierProvider};
