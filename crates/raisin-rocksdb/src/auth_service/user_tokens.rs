//! User JWT token generation, validation, and refresh

use super::{AdminClaims, AuthService};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use raisin_error::Result;
use raisin_models::auth::{
    AuthClaims, AuthTokens, GlobalFlags, Identity, RefreshClaims, Session, TokenType,
};

impl AuthService {
    /// Generate user JWT tokens (access + refresh) for an authenticated identity.
    ///
    /// Returns `AuthTokens` containing both tokens and expiry information.
    ///
    /// # Arguments
    /// * `identity` - The authenticated identity
    /// * `session` - The session information
    /// * `repository` - Optional repository ID for repo-scoped authentication
    /// * `home` - Optional user home path (raisin:User node path)
    pub fn generate_user_tokens(
        &self,
        identity: &Identity,
        session: &Session,
        repository: Option<String>,
        home: Option<String>,
    ) -> Result<AuthTokens> {
        let now = Utc::now();
        let access_expiry = now + Duration::hours(1); // 1 hour access token
        let refresh_expiry = now + Duration::days(30); // 30 days refresh token

        // Generate access token
        let access_claims = AuthClaims {
            sub: identity.identity_id.clone(),
            email: identity.email.clone(),
            tenant_id: identity.tenant_id.clone(),
            repository,
            home,
            sid: session.session_id.clone(),
            auth_strategy: session.strategy_id.clone(),
            auth_time: now.timestamp(),
            global_flags: GlobalFlags {
                is_tenant_admin: false, // Will be determined by workspace roles
                email_verified: identity.email_verified,
                must_change_password: identity
                    .local_credentials
                    .as_ref()
                    .map(|c| c.must_change_password)
                    .unwrap_or(false),
            },
            token_type: TokenType::Access,
            exp: access_expiry.timestamp(),
            iat: now.timestamp(),
            nbf: Some(now.timestamp()),
            jti: uuid::Uuid::new_v4().to_string(),
            iss: Some("raisindb".to_string()),
            aud: None,
        };

        let access_token = encode(
            &Header::default(),
            &access_claims,
            &EncodingKey::from_secret(self.jwt_secret.as_bytes()),
        )
        .map_err(|e| {
            raisin_error::Error::Backend(format!("Failed to generate access token: {}", e))
        })?;

        // Generate refresh token (includes home for preservation across refreshes)
        let refresh_claims = RefreshClaims {
            sub: identity.identity_id.clone(),
            sid: session.session_id.clone(),
            tenant_id: identity.tenant_id.clone(),
            iat: now.timestamp(),
            exp: refresh_expiry.timestamp(),
            jti: uuid::Uuid::new_v4().to_string(),
            family: session.token_family.clone(),
            generation: session.token_generation,
            home: access_claims.home.clone(),
        };

        let refresh_token = encode(
            &Header::default(),
            &refresh_claims,
            &EncodingKey::from_secret(self.jwt_secret.as_bytes()),
        )
        .map_err(|e| {
            raisin_error::Error::Backend(format!("Failed to generate refresh token: {}", e))
        })?;

        Ok(AuthTokens {
            access_token,
            refresh_token,
            token_type: "Bearer".to_string(),
            expires_in: 3600,                         // 1 hour in seconds
            refresh_expires_in: Some(30 * 24 * 3600), // 30 days in seconds
        })
    }

    /// Validate a user access token and extract claims.
    ///
    /// Returns the `AuthClaims` if the token is valid.
    pub fn validate_user_token(&self, token: &str) -> Result<AuthClaims> {
        let token_data = decode::<AuthClaims>(
            token,
            &DecodingKey::from_secret(self.jwt_secret.as_bytes()),
            &Validation::default(),
        )
        .map_err(|e| raisin_error::Error::Unauthorized(format!("Invalid user token: {}", e)))?;

        let claims = token_data.claims;

        // Verify it's an access token (not refresh or admin)
        if !matches!(
            claims.token_type,
            TokenType::Access | TokenType::Impersonation { .. }
        ) {
            return Err(raisin_error::Error::Unauthorized(
                "Expected access token, got different token type".to_string(),
            ));
        }

        Ok(claims)
    }

    /// Validate a refresh token and extract claims.
    ///
    /// Returns the `RefreshClaims` if the token is valid.
    pub fn validate_refresh_token(&self, token: &str) -> Result<RefreshClaims> {
        let token_data = decode::<RefreshClaims>(
            token,
            &DecodingKey::from_secret(self.jwt_secret.as_bytes()),
            &Validation::default(),
        )
        .map_err(|e| raisin_error::Error::Unauthorized(format!("Invalid refresh token: {}", e)))?;

        Ok(token_data.claims)
    }

    /// Generate a new access token from a valid refresh token.
    ///
    /// This rotates the refresh token (increments generation) for security.
    /// The session should be updated with the new generation.
    ///
    /// # Arguments
    /// * `identity` - The authenticated identity
    /// * `session` - The session information
    /// * `old_refresh_claims` - The claims from the refresh token being used
    /// * `home` - Optional user home path (raisin:User node path)
    pub fn refresh_user_tokens(
        &self,
        identity: &Identity,
        session: &Session,
        old_refresh_claims: &RefreshClaims,
        home: Option<String>,
    ) -> Result<(AuthTokens, u32)> {
        let now = Utc::now();
        let access_expiry = now + Duration::hours(1);
        let refresh_expiry = now + Duration::days(30);

        // Verify the refresh token belongs to this session family
        if old_refresh_claims.family != session.token_family {
            return Err(raisin_error::Error::Unauthorized(
                "Token family mismatch - possible token reuse attack".to_string(),
            ));
        }

        // Verify generation matches (detect token replay)
        if old_refresh_claims.generation != session.token_generation {
            return Err(raisin_error::Error::Unauthorized(
                "Token generation mismatch - possible token replay".to_string(),
            ));
        }

        let new_generation = session.token_generation + 1;

        // Generate new access token
        // Note: repository is None for refreshed tokens - they rely on connection context
        let access_claims = AuthClaims {
            sub: identity.identity_id.clone(),
            email: identity.email.clone(),
            tenant_id: identity.tenant_id.clone(),
            repository: None, // Refreshed tokens don't preserve repository scope
            home,
            sid: session.session_id.clone(),
            auth_strategy: session.strategy_id.clone(),
            auth_time: now.timestamp(),
            global_flags: GlobalFlags {
                is_tenant_admin: false,
                email_verified: identity.email_verified,
                must_change_password: identity
                    .local_credentials
                    .as_ref()
                    .map(|c| c.must_change_password)
                    .unwrap_or(false),
            },
            token_type: TokenType::Access,
            exp: access_expiry.timestamp(),
            iat: now.timestamp(),
            nbf: Some(now.timestamp()),
            jti: uuid::Uuid::new_v4().to_string(),
            iss: Some("raisindb".to_string()),
            aud: None,
        };

        let access_token = encode(
            &Header::default(),
            &access_claims,
            &EncodingKey::from_secret(self.jwt_secret.as_bytes()),
        )
        .map_err(|e| {
            raisin_error::Error::Backend(format!("Failed to generate access token: {}", e))
        })?;

        // Generate new refresh token with incremented generation (preserves home)
        let refresh_claims = RefreshClaims {
            sub: identity.identity_id.clone(),
            sid: session.session_id.clone(),
            tenant_id: identity.tenant_id.clone(),
            iat: now.timestamp(),
            exp: refresh_expiry.timestamp(),
            jti: uuid::Uuid::new_v4().to_string(),
            family: session.token_family.clone(),
            generation: new_generation,
            home: access_claims.home.clone(),
        };

        let refresh_token = encode(
            &Header::default(),
            &refresh_claims,
            &EncodingKey::from_secret(self.jwt_secret.as_bytes()),
        )
        .map_err(|e| {
            raisin_error::Error::Backend(format!("Failed to generate refresh token: {}", e))
        })?;

        Ok((
            AuthTokens {
                access_token,
                refresh_token,
                token_type: "Bearer".to_string(),
                expires_in: 3600,
                refresh_expires_in: Some(30 * 24 * 3600),
            },
            new_generation,
        ))
    }

    /// Try to validate a token as either admin or user.
    ///
    /// Returns `Ok(Left(AdminClaims))` for admin tokens,
    /// `Ok(Right(AuthClaims))` for user tokens, or error if invalid.
    pub fn validate_any_token(
        &self,
        token: &str,
    ) -> Result<either::Either<AdminClaims, AuthClaims>> {
        // Try admin token first
        if let Ok(admin_claims) = self.validate_token(token) {
            return Ok(either::Either::Left(admin_claims));
        }

        // Try user token
        let user_claims = self.validate_user_token(token)?;
        Ok(either::Either::Right(user_claims))
    }
}
