// SPDX-License-Identifier: BSL-1.1

//! Path-tenant scoping extractor.
//!
//! The only authoritative source of tenant identity is `x-tenant-id`, which
//! `ensure_tenant_middleware` extracts into `TenantInfo`. Some legacy routes
//! also carry a `{tenant}` URL path segment for backwards compatibility. To
//! prevent a customer-authenticated client from acting on another tenant by
//! crafting the path segment, every handler with a `{tenant}` path param
//! must extract it through [`ScopedTenant`], which verifies the path tenant
//! matches the header tenant and returns 404 on mismatch (no info leak).
//!
//! # Usage
//!
//! ```ignore
//! use crate::middleware::ScopedTenant;
//!
//! pub async fn backup_tenant(
//!     State(state): State<ManagementState<S>>,
//!     ScopedTenant(tenant): ScopedTenant,
//! ) -> Result<...> {
//!     // tenant == x-tenant-id by construction
//! }
//! ```

use axum::{
    extract::{FromRequestParts, Path},
    http::{request::Parts, StatusCode},
};

use super::TenantInfo;

/// A tenant ID extracted from the URL path and verified against the
/// `x-tenant-id` header (via [`TenantInfo`]).
///
/// Returns `StatusCode::NOT_FOUND` if the path tenant does not match the
/// header tenant. We deliberately do not return 403 so the response does
/// not leak the existence of other tenants.
pub struct ScopedTenant(pub String);

impl<S> FromRequestParts<S> for ScopedTenant
where
    S: Send + Sync,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(path_tenant): Path<String> = Path::from_request_parts(parts, state)
            .await
            .map_err(|_| StatusCode::NOT_FOUND)?;

        let header_tenant = parts
            .extensions
            .get::<TenantInfo>()
            .map(|t| t.tenant_id.as_str())
            .unwrap_or("default");

        if path_tenant != header_tenant {
            tracing::warn!(
                path_tenant = %path_tenant,
                header_tenant = %header_tenant,
                "Rejected request: path tenant does not match x-tenant-id header"
            );
            return Err(StatusCode::NOT_FOUND);
        }

        Ok(ScopedTenant(path_tenant))
    }
}
