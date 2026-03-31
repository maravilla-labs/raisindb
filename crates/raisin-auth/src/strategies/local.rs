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

//! Local username/password authentication strategy.
//!
//! This strategy implements traditional username/password authentication with:
//!
//! - Bcrypt password hashing and verification
//! - Failed login attempt tracking
//! - Account lockout after too many failed attempts
//! - Integration with tenant password policies
//!
//! # Security Features
//!
//! - Passwords are hashed using bcrypt with default cost factor (12)
//! - Account lockout prevents brute-force attacks
//! - Failed attempts are tracked and reset on successful login
//! - Lockout duration is configurable via tenant settings
//!
//! # Usage
//!
//! ```ignore
//! use raisin_auth::strategies::LocalStrategy;
//!
//! let strategy = LocalStrategy::new();
//!
//! // Hash a password for storage
//! let hash = LocalStrategy::hash_password("SecurePassword123")?;
//!
//! // Verify during authentication (handled automatically by authenticate)
//! let valid = LocalStrategy::verify_password("SecurePassword123", &hash);
//! ```

use async_trait::async_trait;
use raisin_error::{Error, Result};
use raisin_models::auth::AuthProviderConfig;

use crate::strategy::{AuthCredentials, AuthStrategy, AuthenticationResult, StrategyId};

/// Local username/password authentication strategy.
///
/// This is the simplest authentication strategy that verifies credentials
/// against locally stored password hashes.
#[derive(Debug, Clone)]
pub struct LocalStrategy {
    /// Strategy identifier
    strategy_id: StrategyId,
}

impl LocalStrategy {
    /// Create a new local authentication strategy.
    pub fn new() -> Self {
        Self {
            strategy_id: StrategyId::new(StrategyId::LOCAL),
        }
    }

    /// Hash a password using bcrypt.
    ///
    /// Uses bcrypt with the default cost factor (12), which provides
    /// a good balance between security and performance.
    ///
    /// # Arguments
    ///
    /// * `password` - The plain text password to hash
    ///
    /// # Returns
    ///
    /// The bcrypt hash string on success, or an error if hashing fails.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let hash = LocalStrategy::hash_password("MySecurePassword123")?;
    /// assert!(hash.starts_with("$2b$"));
    /// ```
    pub fn hash_password(password: &str) -> Result<String> {
        bcrypt::hash(password, bcrypt::DEFAULT_COST)
            .map_err(|e| Error::internal(format!("Failed to hash password: {}", e)))
    }

    /// Verify a password against a bcrypt hash.
    ///
    /// # Arguments
    ///
    /// * `password` - The plain text password to verify
    /// * `hash` - The bcrypt hash to verify against
    ///
    /// # Returns
    ///
    /// `true` if the password matches the hash, `false` otherwise.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let hash = LocalStrategy::hash_password("MySecurePassword123")?;
    /// assert!(LocalStrategy::verify_password("MySecurePassword123", &hash));
    /// assert!(!LocalStrategy::verify_password("WrongPassword", &hash));
    /// ```
    pub fn verify_password(password: &str, hash: &str) -> bool {
        bcrypt::verify(password, hash).unwrap_or(false)
    }
}

impl Default for LocalStrategy {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AuthStrategy for LocalStrategy {
    fn id(&self) -> &StrategyId {
        &self.strategy_id
    }

    fn name(&self) -> &str {
        "Local Authentication"
    }

    async fn init(
        &mut self,
        _config: &AuthProviderConfig,
        _decrypted_secret: Option<&str>,
    ) -> Result<()> {
        // Local strategy doesn't need initialization or secrets
        Ok(())
    }

    async fn authenticate(
        &self,
        tenant_id: &str,
        credentials: AuthCredentials,
    ) -> Result<AuthenticationResult> {
        // Extract username and password from credentials
        let (username, password) = match credentials {
            AuthCredentials::UsernamePassword { username, password } => (username, password),
            _ => {
                return Err(Error::Validation(
                    "Local strategy requires username/password credentials".to_string(),
                ))
            }
        };

        // NOTE: In a real implementation, this would:
        // 1. Look up the identity by username/email in the identity store
        // 2. Check if account is locked
        // 3. Verify the password
        // 4. Update failed attempt counters or reset on success
        //
        // For now, this is a skeleton that demonstrates the flow.
        // The actual identity lookup and persistence will be handled
        // by the AuthService that uses this strategy.

        // Placeholder implementation - in reality this is handled by AuthService
        let _ = (tenant_id, username, password);

        Err(Error::internal(
            "LocalStrategy::authenticate is a skeleton - actual authentication \
             is handled by AuthService which performs identity lookup, lockout \
             checks, and credential verification using this strategy's helper methods"
                .to_string(),
        ))
    }

    fn supports(&self, credentials: &AuthCredentials) -> bool {
        matches!(credentials, AuthCredentials::UsernamePassword { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_password_hashing() {
        let password = "SecurePassword123";
        let hash = LocalStrategy::hash_password(password).expect("Failed to hash password");

        // Bcrypt hashes start with $2b$ or $2a$ or $2y$
        assert!(hash.starts_with("$2"));
        assert!(hash.len() > 50); // Bcrypt hashes are typically 60 characters
    }

    #[test]
    fn test_password_verification() {
        let password = "SecurePassword123";
        let hash = LocalStrategy::hash_password(password).expect("Failed to hash password");

        // Correct password should verify
        assert!(LocalStrategy::verify_password(password, &hash));

        // Wrong password should not verify
        assert!(!LocalStrategy::verify_password("WrongPassword", &hash));

        // Empty password should not verify
        assert!(!LocalStrategy::verify_password("", &hash));
    }

    #[test]
    fn test_different_passwords_produce_different_hashes() {
        let password1 = "Password123";
        let password2 = "Password456";

        let hash1 = LocalStrategy::hash_password(password1).expect("Failed to hash password");
        let hash2 = LocalStrategy::hash_password(password2).expect("Failed to hash password");

        // Different passwords should produce different hashes
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_same_password_produces_different_hashes() {
        let password = "SamePassword123";

        let hash1 = LocalStrategy::hash_password(password).expect("Failed to hash password");
        let hash2 = LocalStrategy::hash_password(password).expect("Failed to hash password");

        // Same password hashed twice should produce different hashes (due to salt)
        assert_ne!(hash1, hash2);

        // But both should verify correctly
        assert!(LocalStrategy::verify_password(password, &hash1));
        assert!(LocalStrategy::verify_password(password, &hash2));
    }

    #[test]
    fn test_strategy_id() {
        let strategy = LocalStrategy::new();
        assert_eq!(strategy.id().as_ref(), StrategyId::LOCAL);
        assert_eq!(strategy.name(), "Local Authentication");
    }

    #[test]
    fn test_supports_credentials() {
        let strategy = LocalStrategy::new();

        // Should support username/password
        let creds = AuthCredentials::UsernamePassword {
            username: "user@example.com".to_string(),
            password: "password".to_string(),
        };
        assert!(strategy.supports(&creds));

        // Should not support magic link
        let magic_creds = AuthCredentials::MagicLinkToken {
            token: "token123".to_string(),
        };
        assert!(!strategy.supports(&magic_creds));

        // Should not support one-time token
        let ott_creds = AuthCredentials::OneTimeToken {
            token: "token456".to_string(),
        };
        assert!(!strategy.supports(&ott_creds));
    }

    #[tokio::test]
    async fn test_init_no_op() {
        let mut strategy = LocalStrategy::new();
        let config = AuthProviderConfig::local();

        // Init should succeed with no-op
        let result = strategy.init(&config, None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_authenticate_returns_skeleton_error() {
        let strategy = LocalStrategy::new();
        let creds = AuthCredentials::UsernamePassword {
            username: "user@example.com".to_string(),
            password: "password".to_string(),
        };

        let result = strategy.authenticate("tenant-1", creds).await;

        // Should return error explaining it's a skeleton
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            Error::Internal(msg) => {
                assert!(msg.contains("skeleton"));
                assert!(msg.contains("AuthService"));
            }
            _ => panic!("Expected Internal error"),
        }
    }

    #[tokio::test]
    async fn test_authenticate_wrong_credentials_type() {
        let strategy = LocalStrategy::new();
        let creds = AuthCredentials::MagicLinkToken {
            token: "token123".to_string(),
        };

        let result = strategy.authenticate("tenant-1", creds).await;

        // Should return validation error for wrong credential type
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            Error::Validation(msg) => {
                assert!(msg.contains("username/password"));
            }
            _ => panic!("Expected Validation error"),
        }
    }

    #[test]
    fn test_invalid_hash_returns_false() {
        // Verify should return false for invalid hash
        assert!(!LocalStrategy::verify_password("password", "invalid-hash"));
        assert!(!LocalStrategy::verify_password("password", ""));
        assert!(!LocalStrategy::verify_password(
            "password",
            "$2b$12$invalid"
        ));
    }
}
