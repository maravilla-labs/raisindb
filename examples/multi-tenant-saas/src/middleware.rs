//! Tenant resolution middleware

use axum::{
    extract::Host,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};
use raisin_context::{TenantContext, TenantResolver};

/// Middleware to extract and validate tenant context
pub struct TenantMiddleware<R: TenantResolver> {
    resolver: R,
}

impl<R: TenantResolver> TenantMiddleware<R> {
    pub fn new(resolver: R) -> Self {
        Self { resolver }
    }

    pub async fn layer<B>(
        &self,
        host: Host,
        request: Request<B>,
        next: Next,
    ) -> Result<Response, StatusCode> {
        // Extract tenant from host
        let tenant_ctx = self
            .resolver
            .resolve(&host.0)
            .ok_or(StatusCode::BAD_REQUEST)?;

        // Store in request extensions for later use
        // request.extensions_mut().insert(tenant_ctx);

        Ok(next.run(request).await)
    }
}
