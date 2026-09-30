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

    /// WHO the token speaks for, and therefore what it may become.
    ///
    /// A valid signature proves only that this service minted the token, not
    /// that its holder is an administrator. Anonymous WebSocket connections
    /// used to receive a token from this same service, and the upgrade path
    /// turned every valid access token into `AuthContext::system()`, so a
    /// visitor could reconnect with its own anonymous token and bypass
    /// row-level security. The principal is now part of the signed claims and
    /// the upgrade path grants system only for [`TokenPrincipal::Admin`].
    ///
    /// Defaulted so tokens minted before this claim existed still decode, as
    /// [`TokenPrincipal::Unscoped`]: they grant nothing, and their holder signs
    /// in again.
    #[serde(default)]
    pub principal: TokenPrincipal,
}

/// The principal a WebSocket JWT was issued to.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TokenPrincipal {
    /// An administrator who signed in with admin credentials.
    Admin,
    /// No claim (a token minted before the claim existed). Grants nothing.
    #[default]
    Unscoped,
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

    /// Generate a token pair (access + refresh) for an ADMINISTRATOR who has
    /// just proven admin credentials.
    ///
    /// The only issuer of admin tokens. Never call it for anyone else: an admin
    /// token becomes `AuthContext::system()` on the WebSocket upgrade.
    pub fn generate_admin_token_pair(
        &self,
        user_id: String,
        tenant_id: String,
        repository: Option<String>,
    ) -> Result<TokenPair, AuthError> {
        self.issue(user_id, tenant_id, repository, TokenPrincipal::Admin)
    }

    fn issue(
        &self,
        user_id: String,
        tenant_id: String,
        repository: Option<String>,
        principal: TokenPrincipal,
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
            principal,
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
            principal,
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

        // A refresh keeps the principal it was issued with; it never upgrades.
        self.issue(
            claims.sub,
            claims.tenant_id,
            claims.repository,
            claims.principal,
        )
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
            .generate_admin_token_pair(
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
            .generate_admin_token_pair("user123".to_string(), "tenant1".to_string(), None)
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
    fn admin_tokens_carry_the_admin_principal_and_refresh_keeps_it() {
        let auth_service = JwtAuthService::new("test_secret_key_1234567890");
        let pair = auth_service
            .generate_admin_token_pair("admin".to_string(), "tenant1".to_string(), None)
            .unwrap();
        let claims = auth_service
            .validate_access_token(&pair.access_token)
            .unwrap();
        assert_eq!(claims.principal, TokenPrincipal::Admin);

        let refreshed = auth_service
            .refresh_access_token(&pair.refresh_token)
            .unwrap();
        let claims = auth_service
            .validate_access_token(&refreshed.access_token)
            .unwrap();
        assert_eq!(claims.principal, TokenPrincipal::Admin);
    }

    /// A token signed by this service but without a principal claim — what
    /// anonymous connections used to be handed — decodes as Unscoped, and a
    /// refresh cannot turn it into an admin token.
    #[test]
    fn a_token_without_a_principal_claim_is_unscoped() {
        let secret = "test_secret_key_1234567890";
        let auth_service = JwtAuthService::new(secret);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let legacy = serde_json::json!({
            "sub": "anonymous-node-id",
            "tenant_id": "tenant1",
            "iat": now,
            "exp": now + 3600,
            "token_type": "refresh",
        });
        let token = encode(
            &Header::default(),
            &legacy,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap();
        let claims = auth_service.validate_refresh_token(&token).unwrap();
        assert_eq!(claims.principal, TokenPrincipal::Unscoped);

        let refreshed = auth_service.refresh_access_token(&token).unwrap();
        let claims = auth_service
            .validate_access_token(&refreshed.access_token)
            .unwrap();
        assert_eq!(claims.principal, TokenPrincipal::Unscoped);
    }

    #[test]
    fn test_wrong_token_type() {
        let auth_service = JwtAuthService::new("test_secret_key_1234567890");

        let token_pair = auth_service
            .generate_admin_token_pair("user123".to_string(), "tenant1".to_string(), None)
            .unwrap();

        // Try to validate refresh token as access token
        let result = auth_service.validate_access_token(&token_pair.refresh_token);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AuthError::WrongTokenType));
    }
}
