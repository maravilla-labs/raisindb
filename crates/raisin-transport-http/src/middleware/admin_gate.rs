// SPDX-License-Identifier: BSL-1.1

//! Who may call the management, replication and AI/embedding configuration
//! routes.
//!
//! These routes had no gate at all: an anonymous caller could delete a
//! repository or a branch, rewrite the tenant's embedding endpoint, or apply
//! replication operations. [`admin_gate`] runs after `optional_auth_middleware`
//! and decides from the method and path alone (see [`policy`]), so the whole
//! table sits in one place and one test.
//!
//! * [`Access::Admin`] — an administrator credential: the operator superadmin
//!   bearer, an admin JWT, a `raisin_` API key (all carry `AdminClaims`), or a
//!   system / `system_admin` context.
//! * [`Access::SignedIn`] — any caller who is not anonymous. Used where Studio
//!   editors (identity users with `studio_admin`, not `system_admin`) call
//!   today, so their features keep working.
//! * [`Access::Open`] — unchanged: reads that public site renderers make.
//!
//! An administrator credential is also held to its own tenant: an admin JWT or
//! API key of tenant A cannot act on tenant B, by header or by a tenant path
//! segment. The superadmin bearer is operator-wide and exempt.

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
    middleware::Next,
    response::Response,
};
use raisin_models::auth::AuthContext;

use super::types::TenantInfo;

/// What a route requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    Open,
    SignedIn,
    Admin,
}

/// Subject of the synthetic claims the superadmin bearer carries.
const SUPERADMIN_SUB: &str = "superadmin-bearer";

fn is_read(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// The access a gated route requires. Paths not listed are administrative.
pub(crate) fn policy(method: &Method, path: &str) -> Access {
    let read = is_read(method);
    let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match segs.as_slice() {
        // Repository language config: site renderers read it anonymously;
        // Studio's translation settings write it as an editor.
        ["api", "repositories", _, "translation-config"] => {
            if read {
                Access::Open
            } else {
                Access::SignedIn
            }
        }
        ["api", "repositories", ..] => {
            if read {
                Access::SignedIn
            } else {
                Access::Admin
            }
        }
        // Branches, tags, revisions: Studio reads revisions as an editor.
        ["api", "management", "repositories", ..] => {
            if read {
                Access::SignedIn
            } else {
                Access::Admin
            }
        }
        ["api", "management", "registry", ..] => Access::Admin,
        ["api", "management", "system-definitions", ..] => Access::Admin,
        // Schema: reads stay as they were (the SDK reads types); validating a
        // node changes nothing; every write is an operator's.
        ["api", "management", _, _, _, "validate"] => Access::Open,
        ["api", "management", _, _, ..] if read => Access::Open,
        // Studio lists and downloads local models and reads the AI config as
        // an editor; everything else about AI configuration is an admin's.
        ["api", "tenants", _, "ai", "models", "huggingface", _, "download"] => Access::SignedIn,
        ["api", "tenants", _, "ai", ..] => {
            if read {
                Access::SignedIn
            } else {
                Access::Admin
            }
        }
        // Processing rules: the admin console's; reads for any editor.
        ["api", "repository", _, "ai", "rules", ..] => {
            if read {
                Access::SignedIn
            } else {
                Access::Admin
            }
        }
        _ => Access::Admin,
    }
}

/// The tenant a path names, for the routes that carry one.
fn path_tenant(path: &str) -> Option<&str> {
    let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match segs.as_slice() {
        ["api", "tenants", t, ..]
        | ["api", "replication", t, ..]
        | ["api", "management", "repositories", t, ..]
        | ["api", "admin", "management", "tenant", t, ..]
        | ["api", "admin", "management", "database", t, ..] => Some(t),
        _ => None,
    }
}

fn is_admin(req: &Request<Body>) -> bool {
    req.extensions()
        .get::<raisin_rocksdb::AdminClaims>()
        .is_some()
        || req
            .extensions()
            .get::<AuthContext>()
            .is_some_and(|a| a.is_system || a.permissions().is_some_and(|p| p.is_system_admin))
}

fn is_signed_in(req: &Request<Body>) -> bool {
    is_admin(req)
        || req
            .extensions()
            .get::<AuthContext>()
            .is_some_and(|a| !a.is_anonymous_principal())
}

/// The tenant a caller is held to, or `None` for the operator superadmin.
fn caller_tenant(req: &Request<Body>) -> Option<String> {
    match req.extensions().get::<raisin_rocksdb::AdminClaims>() {
        Some(c) if c.sub == SUPERADMIN_SUB => None,
        Some(c) => Some(c.tenant_id.clone()),
        None => req
            .extensions()
            .get::<TenantInfo>()
            .map(|t| t.tenant_id.clone()),
    }
}

/// The decision, free of the middleware plumbing.
pub(crate) fn decide(req: &Request<Body>) -> Result<(), StatusCode> {
    let access = policy(req.method(), req.uri().path());
    let allowed = match access {
        Access::Open => return Ok(()),
        Access::SignedIn => is_signed_in(req),
        Access::Admin => is_admin(req),
    };
    if !allowed {
        return Err(if is_signed_in(req) {
            StatusCode::FORBIDDEN
        } else {
            StatusCode::UNAUTHORIZED
        });
    }
    if let Some(own) = caller_tenant(req) {
        let request_tenant = req
            .extensions()
            .get::<TenantInfo>()
            .map(|t| t.tenant_id.as_str());
        let named = path_tenant(req.uri().path());
        let admin_claims = req
            .extensions()
            .get::<raisin_rocksdb::AdminClaims>()
            .is_some();
        // A credential is held to its tenant: by path, and (for admin
        // credentials) by the request tenant as well.
        if named.is_some_and(|t| t != own)
            || (admin_claims && request_tenant.is_some_and(|t| t != own))
        {
            tracing::warn!(
                path = %req.uri().path(),
                caller_tenant = %own,
                "refused a management call outside the caller's tenant"
            );
            return Err(StatusCode::FORBIDDEN);
        }
    }
    Ok(())
}

/// Layer after `optional_auth_middleware` on the routes [`policy`] governs.
pub async fn admin_gate(req: Request<Body>, next: Next) -> Result<Response, StatusCode> {
    if let Err(status) = decide(&req) {
        tracing::debug!(
            method = %req.method(),
            path = %req.uri().path(),
            %status,
            "management route refused"
        );
        return Err(status);
    }
    Ok(next.run(req).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::permissions::ResolvedPermissions;

    fn claims(tenant: &str, sub: &str) -> raisin_rocksdb::AdminClaims {
        raisin_rocksdb::AdminClaims {
            sub: sub.to_string(),
            username: sub.to_string(),
            tenant_id: tenant.to_string(),
            access_flags: raisin_models::admin_user::AdminAccessFlags {
                console_login: true,
                cli_access: true,
                api_access: true,
                pgwire_access: true,
                can_impersonate: false,
            },
            must_change_password: false,
            exp: 0,
            iat: 0,
        }
    }

    enum Caller {
        Nobody,
        Anonymous,
        User,
        SystemAdminUser,
        AdminCredential(&'static str),
        Superadmin,
    }

    fn req(method: Method, path: &str, tenant: &str, caller: Caller) -> Request<Body> {
        let mut r = Request::builder()
            .method(method)
            .uri(path)
            .body(Body::empty())
            .unwrap();
        r.extensions_mut().insert(TenantInfo {
            tenant_id: tenant.to_string(),
            deployment_key: "production".to_string(),
        });
        match caller {
            Caller::Nobody => {}
            Caller::Anonymous => {
                r.extensions_mut().insert(
                    AuthContext::anonymous_user("anon")
                        .with_permissions(ResolvedPermissions::anonymous(vec![])),
                );
            }
            Caller::User => {
                r.extensions_mut().insert(
                    AuthContext::for_user("bob")
                        .with_permissions(ResolvedPermissions::empty("bob")),
                );
            }
            Caller::SystemAdminUser => {
                r.extensions_mut().insert(
                    AuthContext::for_user("root")
                        .with_permissions(ResolvedPermissions::system_admin()),
                );
            }
            Caller::AdminCredential(t) => {
                r.extensions_mut().insert(claims(t, "admin-1"));
                r.extensions_mut().insert(AuthContext::system());
            }
            Caller::Superadmin => {
                r.extensions_mut().insert(claims(tenant, SUPERADMIN_SUB));
                r.extensions_mut().insert(AuthContext::system());
            }
        }
        r
    }

    const ADMIN_ONLY: &[(&str, &str)] = &[
        ("POST", "/api/replication/t1/r/operations/batch"),
        ("GET", "/api/replication/t1/r/operations"),
        ("GET", "/api/replication/t1/r/vector-clock"),
        ("POST", "/api/tenants/t1/embeddings/config"),
        ("GET", "/api/tenants/t1/embeddings/config"),
        ("POST", "/api/tenants/t1/embeddings/config/test"),
        ("PUT", "/api/tenants/t1/ai/config"),
        ("DELETE", "/api/tenants/t1/ai/providers/openai"),
        ("POST", "/api/tenants/t1/ai/providers/openai/test"),
        ("DELETE", "/api/tenants/t1/ai/models/huggingface/some-model"),
        ("DELETE", "/api/management/repositories/t1/r/branches/main"),
        ("POST", "/api/management/repositories/t1/r/branches"),
        (
            "PUT",
            "/api/management/repositories/t1/r/branches/main/head",
        ),
        (
            "POST",
            "/api/management/repositories/t1/r/branches/main/merge",
        ),
        ("POST", "/api/management/repositories/t1/r/tags"),
        ("POST", "/api/repositories"),
        ("DELETE", "/api/repositories/r"),
        ("PUT", "/api/repositories/r"),
        ("GET", "/api/management/registry/tenants"),
        ("POST", "/api/management/registry/deployments"),
        ("POST", "/api/management/r/main/nodetypes"),
        ("DELETE", "/api/management/r/main/archetypes/x"),
        ("POST", "/api/management/r/main/elementtypes/x/publish"),
        ("POST", "/api/management/system-definitions/reload"),
        ("GET", "/api/management/system-definitions"),
        ("POST", "/api/admin/management/database/t1/r/reindex/start"),
        ("POST", "/api/admin/management/global/rocksdb/backup"),
        ("POST", "/api/admin/management/tenant/t1/cleanup"),
        ("GET", "/api/admin/management/plugins"),
        ("POST", "/api/repository/r/ai/rules"),
    ];

    fn m(s: &str) -> Method {
        Method::from_bytes(s.as_bytes()).unwrap()
    }

    #[test]
    fn administrative_routes_refuse_everyone_but_administrators() {
        for (method, path) in ADMIN_ONLY {
            let d = |c| decide(&req(m(method), path, "t1", c));
            assert_eq!(
                d(Caller::Nobody),
                Err(StatusCode::UNAUTHORIZED),
                "{method} {path}"
            );
            assert_eq!(
                d(Caller::Anonymous),
                Err(StatusCode::UNAUTHORIZED),
                "{method} {path}"
            );
            assert_eq!(
                d(Caller::User),
                Err(StatusCode::FORBIDDEN),
                "{method} {path}"
            );
            // Admin JWT and API key (AdminClaims + system), the superadmin
            // bearer, and a system_admin identity user all pass.
            assert_eq!(d(Caller::AdminCredential("t1")), Ok(()), "{method} {path}");
            assert_eq!(d(Caller::Superadmin), Ok(()), "{method} {path}");
            assert_eq!(d(Caller::SystemAdminUser), Ok(()), "{method} {path}");
        }
    }

    #[test]
    fn an_admin_credential_is_held_to_its_tenant() {
        let r = req(
            Method::POST,
            "/api/tenants/t2/embeddings/config",
            "t2",
            Caller::AdminCredential("t1"),
        );
        assert_eq!(decide(&r), Err(StatusCode::FORBIDDEN));
        let r = req(
            Method::DELETE,
            "/api/repositories/r",
            "t2",
            Caller::AdminCredential("t1"),
        );
        assert_eq!(decide(&r), Err(StatusCode::FORBIDDEN));
        // The operator superadmin is not tenant-bound.
        let r = req(
            Method::POST,
            "/api/tenants/t2/embeddings/config",
            "t1",
            Caller::Superadmin,
        );
        assert_eq!(decide(&r), Ok(()));
    }

    /// What Studio editors (identity users, `studio_admin`) call today.
    #[test]
    fn studio_editor_paths_stay_open_to_signed_in_users() {
        for (method, path) in [
            ("GET", "/api/tenants/t1/ai/config"),
            ("GET", "/api/tenants/t1/ai/models/huggingface"),
            (
                "POST",
                "/api/tenants/t1/ai/models/huggingface/org%2Fmodel/download",
            ),
            ("GET", "/api/management/repositories/t1/r/revisions"),
            ("GET", "/api/management/repositories/t1/r/revisions/123"),
            ("PATCH", "/api/repositories/r/translation-config"),
            ("GET", "/api/repositories/r"),
            ("GET", "/api/repository/r/ai/rules"),
        ] {
            let d = |c| decide(&req(m(method), path, "t1", c));
            assert_eq!(d(Caller::User), Ok(()), "{method} {path}");
            assert_eq!(
                d(Caller::Anonymous),
                Err(StatusCode::UNAUTHORIZED),
                "{method} {path}"
            );
            assert_eq!(d(Caller::AdminCredential("t1")), Ok(()), "{method} {path}");
        }
        // ...but a user of another tenant naming this one is refused.
        let r = req(Method::GET, "/api/tenants/t2/ai/config", "t1", Caller::User);
        assert_eq!(decide(&r), Err(StatusCode::FORBIDDEN));
    }

    /// Reads public renderers and the SDK make stay as they were.
    #[test]
    fn public_reads_stay_open() {
        for (method, path) in [
            ("GET", "/api/repositories/r/translation-config"),
            ("GET", "/api/management/r/main/nodetypes"),
            ("GET", "/api/management/r/main/archetypes/x/resolved"),
            ("POST", "/api/management/r/main/nodetypes/validate"),
        ] {
            assert_eq!(
                decide(&req(m(method), path, "t1", Caller::Anonymous)),
                Ok(()),
                "{method} {path}"
            );
        }
    }
}
