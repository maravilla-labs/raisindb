// SPDX-License-Identifier: BSL-1.1

//! Tenant initialization middleware.
//!
//! Ensures NodeTypes are lazily initialized for a tenant on first request.

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};
use raisin_core::init::{calculate_nodetype_version, init_tenant_nodetypes};
use raisin_storage::{RegistryRepository, Storage};

use crate::state::AppState;

use super::types::TenantInfo;

/// Middleware that ensures tenant NodeTypes are initialized.
///
/// This middleware:
/// 1. Extracts tenant_id and deployment_key from request headers
/// 2. Checks if the tenant needs NodeType initialization
/// 3. Initializes NodeTypes if needed (lazy initialization on first request)
/// 4. Stores TenantInfo in request extensions for downstream handlers
///
/// Expected headers:
/// - `x-tenant-id`: Tenant identifier
/// - `x-deployment-key`: Deployment environment (e.g., "production", "staging")

/// Per-process memo of tenant NodeType initialization.
///
/// # Why this exists
///
/// `ensure_tenant_middleware` re-checked initialization on EVERY request. When
/// the check says "not initialized" it calls `init_tenant_nodetypes`, which
/// loads every embedded NodeType and seeds it into the hardcoded `main` repo
/// and `main` branch (`raisin-storage/src/tenant_init.rs`). On a deployment
/// with no repo called `main` that fails with `Branch 'main' not found` —
/// AFTER doing the work and BEFORE `update_deployment_nodetype_version`. The
/// version is therefore never persisted, so the next request repeats all of
/// it. Measured at ~20 ms on every request, indefinitely, and invisible
/// because the error was swallowed at `debug!`.
///
/// This memo makes the outcome sticky within the process:
/// * success — never checked again for that version;
/// * failure — not retried for [`RETRY_FAILED_AFTER`], which keeps the
///   documented intent (a pre-customer tenant whose repo does not exist yet
///   should initialize once it does) without paying for it per request.
///
/// The NodeType version is part of the decision, so a binary carrying new
/// NodeTypes re-initializes exactly as before. The memo is per process:
/// another node makes its own attempt, and initialization is idempotent.
mod init_memo {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    /// How long a FAILED attempt suppresses retries. Long enough that a
    /// request storm costs one attempt, short enough that a tenant whose repo
    /// appears later initializes without a restart.
    const RETRY_FAILED_AFTER: Duration = Duration::from_secs(60);

    struct Outcome {
        version: String,
        succeeded: bool,
        at: Instant,
    }

    fn memo() -> &'static Mutex<HashMap<(String, String), Outcome>> {
        static MEMO: OnceLock<Mutex<HashMap<(String, String), Outcome>>> = OnceLock::new();
        MEMO.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// Should this request check (and possibly perform) initialization?
    pub(super) fn should_check(tenant: &str, deployment: &str, version: &str) -> bool {
        let Ok(map) = memo().lock() else {
            return true;
        };
        match map.get(&(tenant.to_string(), deployment.to_string())) {
            Some(o) if o.version == version && o.succeeded => false,
            Some(o) if o.version == version => o.at.elapsed() >= RETRY_FAILED_AFTER,
            _ => true,
        }
    }

    /// Record what happened, so the next request can skip.
    pub(super) fn record(tenant: &str, deployment: &str, version: &str, succeeded: bool) {
        if let Ok(mut map) = memo().lock() {
            map.insert(
                (tenant.to_string(), deployment.to_string()),
                Outcome {
                    version: version.to_string(),
                    succeeded,
                    at: Instant::now(),
                },
            );
        }
    }

    /// True the first time a given failure is seen, so it can be logged loudly
    /// once instead of silently forever.
    pub(super) fn first_failure(tenant: &str, deployment: &str, version: &str) -> bool {
        let Ok(map) = memo().lock() else {
            return false;
        };
        !matches!(
            map.get(&(tenant.to_string(), deployment.to_string())),
            Some(o) if o.version == version && !o.succeeded
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_success_is_never_rechecked_and_a_new_version_is() {
            assert!(should_check("t1", "production", "v1"));
            record("t1", "production", "v1", true);
            assert!(!should_check("t1", "production", "v1"));
            // A binary with different NodeTypes must initialize again.
            assert!(should_check("t1", "production", "v2"));
        }

        #[test]
        fn a_failure_suppresses_retries_but_is_reported_once() {
            assert!(first_failure("t2", "production", "v1"));
            record("t2", "production", "v1", false);
            // Suppressed for the retry window rather than hammered per request.
            assert!(!should_check("t2", "production", "v1"));
            // ...and not re-warned.
            assert!(!first_failure("t2", "production", "v1"));
        }
    }
}

/// What the request's bearer credential says about which tenant it may act on.
enum CredentialTenant {
    /// The operator superadmin bearer, or an admin token flagged
    /// `can_impersonate`. Free to name any tenant in the header.
    MayCrossTenants,
    /// A credential issued for exactly this tenant.
    Bound(String),
    /// No readable credential. The header decides, as it always did.
    Unknown,
}

/// Read the tenant a request's bearer credential is bound to.
///
/// This runs BEFORE the auth middlewares (this layer is outermost), so it
/// re-reads the token rather than waiting for an `AuthContext`. It only ever
/// *reads*: a token that fails validation here is `Unknown` and is rejected
/// later by whichever auth layer the route carries, so this can never turn a
/// bad token into a 200.
#[cfg(feature = "storage-rocksdb")]
fn resolve_credential_tenant(state: &AppState, req: &Request<Body>) -> CredentialTenant {
    let Some(token) = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
    else {
        return CredentialTenant::Unknown;
    };

    // The operator superadmin bearer. Constant-time compare, same primitive as
    // the auth middlewares use.
    if let Ok(expected) = std::env::var("RAISIN_SUPERADMIN_TOKEN") {
        if !expected.is_empty() && token.len() == expected.len() {
            let mut diff: u8 = 0;
            for (a, b) in token.as_bytes().iter().zip(expected.as_bytes().iter()) {
                diff |= a ^ b;
            }
            if diff == 0 {
                return CredentialTenant::MayCrossTenants;
            }
        }
    }

    let Some(auth_service) = state.auth_service() else {
        return CredentialTenant::Unknown;
    };

    if token.starts_with("raisin_") {
        return match auth_service.validate_api_key(token) {
            Ok(Some(key)) => CredentialTenant::Bound(key.tenant_id),
            _ => CredentialTenant::Unknown,
        };
    }

    if let Ok(admin) = auth_service.validate_token(token) {
        return if admin.access_flags.can_impersonate {
            CredentialTenant::MayCrossTenants
        } else {
            CredentialTenant::Bound(admin.tenant_id)
        };
    }

    match auth_service.validate_user_token(token) {
        Ok(user) => CredentialTenant::Bound(user.tenant_id),
        Err(_) => CredentialTenant::Unknown,
    }
}

/// Without the RocksDB auth service there is no token to read.
#[cfg(not(feature = "storage-rocksdb"))]
fn resolve_credential_tenant(_state: &AppState, _req: &Request<Body>) -> CredentialTenant {
    CredentialTenant::Unknown
}

///
/// If headers are missing, defaults to "default" tenant and "production" deployment.
pub async fn ensure_tenant_middleware(
    State(state): State<AppState>,
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    // The `x-tenant-id` header is a REQUEST, not an assertion. A bearer
    // credential names the tenant it was issued for, and until this check
    // existed nothing compared the two: any authenticated caller could reach
    // another tenant's data by sending a different header, and an unrecognised
    // value auto-registered a brand-new tenant as a side effect of one GET.
    let header_tenant = req
        .headers()
        .get(raisin_context::TENANT_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty());

    let tenant_id = match resolve_credential_tenant(&state, &req) {
        // The operator superadmin bearer, and an admin whose token carries
        // `can_impersonate`, act across tenants on purpose — that is what the
        // operator dashboard is. The header steers them, as before.
        CredentialTenant::MayCrossTenants => raisin_context::resolve_tenant_id(header_tenant),
        // A credential bound to one tenant. The header may agree with it or be
        // absent; it may not overrule it.
        CredentialTenant::Bound(token_tenant) => match header_tenant {
            Some(h) if h != token_tenant => {
                tracing::warn!(
                    header_tenant = %h,
                    token_tenant = %token_tenant,
                    "Rejected request: x-tenant-id disagrees with the token's tenant"
                );
                return Err(StatusCode::FORBIDDEN);
            }
            // No header: the token's own tenant, not the "default" fallback.
            // Falling back used to route a tenant's user at another tenant's
            // data whenever a client simply forgot the header.
            _ => token_tenant,
        },
        // No credential we can read (no token, an API key we cannot validate,
        // an OAuth resource token resolved later at the resource). Unchanged:
        // the header, or "default".
        CredentialTenant::Unknown => raisin_context::resolve_tenant_id(header_tenant),
    };

    let deployment_key = req
        .headers()
        .get("x-deployment-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("production")
        .to_string();

    // Check if tenant needs initialization.
    //
    // The memo short-circuits this entirely once the answer is known, which is
    // every request after the first — see `init_memo`.
    let storage = state.storage();
    let current_version = calculate_nodetype_version();

    // Only a tenant that already EXISTS gets lazily initialized here.
    // `init_tenant_nodetypes` calls `register_tenant`, so before this gate one
    // GET carrying an unheard-of `x-tenant-id` created that tenant, seeded
    // every embedded NodeType under it and left the registration behind.
    // Tenants are created by the provisioning endpoints
    // (`POST /api/registry/tenants`, repository creation), never by a header.
    // `default` is exempt: it is the single-operator / dev tenant and the
    // server seeds it at startup.
    let tenant_is_known = tenant_id == raisin_context::DEFAULT_TENANT_ID
        || matches!(storage.registry().get_tenant(&tenant_id).await, Ok(Some(_)));

    if tenant_is_known && init_memo::should_check(&tenant_id, &deployment_key, &current_version) {
        match storage
            .registry()
            .get_deployment(&tenant_id, &deployment_key)
            .await
        {
            Ok(needs_init) => {
                let up_to_date = needs_init
                    .and_then(|d| d.nodetype_version)
                    .as_ref()
                    .map(|v| v == &current_version)
                    .unwrap_or(false);

                if up_to_date {
                    // Remember, so the storage read above is not repeated for
                    // every subsequent request.
                    init_memo::record(&tenant_id, &deployment_key, &current_version, true);
                } else {
                    tracing::info!(
                        "Initializing NodeTypes for tenant {}/{}",
                        tenant_id,
                        deployment_key
                    );
                    match init_tenant_nodetypes((**storage).clone(), &tenant_id, &deployment_key)
                        .await
                    {
                        Ok(()) => {
                            init_memo::record(&tenant_id, &deployment_key, &current_version, true)
                        }
                        Err(e) => {
                            // The common case is a pre-customer tenant whose
                            // default repo + branch don't exist yet: skip and
                            // let the request through. Warn the FIRST time only
                            // — logging this at debug forever is exactly what
                            // let it run on every request unnoticed.
                            if init_memo::first_failure(
                                &tenant_id,
                                &deployment_key,
                                &current_version,
                            ) {
                                tracing::warn!(
                                    "NodeType initialization failed for tenant {}/{}: {} \
                                     (suppressed for 60s; requests are unaffected)",
                                    tenant_id,
                                    deployment_key,
                                    e
                                );
                            } else {
                                tracing::debug!(
                                    "NodeType initialization still failing for {}/{}: {}",
                                    tenant_id,
                                    deployment_key,
                                    e
                                );
                            }
                            init_memo::record(&tenant_id, &deployment_key, &current_version, false);
                        }
                    }
                }
            }
            Err(_) => {
                tracing::warn!(
                    "Failed to check deployment status for {}/{}",
                    tenant_id,
                    deployment_key
                );
            }
        }
    }

    // Learn this tenant's public origin while we have a request that carries
    // one. A co-located app reaches RaisinDB over loopback and cannot supply
    // it, so without this the only way to build that tenant's magic-link URL
    // is per-app configuration that a new org silently misses.
    crate::origin::note_request_origin(req.headers(), &tenant_id);

    req.extensions_mut().insert(TenantInfo {
        tenant_id,
        deployment_key,
    });

    Ok(next.run(req).await)
}
