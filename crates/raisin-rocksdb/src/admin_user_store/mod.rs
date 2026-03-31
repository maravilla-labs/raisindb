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

//! Storage layer for database admin users.
//!
//! Admin users are stored in the `admin_users` column family with keys:
//! ```text
//! sys\0{tenant_id}\0users\0{username}
//! ```
//!
//! This allows:
//! - Unique usernames per tenant
//! - Easy listing of all users for a tenant (prefix scan)
//! - Separation from workspace-level users

mod crud;
#[cfg(test)]
mod tests;

use crate::cf;
use rocksdb::DB;
use std::sync::Arc;

/// Storage service for database admin users
#[derive(Clone)]
pub struct AdminUserStore {
    pub(super) db: Arc<DB>,
    pub(super) operation_capture: Option<Arc<crate::OperationCapture>>,
}

impl AdminUserStore {
    /// Create a new admin user store
    pub fn new(db: Arc<DB>) -> Self {
        Self {
            db,
            operation_capture: None,
        }
    }

    /// Create a new admin user store with operation capture
    pub fn new_with_capture(db: Arc<DB>, operation_capture: Arc<crate::OperationCapture>) -> Self {
        Self {
            db,
            operation_capture: Some(operation_capture),
        }
    }

    /// Get a reference to the underlying database
    pub fn db(&self) -> Arc<DB> {
        self.db.clone()
    }

    /// Build storage key for an admin user
    ///
    /// Format: `sys\0{tenant_id}\0users\0{username}`
    pub(super) fn build_key(tenant_id: &str, username: &str) -> Vec<u8> {
        let mut key = Vec::new();
        key.extend_from_slice(b"sys");
        key.push(0);
        key.extend_from_slice(tenant_id.as_bytes());
        key.push(0);
        key.extend_from_slice(b"users");
        key.push(0);
        key.extend_from_slice(username.as_bytes());
        key
    }

    /// Build prefix for listing all users in a tenant
    ///
    /// Format: `sys\0{tenant_id}\0users\0`
    pub(super) fn build_tenant_prefix(tenant_id: &str) -> Vec<u8> {
        let mut prefix = Vec::new();
        prefix.extend_from_slice(b"sys");
        prefix.push(0);
        prefix.extend_from_slice(tenant_id.as_bytes());
        prefix.push(0);
        prefix.extend_from_slice(b"users");
        prefix.push(0);
        prefix
    }
}
