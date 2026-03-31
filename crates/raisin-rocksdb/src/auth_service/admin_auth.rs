//! Admin authentication: login, JWT token generation and validation, password change

use super::{AdminClaims, AuthService};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use raisin_error::Result;
use raisin_models::admin_user::{AdminInterface, DatabaseAdminUser};

impl AuthService {
    /// Authenticate a user with username and password
    pub fn authenticate(
        &self,
        tenant_id: &str,
        username: &str,
        password: &str,
        interface: AdminInterface,
    ) -> Result<(DatabaseAdminUser, String)> {
        // DEBUG: Log authentication attempt
        eprintln!(
            "authenticate() called: tenant={}, username={}",
            tenant_id, username
        );

        // Get user from store
        let user_result = self.store.get_user(tenant_id, username)?;
        eprintln!("get_user() returned: {:?}", user_result.is_some());

        let mut user = user_result.ok_or_else(|| {
            eprintln!("User not found in store - returning Unauthorized");
            raisin_error::Error::Unauthorized("Invalid username or password".to_string())
        })?;

        eprintln!(
            "User found: username={}, user_id={}",
            user.username, user.user_id
        );

        // Check if user is active
        eprintln!("Checking if user is active: {}", user.is_active);
        if !user.is_active {
            eprintln!("User account is disabled");
            return Err(raisin_error::Error::Unauthorized(
                "User account is disabled".to_string(),
            ));
        }

        // Verify password
        eprintln!("Verifying password...");
        eprintln!(
            "Password hash from DB: {}...",
            &user.password_hash.chars().take(20).collect::<String>()
        );
        eprintln!("Password provided length: {}", password.len());
        let password_valid = Self::verify_password(password, &user.password_hash)?;
        eprintln!("Password verification result: {}", password_valid);
        if !password_valid {
            eprintln!("Password verification failed");
            return Err(raisin_error::Error::Unauthorized(
                "Invalid username or password".to_string(),
            ));
        }
        eprintln!("Password verified successfully");

        // Check if user has access to the requested interface
        eprintln!("Checking interface access for: {:?}", interface);
        if !user.can_access(interface) {
            eprintln!("User does not have access to interface: {:?}", interface);
            return Err(raisin_error::Error::Forbidden(format!(
                "User does not have {:?} access",
                interface
            )));
        }
        eprintln!("Interface access granted");

        // Record login
        user.record_login();
        self.store.update_user(&user)?;

        // Generate JWT token
        let token = self.generate_token(&user)?;

        Ok((user, token))
    }

    /// Generate a JWT token for a user
    pub fn generate_token(&self, user: &DatabaseAdminUser) -> Result<String> {
        let now = Utc::now();
        let expiry = now + Duration::hours(self.token_expiry_hours);

        let claims = AdminClaims {
            sub: user.user_id.clone(),
            username: user.username.clone(),
            tenant_id: user.tenant_id.clone(),
            access_flags: user.access_flags.clone(),
            must_change_password: user.must_change_password,
            exp: expiry.timestamp(),
            iat: now.timestamp(),
        };

        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.jwt_secret.as_bytes()),
        )
        .map_err(|e| raisin_error::Error::Backend(format!("Failed to generate JWT token: {}", e)))
    }

    /// Validate a JWT token and extract claims
    pub fn validate_token(&self, token: &str) -> Result<AdminClaims> {
        let token_data = decode::<AdminClaims>(
            token,
            &DecodingKey::from_secret(self.jwt_secret.as_bytes()),
            &Validation::default(),
        )
        .map_err(|e| raisin_error::Error::Unauthorized(format!("Invalid token: {}", e)))?;

        Ok(token_data.claims)
    }

    /// Change a user's password
    pub fn change_password(
        &self,
        tenant_id: &str,
        username: &str,
        old_password: &str,
        new_password: &str,
    ) -> Result<()> {
        // Get user
        let mut user = self
            .store
            .get_user(tenant_id, username)?
            .ok_or_else(|| raisin_error::Error::NotFound("User not found".to_string()))?;

        // Verify old password
        if !Self::verify_password(old_password, &user.password_hash)? {
            return Err(raisin_error::Error::Unauthorized(
                "Invalid current password".to_string(),
            ));
        }

        // Validate new password
        Self::validate_password_strength(new_password)?;

        // Hash and update password
        let new_hash = Self::hash_password(new_password)?;
        user.update_password(new_hash);

        // Save updated user
        self.store.update_user(&user)?;

        Ok(())
    }
}
