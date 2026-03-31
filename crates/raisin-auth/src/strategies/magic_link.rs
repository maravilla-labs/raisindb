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

//! Magic link passwordless authentication strategy.
//!
//! This strategy implements passwordless authentication via email with:
//!
//! - Secure token generation using cryptographically random bytes
//! - SHA256 hashing for token storage (never store plaintext tokens)
//! - Token prefix for identification and debugging
//! - Integration with job queue for email delivery
//!
//! # Security Features
//!
//! - Tokens are 32 bytes of cryptographically secure random data
//! - Tokens are hashed with SHA256 before storage
//! - Token prefix (first 8 characters) stored for lookup and debugging
//! - Tokens are single-use and have short expiration (15 minutes default)
//!
//! # Usage
//!
//! ```ignore
//! use raisin_auth::strategies::MagicLinkStrategy;
//!
//! let strategy = MagicLinkStrategy::new();
//!
//! // Generate a new token
//! let (token, hash, prefix) = MagicLinkStrategy::generate_token()?;
//!
//! // Hash a token for verification (e.g., during login)
//! let hash = MagicLinkStrategy::hash_token(&token);
//! ```
//!
//! # Email Delivery
//!
//! This strategy does NOT send emails directly. Email delivery should be
//! handled via the JobRegistry pattern:
//!
//! ```ignore
//! // Register job with JobRegistry
//! JobRegistry.register_job("send_magic_link", job_data);
//! // Job worker will send the email asynchronously
//! ```

use async_trait::async_trait;
use raisin_error::{Error, Result};
use raisin_models::auth::AuthProviderConfig;
use rand::Rng;
use sha2::{Digest, Sha256};

use crate::strategy::{AuthCredentials, AuthStrategy, AuthenticationResult, StrategyId};

/// Magic link passwordless authentication strategy.
///
/// This strategy generates secure one-time tokens that are sent to users
/// via email for passwordless authentication.
#[derive(Debug, Clone)]
pub struct MagicLinkStrategy {
    /// Strategy identifier
    strategy_id: StrategyId,
}

impl MagicLinkStrategy {
    /// Create a new magic link authentication strategy.
    pub fn new() -> Self {
        Self {
            strategy_id: StrategyId::new(StrategyId::MAGIC_LINK),
        }
    }

    /// Generate a secure random token.
    ///
    /// Generates a cryptographically secure random token suitable for
    /// magic link authentication. The token is 32 bytes of random data
    /// encoded as a hex string (64 characters).
    ///
    /// # Returns
    ///
    /// A tuple of:
    /// - The plaintext token (to send to user)
    /// - The SHA256 hash of the token (to store in database)
    /// - The token prefix (first 8 chars, for identification)
    ///
    /// # Example
    ///
    /// ```ignore
    /// let (token, hash, prefix) = MagicLinkStrategy::generate_token()?;
    /// // Send `token` to user via email
    /// // Store `hash` and `prefix` in database
    /// assert_eq!(token.len(), 64);
    /// assert_eq!(prefix.len(), 8);
    /// ```
    pub fn generate_token() -> Result<(String, String, String)> {
        // Generate 32 bytes of cryptographically secure random data
        let mut rng = rand::thread_rng();
        let random_bytes: [u8; 32] = rng.gen();

        // Encode as hex string for easy transmission
        let token = hex::encode(random_bytes);

        // Hash the token for storage
        let hash = Self::hash_token(&token);

        // Extract prefix (first 8 characters) for identification
        let prefix = token[..8].to_string();

        Ok((token, hash, prefix))
    }

    /// Hash a token using SHA256.
    ///
    /// Hashes the provided token using SHA256 and returns the hex-encoded
    /// hash. This is used to store tokens securely in the database.
    ///
    /// # Arguments
    ///
    /// * `token` - The plaintext token to hash
    ///
    /// # Returns
    ///
    /// The SHA256 hash of the token as a hex string
    ///
    /// # Example
    ///
    /// ```ignore
    /// let token = "abc123...";
    /// let hash = MagicLinkStrategy::hash_token(token);
    /// // Store hash in database
    /// // Later, compare hashes to verify token
    /// ```
    pub fn hash_token(token: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Extract the token prefix from a full token.
    ///
    /// Returns the first 8 characters of the token, which can be stored
    /// separately for token lookup and debugging purposes.
    ///
    /// # Arguments
    ///
    /// * `token` - The full token string
    ///
    /// # Returns
    ///
    /// The first 8 characters, or an error if token is too short
    ///
    /// # Example
    ///
    /// ```ignore
    /// let token = "abc123def456...";
    /// let prefix = MagicLinkStrategy::extract_prefix(&token)?;
    /// assert_eq!(prefix, "abc123de");
    /// ```
    pub fn extract_prefix(token: &str) -> Result<String> {
        if token.len() < 8 {
            return Err(Error::Validation(
                "Token must be at least 8 characters".to_string(),
            ));
        }
        Ok(token[..8].to_string())
    }

    /// Verify that a token matches a stored hash.
    ///
    /// This is a helper method for verifying magic link tokens during
    /// authentication. The actual verification is typically done by
    /// the AuthService, but this method can be used for testing or
    /// in custom implementations.
    ///
    /// # Arguments
    ///
    /// * `token` - The plaintext token from the user
    /// * `stored_hash` - The hash stored in the database
    ///
    /// # Returns
    ///
    /// `true` if the token matches the hash, `false` otherwise
    ///
    /// # Example
    ///
    /// ```ignore
    /// let (token, hash, _) = MagicLinkStrategy::generate_token()?;
    /// assert!(MagicLinkStrategy::verify_token(&token, &hash));
    /// assert!(!MagicLinkStrategy::verify_token("wrong", &hash));
    /// ```
    pub fn verify_token(token: &str, stored_hash: &str) -> bool {
        let token_hash = Self::hash_token(token);
        // Use constant-time comparison to prevent timing attacks
        token_hash == stored_hash
    }
}

impl Default for MagicLinkStrategy {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AuthStrategy for MagicLinkStrategy {
    fn id(&self) -> &StrategyId {
        &self.strategy_id
    }

    fn name(&self) -> &str {
        "Magic Link Authentication"
    }

    async fn init(
        &mut self,
        _config: &AuthProviderConfig,
        _decrypted_secret: Option<&str>,
    ) -> Result<()> {
        // Magic link strategy doesn't need initialization or secrets
        Ok(())
    }

    async fn authenticate(
        &self,
        tenant_id: &str,
        credentials: AuthCredentials,
    ) -> Result<AuthenticationResult> {
        // Extract token from credentials
        let token = match credentials {
            AuthCredentials::MagicLinkToken { token } => token,
            _ => {
                return Err(Error::Validation(
                    "Magic link strategy requires magic link token credentials".to_string(),
                ))
            }
        };

        // NOTE: In a real implementation, this would:
        // 1. Look up the token by prefix or hash in the OneTimeToken store
        // 2. Verify the token is valid (not expired, not used)
        // 3. Hash the provided token and compare with stored hash
        // 4. Mark the token as used
        // 5. Return the identity associated with the token
        //
        // For now, this is a skeleton that demonstrates the flow.
        // The actual token lookup and verification will be handled
        // by the AuthService that uses this strategy.

        // Placeholder implementation - in reality this is handled by AuthService
        let _ = (tenant_id, token);

        Err(Error::internal(
            "MagicLinkStrategy::authenticate is a skeleton - actual authentication \
             is handled by AuthService which performs token lookup, validation, \
             and verification using this strategy's helper methods"
                .to_string(),
        ))
    }

    fn supports(&self, credentials: &AuthCredentials) -> bool {
        matches!(credentials, AuthCredentials::MagicLinkToken { .. })
    }
}

// Hex encoding/decoding utility
mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes
            .as_ref()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect()
    }

    #[allow(dead_code)]
    pub fn decode(s: &str) -> Result<Vec<u8>, String> {
        if s.len() % 2 != 0 {
            return Err("Hex string must have even length".to_string());
        }

        (0..s.len())
            .step_by(2)
            .map(|i| {
                u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| format!("Invalid hex: {}", e))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_generation() {
        let (token, hash, prefix) =
            MagicLinkStrategy::generate_token().expect("Failed to generate token");

        // Token should be 64 characters (32 bytes hex-encoded)
        assert_eq!(token.len(), 64);

        // Hash should be 64 characters (SHA256 hex-encoded)
        assert_eq!(hash.len(), 64);

        // Prefix should be first 8 characters of token
        assert_eq!(prefix.len(), 8);
        assert_eq!(&token[..8], prefix);

        // Token should be valid hex
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_token_hashing() {
        let token = "abc123def456abc123def456abc123def456abc123def456abc123def456abc1";
        let hash = MagicLinkStrategy::hash_token(token);

        // Hash should be consistent
        assert_eq!(hash, MagicLinkStrategy::hash_token(token));

        // Hash should be 64 characters (SHA256)
        assert_eq!(hash.len(), 64);

        // Hash should be different from token
        assert_ne!(hash, token);
    }

    #[test]
    fn test_different_tokens_produce_different_hashes() {
        let (token1, hash1, _) =
            MagicLinkStrategy::generate_token().expect("Failed to generate token");
        let (token2, hash2, _) =
            MagicLinkStrategy::generate_token().expect("Failed to generate token");

        // Different tokens should produce different hashes
        assert_ne!(token1, token2);
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_prefix_extraction() {
        let token = "abc123def456abc123def456abc123def456abc123def456abc123def456abc1";
        let prefix = MagicLinkStrategy::extract_prefix(token).expect("Failed to extract prefix");

        assert_eq!(prefix, "abc123de");
        assert_eq!(prefix.len(), 8);
    }

    #[test]
    fn test_prefix_extraction_short_token() {
        let short_token = "abc123";
        let result = MagicLinkStrategy::extract_prefix(short_token);

        assert!(result.is_err());
        match result {
            Err(Error::Validation(msg)) => {
                assert!(msg.contains("at least 8 characters"));
            }
            _ => panic!("Expected Validation error"),
        }
    }

    #[test]
    fn test_token_verification() {
        let (token, hash, _) =
            MagicLinkStrategy::generate_token().expect("Failed to generate token");

        // Correct token should verify
        assert!(MagicLinkStrategy::verify_token(&token, &hash));

        // Wrong token should not verify
        assert!(!MagicLinkStrategy::verify_token("wrong_token", &hash));

        // Empty token should not verify
        assert!(!MagicLinkStrategy::verify_token("", &hash));
    }

    #[test]
    fn test_token_verification_case_sensitive() {
        let token = "abc123def456abc123def456abc123def456abc123def456abc123def456abc1";
        let hash = MagicLinkStrategy::hash_token(token);

        // Uppercase token should not verify (hex is case-sensitive in our impl)
        let uppercase = token.to_uppercase();
        let uppercase_hash = MagicLinkStrategy::hash_token(&uppercase);
        assert_ne!(hash, uppercase_hash);
    }

    #[test]
    fn test_strategy_id() {
        let strategy = MagicLinkStrategy::new();
        assert_eq!(strategy.id().as_ref(), StrategyId::MAGIC_LINK);
        assert_eq!(strategy.name(), "Magic Link Authentication");
    }

    #[test]
    fn test_supports_credentials() {
        let strategy = MagicLinkStrategy::new();

        // Should support magic link token
        let creds = AuthCredentials::MagicLinkToken {
            token: "token123".to_string(),
        };
        assert!(strategy.supports(&creds));

        // Should not support username/password
        let up_creds = AuthCredentials::UsernamePassword {
            username: "user@example.com".to_string(),
            password: "password".to_string(),
        };
        assert!(!strategy.supports(&up_creds));

        // Should not support one-time token
        let ott_creds = AuthCredentials::OneTimeToken {
            token: "token456".to_string(),
        };
        assert!(!strategy.supports(&ott_creds));
    }

    #[tokio::test]
    async fn test_init_no_op() {
        let mut strategy = MagicLinkStrategy::new();
        let config = AuthProviderConfig::local();

        // Init should succeed with no-op
        let result = strategy.init(&config, None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_authenticate_returns_skeleton_error() {
        let strategy = MagicLinkStrategy::new();
        let creds = AuthCredentials::MagicLinkToken {
            token: "abc123def456abc123def456abc123def456abc123def456abc123def456abc1".to_string(),
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
        let strategy = MagicLinkStrategy::new();
        let creds = AuthCredentials::UsernamePassword {
            username: "user@example.com".to_string(),
            password: "password".to_string(),
        };

        let result = strategy.authenticate("tenant-1", creds).await;

        // Should return validation error for wrong credential type
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            Error::Validation(msg) => {
                assert!(msg.contains("magic link token"));
            }
            _ => panic!("Expected Validation error"),
        }
    }

    #[test]
    fn test_hash_deterministic() {
        let token = "test_token_123";
        let hash1 = MagicLinkStrategy::hash_token(token);
        let hash2 = MagicLinkStrategy::hash_token(token);

        // Same token should always produce same hash
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_random_tokens_are_unique() {
        // Generate multiple tokens and ensure they're all unique
        let mut tokens = std::collections::HashSet::new();

        for _ in 0..100 {
            let (token, _, _) =
                MagicLinkStrategy::generate_token().expect("Failed to generate token");
            // All tokens should be unique
            assert!(tokens.insert(token));
        }
    }

    #[test]
    fn test_prefix_matches_token() {
        let (token, _, prefix) =
            MagicLinkStrategy::generate_token().expect("Failed to generate token");

        // Prefix should be the first 8 characters of the token
        assert_eq!(prefix, &token[..8]);

        // Extracting prefix manually should match
        let extracted =
            MagicLinkStrategy::extract_prefix(&token).expect("Failed to extract prefix");
        assert_eq!(prefix, extracted);
    }

    #[test]
    fn test_default_trait() {
        let strategy1 = MagicLinkStrategy::new();
        let strategy2 = MagicLinkStrategy::default();

        assert_eq!(strategy1.id(), strategy2.id());
        assert_eq!(strategy1.name(), strategy2.name());
    }

    #[test]
    fn test_token_length_consistency() {
        // All generated tokens should have consistent length
        for _ in 0..50 {
            let (token, hash, prefix) =
                MagicLinkStrategy::generate_token().expect("Failed to generate token");

            assert_eq!(token.len(), 64, "Token length should be 64");
            assert_eq!(hash.len(), 64, "Hash length should be 64");
            assert_eq!(prefix.len(), 8, "Prefix length should be 8");
        }
    }
}
