//! Password hashing, verification, generation, and strength validation

use bcrypt::{hash, verify, DEFAULT_COST};
use raisin_error::Result;

use super::AuthService;

impl AuthService {
    /// Hash a password using bcrypt
    pub fn hash_password(password: &str) -> Result<String> {
        hash(password, DEFAULT_COST)
            .map_err(|e| raisin_error::Error::Backend(format!("Failed to hash password: {}", e)))
    }

    /// Verify a password against a bcrypt hash
    pub fn verify_password(password: &str, hash: &str) -> Result<bool> {
        verify(password, hash)
            .map_err(|e| raisin_error::Error::Backend(format!("Failed to verify password: {}", e)))
    }

    /// Generate a random password (16 characters)
    pub fn generate_password() -> String {
        use rand::Rng;
        const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ\
                                  abcdefghijklmnopqrstuvwxyz\
                                  0123456789\
                                  !@#$%^&*";
        let mut rng = rand::thread_rng();
        (0..16)
            .map(|_| {
                let idx = rng.gen_range(0..CHARSET.len());
                CHARSET[idx] as char
            })
            .collect()
    }

    /// Validate password strength
    pub(super) fn validate_password_strength(password: &str) -> Result<()> {
        if password.len() < 12 {
            return Err(raisin_error::Error::Validation(
                "Password must be at least 12 characters long".to_string(),
            ));
        }

        let has_uppercase = password.chars().any(|c| c.is_uppercase());
        let has_lowercase = password.chars().any(|c| c.is_lowercase());
        let has_digit = password.chars().any(|c| c.is_numeric());
        let has_special = password.chars().any(|c| !c.is_alphanumeric());

        if !has_uppercase || !has_lowercase || !has_digit || !has_special {
            return Err(raisin_error::Error::Validation(
                "Password must contain uppercase, lowercase, digit, and special character"
                    .to_string(),
            ));
        }

        Ok(())
    }
}
