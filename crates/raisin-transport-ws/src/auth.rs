// SPDX-License-Identifier: BSL-1.1

//! JWT Authentication for WebSocket connections
//!
//! This module handles JWT token generation, validation, and extraction
//! from WebSocket connections.

use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

/// JWT claims structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Subject (user ID)
    pub sub: String,

    /// Tenant ID
    pub tenant_id: String,

    /// Repository (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,

    /// Issued at (timestamp)
    pub iat: i64,

    /// Expiration time (timestamp)
    pub exp: i64,

    /// Token type (access or refresh)
    pub token_type: TokenType,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TokenType {
    Access,
    Refresh,
}

/// JWT token pair
#[derive(Debug, Clone)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
}

/// JWT authentication service
pub struct JwtAuthService {
    /// Encoding key for signing tokens
    encoding_key: EncodingKey,

    /// Decoding key for validating tokens
    decoding_key: DecodingKey,

    /// Access token expiration in seconds (default: 1 hour)
    access_token_expiration: i64,

    /// Refresh token expiration in seconds (default: 7 days)
    refresh_token_expiration: i64,
}

impl JwtAuthService {
    /// Create a new JWT authentication service with a secret
    pub fn new(secret: &str) -> Self {
        Self {
            encoding_key: EncodingKey::from_secret(secret.as_bytes()),
            decoding_key: DecodingKey::from_secret(secret.as_bytes()),
            access_token_expiration: 3600,    // 1 hour
            refresh_token_expiration: 604800, // 7 days
        }
    }

    /// Create a new JWT authentication service with custom expiration times
    pub fn with_expiration(
        secret: &str,
        access_token_expiration: i64,
        refresh_token_expiration: i64,
    ) -> Self {
        Self {
            encoding_key: EncodingKey::from_secret(secret.as_bytes()),
            decoding_key: DecodingKey::from_secret(secret.as_bytes()),
            access_token_expiration,
            refresh_token_expiration,
        }
    }

    /// Generate a token pair (access + refresh) for a user
    pub fn generate_token_pair(
        &self,
        user_id: String,
        tenant_id: String,
        repository: Option<String>,
    ) -> Result<TokenPair, AuthError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| AuthError::SystemTimeError)?
            .as_secs() as i64;

        // Generate access token
        let access_claims = Claims {
            sub: user_id.clone(),
            tenant_id: tenant_id.clone(),
            repository: repository.clone(),
            iat: now,
            exp: now + self.access_token_expiration,
            token_type: TokenType::Access,
        };

        let access_token = encode(&Header::default(), &access_claims, &self.encoding_key)
            .map_err(|e| AuthError::TokenGenerationError(e.to_string()))?;

        // Generate refresh token
        let refresh_claims = Claims {
            sub: user_id,
            tenant_id,
            repository,
            iat: now,
            exp: now + self.refresh_token_expiration,
            token_type: TokenType::Refresh,
        };

        let refresh_token = encode(&Header::default(), &refresh_claims, &self.encoding_key)
            .map_err(|e| AuthError::TokenGenerationError(e.to_string()))?;

        Ok(TokenPair {
            access_token,
            refresh_token,
            expires_in: self.access_token_expiration,
        })
    }

    /// Validate a token and extract claims
    pub fn validate_token(&self, token: &str) -> Result<Claims, AuthError> {
        let validation = Validation::default();
        let token_data = decode::<Claims>(token, &self.decoding_key, &validation)
            .map_err(|e| AuthError::InvalidToken(e.to_string()))?;

        Ok(token_data.claims)
    }

    /// Validate an access token specifically
    pub fn validate_access_token(&self, token: &str) -> Result<Claims, AuthError> {
        let claims = self.validate_token(token)?;

        if claims.token_type != TokenType::Access {
            return Err(AuthError::WrongTokenType);
        }

        Ok(claims)
    }

    /// Validate a refresh token specifically
    pub fn validate_refresh_token(&self, token: &str) -> Result<Claims, AuthError> {
        let claims = self.validate_token(token)?;

        if claims.token_type != TokenType::Refresh {
            return Err(AuthError::WrongTokenType);
        }

        Ok(claims)
    }

    /// Refresh an access token using a refresh token
    pub fn refresh_access_token(&self, refresh_token: &str) -> Result<TokenPair, AuthError> {
        let claims = self.validate_refresh_token(refresh_token)?;

        self.generate_token_pair(claims.sub, claims.tenant_id, claims.repository)
    }

    /// Extract token from WebSocket headers or query parameters
    pub fn extract_token_from_headers(headers: &axum::http::HeaderMap) -> Option<String> {
        // Try Authorization header first (Bearer token)
        if let Some(auth_header) = headers.get(axum::http::header::AUTHORIZATION) {
            if let Ok(auth_str) = auth_header.to_str() {
                if let Some(token) = auth_str.strip_prefix("Bearer ") {
                    return Some(token.to_string());
                }
            }
        }

        // Try Sec-WebSocket-Protocol header (some clients send token here)
        if let Some(protocol_header) = headers.get("sec-websocket-protocol") {
            if let Ok(protocol_str) = protocol_header.to_str() {
                return Some(protocol_str.to_string());
            }
        }

        None
    }
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("Failed to generate token: {0}")]
    TokenGenerationError(String),

    #[error("Invalid token: {0}")]
    InvalidToken(String),

    #[error("Wrong token type")]
    WrongTokenType,

    #[error("System time error")]
    SystemTimeError,

    #[error("Token expired")]
    TokenExpired,

    #[error("Missing authorization")]
    MissingAuthorization,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_and_validate_token() {
        let auth_service = JwtAuthService::new("test_secret_key_1234567890");

        let token_pair = auth_service
            .generate_token_pair(
                "user123".to_string(),
                "tenant1".to_string(),
                Some("repo1".to_string()),
            )
            .unwrap();

        // Validate access token
        let claims = auth_service
            .validate_access_token(&token_pair.access_token)
            .unwrap();

        assert_eq!(claims.sub, "user123");
        assert_eq!(claims.tenant_id, "tenant1");
        assert_eq!(claims.repository, Some("repo1".to_string()));
        assert_eq!(claims.token_type, TokenType::Access);

        // Validate refresh token
        let refresh_claims = auth_service
            .validate_refresh_token(&token_pair.refresh_token)
            .unwrap();

        assert_eq!(refresh_claims.sub, "user123");
        assert_eq!(refresh_claims.token_type, TokenType::Refresh);
    }

    #[test]
    fn test_refresh_token() {
        let auth_service = JwtAuthService::new("test_secret_key_1234567890");

        let token_pair = auth_service
            .generate_token_pair("user123".to_string(), "tenant1".to_string(), None)
            .unwrap();

        // Refresh the access token
        let new_pair = auth_service
            .refresh_access_token(&token_pair.refresh_token)
            .unwrap();

        // Validate new access token
        let claims = auth_service
            .validate_access_token(&new_pair.access_token)
            .unwrap();

        assert_eq!(claims.sub, "user123");
    }

    #[test]
    fn test_wrong_token_type() {
        let auth_service = JwtAuthService::new("test_secret_key_1234567890");

        let token_pair = auth_service
            .generate_token_pair("user123".to_string(), "tenant1".to_string(), None)
            .unwrap();

        // Try to validate refresh token as access token
        let result = auth_service.validate_access_token(&token_pair.refresh_token);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AuthError::WrongTokenType));
    }
}
