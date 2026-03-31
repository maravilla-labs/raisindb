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

//! Storage layer for API keys.
//!
//! API keys are stored in the `admin_users` column family with keys:
//! ```text
//! sys\0{tenant_id}\0api_keys\0{user_id}\0{key_id}
//! ```
//!
//! Additionally, a hash index for fast token validation:
//! ```text
//! sys\0api_key_hash\0{hash_prefix}
//! ```

mod crud;
#[cfg(test)]
mod tests;

use rocksdb::DB;
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// Storage service for API keys
#[derive(Clone)]
pub struct ApiKeyStore {
    pub(super) db: Arc<DB>,
}

impl ApiKeyStore {
    /// Create a new API key store
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Build storage key for an API key
    ///
    /// Format: `sys\0{tenant_id}\0api_keys\0{user_id}\0{key_id}`
    pub(super) fn build_key(tenant_id: &str, user_id: &str, key_id: &str) -> Vec<u8> {
        let mut key = Vec::new();
        key.extend_from_slice(b"sys");
        key.push(0);
        key.extend_from_slice(tenant_id.as_bytes());
        key.push(0);
        key.extend_from_slice(b"api_keys");
        key.push(0);
        key.extend_from_slice(user_id.as_bytes());
        key.push(0);
        key.extend_from_slice(key_id.as_bytes());
        key
    }

    /// Build prefix for listing all API keys for a user
    ///
    /// Format: `sys\0{tenant_id}\0api_keys\0{user_id}\0`
    pub(super) fn build_user_prefix(tenant_id: &str, user_id: &str) -> Vec<u8> {
        let mut prefix = Vec::new();
        prefix.extend_from_slice(b"sys");
        prefix.push(0);
        prefix.extend_from_slice(tenant_id.as_bytes());
        prefix.push(0);
        prefix.extend_from_slice(b"api_keys");
        prefix.push(0);
        prefix.extend_from_slice(user_id.as_bytes());
        prefix.push(0);
        prefix
    }

    /// Build hash index key for fast token lookup
    ///
    /// Format: `sys\0api_key_hash\0{hash}`
    pub(super) fn build_hash_index_key(key_hash: &str) -> Vec<u8> {
        let mut key = Vec::new();
        key.extend_from_slice(b"sys");
        key.push(0);
        key.extend_from_slice(b"api_key_hash");
        key.push(0);
        key.extend_from_slice(key_hash.as_bytes());
        key
    }

    /// Generate a new API token
    ///
    /// Returns (raw_token, hash, prefix)
    pub fn generate_token() -> (String, String, String) {
        use rand::Rng;
        const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

        let mut rng = rand::thread_rng();

        // Generate 32 random characters
        let random_part: String = (0..32)
            .map(|_| {
                let idx = rng.gen_range(0..CHARSET.len());
                CHARSET[idx] as char
            })
            .collect();

        let raw_token = format!("raisin_{}", random_part);
        let prefix = raw_token[..16].to_string(); // "raisin_" + first 9 chars

        // Hash the token
        let mut hasher = Sha256::new();
        hasher.update(raw_token.as_bytes());
        let hash = format!("{:x}", hasher.finalize());

        (raw_token, hash, prefix)
    }

    /// Hash a raw token for comparison
    pub fn hash_token(raw_token: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(raw_token.as_bytes());
        format!("{:x}", hasher.finalize())
    }
}
