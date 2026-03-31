// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Token generation, hashing, and verification operations.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use raisin_error::{Error, Result};
use rand::RngCore;
use sha2::{Digest, Sha256};

use super::OneTimeTokenStrategy;

/// Default token length in bytes (before base64 encoding)
pub const DEFAULT_TOKEN_BYTES: usize = 32;

/// Minimum safe token length in bytes
pub const MIN_TOKEN_BYTES: usize = 16;

impl OneTimeTokenStrategy {
    /// Generate a new cryptographically secure token.
    ///
    /// Returns both the plaintext token (to give to the user) and the
    /// hash (to store in the database).
    ///
    /// # Arguments
    ///
    /// * `prefix` - Token prefix for identification (e.g., "rdb_api", "rdb_inv")
    ///
    /// # Returns
    ///
    /// A tuple of `(plaintext_token, token_hash)` on success.
    pub fn generate_token(prefix: &str) -> Result<(String, String)> {
        Self::generate_token_with_length(prefix, DEFAULT_TOKEN_BYTES)
    }

    /// Generate a token with a custom length.
    ///
    /// # Arguments
    ///
    /// * `prefix` - Token prefix for identification
    /// * `length_bytes` - Number of random bytes to generate (min 16)
    pub fn generate_token_with_length(
        prefix: &str,
        length_bytes: usize,
    ) -> Result<(String, String)> {
        if length_bytes < MIN_TOKEN_BYTES {
            return Err(Error::Validation(format!(
                "Token length must be at least {} bytes for security",
                MIN_TOKEN_BYTES
            )));
        }

        let mut token_bytes = vec![0u8; length_bytes];
        rand::thread_rng().fill_bytes(&mut token_bytes);

        let token_b64 = URL_SAFE_NO_PAD.encode(&token_bytes);

        let plaintext = if prefix.is_empty() {
            token_b64
        } else {
            format!("{}_{}", prefix, token_b64)
        };

        let hash = Self::hash_token(&plaintext);

        Ok((plaintext, hash))
    }

    /// Hash a token for secure storage using SHA-256.
    pub fn hash_token(token: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Verify a token against its stored hash using constant-time comparison.
    pub fn verify_token(token: &str, stored_hash: &str) -> bool {
        let computed_hash = Self::hash_token(token);
        Self::constant_time_compare(&computed_hash, stored_hash)
    }

    /// Extract the token prefix from a full token.
    ///
    /// Returns everything before the first underscore.
    pub fn extract_prefix(token: &str) -> &str {
        if let Some(pos) = token.find('_') {
            &token[..pos]
        } else {
            ""
        }
    }

    /// Constant-time string comparison to prevent timing attacks.
    pub(super) fn constant_time_compare(a: &str, b: &str) -> bool {
        if a.len() != b.len() {
            return false;
        }

        let a_bytes = a.as_bytes();
        let b_bytes = b.as_bytes();

        let mut result = 0u8;
        for i in 0..a_bytes.len() {
            result |= a_bytes[i] ^ b_bytes[i];
        }

        result == 0
    }
}
