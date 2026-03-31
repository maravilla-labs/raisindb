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

//! Authentication service for database admin users.
//!
//! Provides password hashing with bcrypt and JWT token generation/validation.

mod admin_auth;
mod api_keys;
mod password;
mod user_crud;
mod user_tokens;

#[cfg(test)]
mod tests;

use crate::api_key_store::ApiKeyStore;
use crate::AdminUserStore;
use raisin_models::admin_user::AdminAccessFlags;
use serde::{Deserialize, Serialize};

/// JWT claims for admin users
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminClaims {
    /// Subject (user_id)
    pub sub: String,
    /// Username
    pub username: String,
    /// Tenant ID
    pub tenant_id: String,
    /// Access flags
    pub access_flags: AdminAccessFlags,
    /// Must change password flag
    pub must_change_password: bool,
    /// Expiration time (Unix timestamp)
    pub exp: i64,
    /// Issued at (Unix timestamp)
    pub iat: i64,
}

/// Authentication service for admin users
#[derive(Clone)]
pub struct AuthService {
    pub(super) store: AdminUserStore,
    pub(super) api_key_store: ApiKeyStore,
    pub(super) jwt_secret: String,
    pub(super) token_expiry_hours: i64,
}

impl AuthService {
    /// Create a new authentication service
    pub fn new(store: AdminUserStore, jwt_secret: String) -> Self {
        let api_key_store = ApiKeyStore::new(store.db());
        Self {
            store,
            api_key_store,
            jwt_secret,
            token_expiry_hours: 24, // Default: 24 hours
        }
    }

    /// Set custom token expiry time (in hours)
    pub fn with_token_expiry(mut self, hours: i64) -> Self {
        self.token_expiry_hours = hours;
        self
    }
}
