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

//! API Key models and types.
//!
//! API keys are long-lived tokens that can be used for programmatic access
//! to RaisinDB, including pgwire connections.

use serde::{Deserialize, Serialize};

use crate::timestamp::StorageTimestamp;

/// An API key for programmatic access
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApiKey {
    /// Unique identifier for the API key
    pub key_id: String,
    /// User ID this key belongs to
    pub user_id: String,
    /// Tenant ID this key belongs to
    pub tenant_id: String,
    /// User-provided name for the key (e.g., "CI/CD Pipeline")
    pub name: String,
    /// SHA-256 hash of the key (never store raw key)
    pub key_hash: String,
    /// First 8 characters of the key for display (e.g., "raisin_ab")
    pub key_prefix: String,
    /// When the key was created
    pub created_at: StorageTimestamp,
    /// Last time the key was used for authentication
    pub last_used_at: Option<StorageTimestamp>,
    /// Whether the key is active (can be revoked)
    pub is_active: bool,
}

impl ApiKey {
    /// Create a new API key entry (without the raw token)
    pub fn new(
        key_id: String,
        user_id: String,
        tenant_id: String,
        name: String,
        key_hash: String,
        key_prefix: String,
    ) -> Self {
        Self {
            key_id,
            user_id,
            tenant_id,
            name,
            key_hash,
            key_prefix,
            created_at: StorageTimestamp::now(),
            last_used_at: None,
            is_active: true,
        }
    }

    /// Record that this key was used
    pub fn record_usage(&mut self) {
        self.last_used_at = Some(StorageTimestamp::now());
    }

    /// Revoke this key
    pub fn revoke(&mut self) {
        self.is_active = false;
    }
}

/// Request to create a new API key
#[derive(Debug, Clone, Deserialize)]
pub struct CreateApiKeyRequest {
    /// User-provided name for the key
    pub name: String,
}

/// Response after creating an API key
/// Note: The full token is only returned ONCE at creation time
#[derive(Debug, Clone, Serialize)]
pub struct CreateApiKeyResponse {
    /// The API key metadata
    pub key: ApiKeyResponse,
    /// The full API token - only shown once!
    pub token: String,
}

/// API key information (without sensitive data)
#[derive(Debug, Clone, Serialize)]
pub struct ApiKeyResponse {
    pub key_id: String,
    pub name: String,
    /// First 8 characters for identification (e.g., "raisin_ab")
    pub key_prefix: String,
    pub created_at: StorageTimestamp,
    pub last_used_at: Option<StorageTimestamp>,
    pub is_active: bool,
}

impl From<ApiKey> for ApiKeyResponse {
    fn from(key: ApiKey) -> Self {
        Self {
            key_id: key.key_id,
            name: key.name,
            key_prefix: key.key_prefix,
            created_at: key.created_at,
            last_used_at: key.last_used_at,
            is_active: key.is_active,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_key_creation() {
        let key = ApiKey::new(
            "key1".to_string(),
            "user1".to_string(),
            "default".to_string(),
            "My API Key".to_string(),
            "hash123".to_string(),
            "raisin_ab".to_string(),
        );

        assert_eq!(key.key_id, "key1");
        assert_eq!(key.name, "My API Key");
        assert!(key.is_active);
        assert!(key.last_used_at.is_none());
    }

    #[test]
    fn test_api_key_revoke() {
        let mut key = ApiKey::new(
            "key1".to_string(),
            "user1".to_string(),
            "default".to_string(),
            "Test".to_string(),
            "hash".to_string(),
            "raisin_xx".to_string(),
        );

        assert!(key.is_active);
        key.revoke();
        assert!(!key.is_active);
    }

    #[test]
    fn test_api_key_usage() {
        let mut key = ApiKey::new(
            "key1".to_string(),
            "user1".to_string(),
            "default".to_string(),
            "Test".to_string(),
            "hash".to_string(),
            "raisin_xx".to_string(),
        );

        assert!(key.last_used_at.is_none());
        key.record_usage();
        assert!(key.last_used_at.is_some());
    }
}
