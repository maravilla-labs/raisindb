//! Password operations for identities.
//!
//! Provides password hashing, verification, and login attempt tracking.

use super::IdentityRepository;
use crate::keys;
use raisin_auth::strategies::LocalStrategy;
use raisin_error::Result;
use raisin_models::auth::LocalCredentials;
use tracing::info;

impl IdentityRepository {
    /// Hash a password for storage.
    ///
    /// Uses bcrypt with the default cost factor.
    pub fn hash_password(password: &str) -> Result<String> {
        LocalStrategy::hash_password(password)
    }

    /// Verify a password against an identity's stored credentials.
    ///
    /// Returns:
    /// - `Ok(true)` if password matches
    /// - `Ok(false)` if password doesn't match
    /// - `Err(_)` if identity not found or has no local credentials
    pub async fn verify_password(
        &self,
        tenant_id: &str,
        identity_id: &str,
        password: &str,
    ) -> Result<bool> {
        let identity = self.get(tenant_id, identity_id).await?.ok_or_else(|| {
            raisin_error::Error::NotFound(format!("Identity {} not found", identity_id))
        })?;

        let credentials = identity.local_credentials.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation(format!(
                "Identity {} has no local credentials",
                identity_id
            ))
        })?;

        // Check if account is locked
        if credentials.is_locked() {
            return Err(raisin_error::Error::Validation(
                "Account is temporarily locked due to too many failed login attempts".to_string(),
            ));
        }

        Ok(LocalStrategy::verify_password(
            password,
            &credentials.password_hash,
        ))
    }

    /// Set local credentials for an identity.
    ///
    /// This hashes the password and stores it with the identity.
    pub async fn set_password(
        &self,
        tenant_id: &str,
        identity_id: &str,
        password: &str,
        require_change: bool,
        actor: &str,
    ) -> Result<()> {
        let mut identity = self.get(tenant_id, identity_id).await?.ok_or_else(|| {
            raisin_error::Error::NotFound(format!("Identity {} not found", identity_id))
        })?;

        let password_hash = Self::hash_password(password)?;

        if require_change {
            identity.local_credentials =
                Some(LocalCredentials::new_with_change_required(password_hash));
        } else {
            identity.local_credentials = Some(LocalCredentials::new(password_hash));
        }

        self.upsert(tenant_id, &identity, actor).await
    }

    /// Record a failed login attempt for an identity.
    ///
    /// This increments the failed attempt counter and may lock the account.
    pub async fn record_failed_login(
        &self,
        tenant_id: &str,
        identity_id: &str,
        lockout_threshold: u32,
        lockout_duration_minutes: u64,
    ) -> Result<()> {
        let mut identity = self.get(tenant_id, identity_id).await?.ok_or_else(|| {
            raisin_error::Error::NotFound(format!("Identity {} not found", identity_id))
        })?;

        if let Some(ref mut credentials) = identity.local_credentials {
            credentials.record_failed_attempt(lockout_threshold, lockout_duration_minutes);
            let attempts = credentials.failed_attempts;

            // Update identity directly without capturing to oplog (this is a transient state)
            let cf = self.cf_identities()?;
            let key = keys::identity_key(tenant_id, identity_id);
            let bytes = rmp_serde::to_vec_named(&identity)
                .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

            self.db.put_cf(cf, &key, &bytes).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to update identity: {}", e))
            })?;

            info!(
                "Recorded failed login attempt for identity {} (attempts: {})",
                identity_id, attempts
            );
        }

        Ok(())
    }

    /// Record a successful login for an identity.
    ///
    /// This resets the failed attempt counter and updates last login time.
    pub async fn record_successful_login(
        &self,
        tenant_id: &str,
        identity_id: &str,
        actor: &str,
    ) -> Result<()> {
        let mut identity = self.get(tenant_id, identity_id).await?.ok_or_else(|| {
            raisin_error::Error::NotFound(format!("Identity {} not found", identity_id))
        })?;

        // Reset failed attempts
        if let Some(ref mut credentials) = identity.local_credentials {
            credentials.reset_failed_attempts();
        }

        // Record login time
        identity.record_login();

        self.upsert(tenant_id, &identity, actor).await
    }
}
