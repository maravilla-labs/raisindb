// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! The tenant's retired user keys ledger (`raisin_rocksdb::retired_keys`),
//! for administrators: list it, and un-retire a key retired by mistake.
//!
//! - `GET    /api/raisindb/sys/{tenant_id}/retired-user-keys`
//! - `DELETE /api/raisindb/sys/{tenant_id}/retired-user-keys/{user_key}`
//!
//! Both need an admin token of that tenant with console access. An un-retire
//! is audit-logged with the actor and the key (nothing else).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use raisin_rocksdb::AdminClaims;

use crate::{error::ApiError, state::AppState};

/// Tenant admin with console access, for `tenant_id`.
pub(crate) fn require_tenant_admin(claims: &AdminClaims, tenant_id: &str) -> Result<(), ApiError> {
    if claims.tenant_id != tenant_id {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "TENANT_MISMATCH",
            "Token tenant does not match requested tenant",
        ));
    }
    if !claims.access_flags.console_login {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "INSUFFICIENT_PERMISSIONS",
            "Admin console access required",
        ));
    }
    Ok(())
}

fn db(state: &AppState) -> Result<&std::sync::Arc<rocksdb::DB>, ApiError> {
    state
        .rocksdb_storage
        .as_ref()
        .map(|s| s.db())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "STORAGE_NOT_AVAILABLE",
                "RocksDB storage not available",
            )
        })
}

/// List the tenant's retired user keys (keys and times only).
pub async fn list_retired_user_keys(
    State(state): State<AppState>,
    Path(tenant_id): Path<String>,
    Extension(claims): Extension<AdminClaims>,
) -> Result<Json<Vec<raisin_rocksdb::retired_keys::RetiredUserKey>>, ApiError> {
    require_tenant_admin(&claims, &tenant_id)?;
    let list = raisin_rocksdb::retired_keys::list(db(&state)?, &tenant_id)
        .map_err(|e| ApiError::storage_error(e.to_string()))?;
    Ok(Json(list))
}

/// Un-retire a key retired by mistake (204; 404 when it is not retired).
pub async fn unretire_user_key(
    State(state): State<AppState>,
    Path((tenant_id, user_key)): Path<(String, String)>,
    Extension(claims): Extension<AdminClaims>,
) -> Result<StatusCode, ApiError> {
    require_tenant_admin(&claims, &tenant_id)?;
    if !raisin_models::auth::is_user_key(&user_key) {
        return Err(ApiError::validation_failed("not a user key"));
    }
    let removed =
        raisin_rocksdb::retired_keys::unretire(db(&state)?, &tenant_id, &user_key, &claims.sub)
            .map_err(|e| ApiError::storage_error(e.to_string()))?;
    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found(format!("{user_key} is not retired")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims(tenant: &str, console: bool) -> AdminClaims {
        AdminClaims {
            sub: "admin".into(),
            username: "admin".into(),
            tenant_id: tenant.into(),
            access_flags: raisin_models::admin_user::AdminAccessFlags {
                console_login: console,
                ..Default::default()
            },
            must_change_password: false,
            exp: 0,
            iat: 0,
        }
    }

    #[test]
    fn only_a_console_admin_of_the_tenant_manages_the_ledger() {
        assert!(require_tenant_admin(&claims("t", true), "t").is_ok());
        assert!(require_tenant_admin(&claims("t", false), "t").is_err());
        assert!(require_tenant_admin(&claims("other", true), "t").is_err());
    }
}
