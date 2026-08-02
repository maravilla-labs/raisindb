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

//! Durable storage for the OAuth 2.1 authorization server.
//!
//! Implements [`ClientStore`](raisin_auth::authserver::ClientStore) and
//! [`RefreshTokenStore`](raisin_auth::authserver::RefreshTokenStore) on top of
//! RocksDB, replacing the in-process default. Authorization codes are not
//! stored at all — they carry their own grant (`raisin_auth::authserver::code_codec`).
//!
//! # Why this has to be persistent
//!
//! An MCP host (ChatGPT, Claude, Cursor) registers itself once via RFC 7591
//! Dynamic Client Registration and then caches the issued `client_id`
//! **indefinitely** in its connector configuration — it has no reason to expect
//! the server to forget it. With an in-memory client store, every restart
//! invalidated every connector in existence: the next `/authorize` returned
//! `invalid_client: unknown client_id '…'` and the only user-visible remedy was
//! deleting and re-adding the connector. Clients therefore MUST outlive the
//! process.
//!
//! # Layout
//!
//! Records live in the `admin_users` column family — the same one the API-key
//! store uses — so no new column family (and therefore no migration on an
//! existing database) is required. Keys:
//!
//! ```text
//! sys\0{tenant}\0oauth_client\0{client_id}
//! sys\0{tenant}\0oauth_refresh\0{token_hash}
//! sys\0{tenant}\0oauth_refresh_fam\0{family_id}\0{token_hash}   (family index)
//! ```
//!
//! The family index exists solely so replay detection can revoke a whole
//! rotation chain with a prefix scan instead of a full table scan.
//!
//! # Atomicity
//!
//! `consume_refresh_token` is a read-then-write pair that must not interleave.
//! The underlying handle is a plain `rocksdb::DB` with no compare-and-swap, so
//! it is serialized per key through a [`KeyedMutex`]. That is a single-process
//! guarantee and only covers this node's own copy of the record.
//!
//! **Cluster-wide replay detection does not live here.** It is the lock lease
//! the authorization server takes before calling in (see
//! `AuthorizationServer::claim_once`), which is visible on every node the
//! moment it is taken instead of waiting for the replicated `consumed_at` to
//! converge. This store's `consumed_at` is the durable backstop for when that
//! backend is degraded.

mod clients;

mod refresh;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use raisin_auth::authserver::{AuthServerError, AuthServerResult};
use rocksdb::DB;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::cf;
use crate::jobs::KeyedMutex;

/// RocksDB-backed authorization-server state.
#[derive(Clone)]
pub struct RocksDbOAuthStore {
    db: Arc<DB>,
    /// Emits replication operations so clients and refresh tokens reach every
    /// node. `None` when replication is off (single node, tests).
    capture: Option<Arc<crate::OperationCapture>>,
    /// Serializes `consume_refresh_token`'s read-modify-write within this
    /// process. Cluster-wide exclusion is the caller's lock lease.
    locks: Arc<KeyedMutex<Vec<u8>>>,
}

impl RocksDbOAuthStore {
    /// Create a store over an open database handle.
    pub fn new(db: Arc<DB>) -> Self {
        Self {
            db,
            capture: None,
            locks: Arc::new(KeyedMutex::new()),
        }
    }

    /// Attach operation capture so writes replicate to the other nodes.
    pub fn with_capture(mut self, capture: Arc<crate::OperationCapture>) -> Self {
        self.capture = Some(capture);
        self
    }

    /// The capture handle, when replication is on and enabled.
    pub(super) fn capture(&self) -> Option<&Arc<crate::OperationCapture>> {
        self.capture.as_ref().filter(|c| c.is_enabled())
    }

    /// Resolve the column family the records live in.
    pub(super) fn cf(&self) -> AuthServerResult<&rocksdb::ColumnFamily> {
        self.db.cf_handle(cf::ADMIN_USERS).ok_or_else(|| {
            AuthServerError::ServerError("admin_users column family not found".to_string())
        })
    }

    /// Compose `sys\0{tenant}\0{kind}\0{id}`.
    pub(super) fn build_key(tenant_id: &str, kind: &str, id: &str) -> Vec<u8> {
        let mut key = Vec::with_capacity(8 + tenant_id.len() + kind.len() + id.len());
        key.extend_from_slice(b"sys");
        key.push(0);
        key.extend_from_slice(tenant_id.as_bytes());
        key.push(0);
        key.extend_from_slice(kind.as_bytes());
        key.push(0);
        key.extend_from_slice(id.as_bytes());
        key
    }

    /// Compose the family-index prefix `sys\0{tenant}\0oauth_refresh_fam\0{family}\0`.
    pub(super) fn build_family_prefix(tenant_id: &str, family_id: &str) -> Vec<u8> {
        let mut key = Self::build_key(tenant_id, refresh::FAMILY_KIND, family_id);
        key.push(0);
        key
    }

    /// Read and decode a record, mapping a decode failure to a server error.
    pub(super) fn get_record<T: DeserializeOwned>(
        &self,
        key: &[u8],
    ) -> AuthServerResult<Option<T>> {
        let cf = self.cf()?;
        let Some(bytes) = self
            .db
            .get_cf(cf, key)
            .map_err(|e| AuthServerError::ServerError(format!("oauth store read failed: {e}")))?
        else {
            return Ok(None);
        };
        let record = rmp_serde::from_slice(&bytes).map_err(|e| {
            AuthServerError::ServerError(format!("oauth record could not be decoded: {e}"))
        })?;
        Ok(Some(record))
    }

    /// Encode and write a record.
    ///
    /// Uses the **named** (map) MessagePack encoding, not the compact
    /// positional one. `OAuthClient` and `RefreshToken` both carry
    /// `skip_serializing_if` fields, and a skipped field in a positional
    /// encoding shifts every field after it — a `None` `client_secret_hash`
    /// made a stored client fail to decode as "invalid type: sequence, expected
    /// a string" on the way back out. Named encoding also lets a future field
    /// be added without orphaning already-persisted records.
    pub(super) fn put_record<T: Serialize>(&self, key: &[u8], record: &T) -> AuthServerResult<()> {
        let cf = self.cf()?;
        let bytes = rmp_serde::to_vec_named(record).map_err(|e| {
            AuthServerError::ServerError(format!("oauth record could not be encoded: {e}"))
        })?;
        self.db
            .put_cf(cf, key, &bytes)
            .map_err(|e| AuthServerError::ServerError(format!("oauth store write failed: {e}")))
    }

    /// Delete a record.
    pub(super) fn delete_record(&self, key: &[u8]) -> AuthServerResult<()> {
        let cf = self.cf()?;
        self.db
            .delete_cf(cf, key)
            .map_err(|e| AuthServerError::ServerError(format!("oauth store delete failed: {e}")))
    }

    /// Borrow the per-key lock registry.
    pub(super) fn locks(&self) -> &Arc<KeyedMutex<Vec<u8>>> {
        &self.locks
    }

    /// Borrow the database handle.
    pub(super) fn db(&self) -> &Arc<DB> {
        &self.db
    }
}
