//! API key management methods

use super::AuthService;
use raisin_error::Result;
use raisin_models::api_key::{ApiKey, ApiKeyScope, SCOPED_KEY_OWNER};

impl AuthService {
    /// Create a new API key for a user
    ///
    /// Returns (ApiKey, raw_token) - the raw token is only shown once!
    pub fn create_api_key(
        &self,
        tenant_id: &str,
        user_id: &str,
        name: &str,
    ) -> Result<(ApiKey, String)> {
        self.api_key_store.create_api_key(tenant_id, user_id, name)
    }

    /// Create a scoped key (no owner; resolves to `scope.roles` in
    /// `scope.repo`). Role validation is the caller's: it needs the repo.
    pub fn create_scoped_api_key(
        &self,
        tenant_id: &str,
        name: &str,
        scope: ApiKeyScope,
    ) -> Result<(ApiKey, String)> {
        self.api_key_store
            .create_scoped_api_key(tenant_id, name, scope)
    }

    /// The tenant's scoped keys, revoked ones included.
    pub fn list_scoped_api_keys(&self, tenant_id: &str) -> Result<Vec<ApiKey>> {
        self.api_key_store
            .list_user_api_keys(tenant_id, SCOPED_KEY_OWNER)
    }

    /// Revoke a scoped key. `Ok(false)` for an unknown key, which is not an
    /// error to the operator (revoking twice, or a key minted elsewhere).
    pub fn revoke_scoped_api_key(&self, tenant_id: &str, key_id: &str) -> Result<bool> {
        if self
            .api_key_store
            .get_api_key(tenant_id, SCOPED_KEY_OWNER, key_id)?
            .is_none()
        {
            return Ok(false);
        }
        self.api_key_store
            .revoke_api_key(tenant_id, SCOPED_KEY_OWNER, key_id)?;
        Ok(true)
    }

    /// List all API keys for a user
    pub fn list_user_api_keys(&self, tenant_id: &str, user_id: &str) -> Result<Vec<ApiKey>> {
        self.api_key_store.list_user_api_keys(tenant_id, user_id)
    }

    /// Revoke an API key
    pub fn revoke_api_key(&self, tenant_id: &str, user_id: &str, key_id: &str) -> Result<()> {
        self.api_key_store
            .revoke_api_key(tenant_id, user_id, key_id)
    }

    /// Validate an API key and return the associated ApiKey if valid
    ///
    /// This also updates the last_used_at timestamp
    pub fn validate_api_key(&self, raw_token: &str) -> Result<Option<ApiKey>> {
        self.api_key_store.validate_api_key(raw_token)
    }

    /// Get a specific API key by ID
    pub fn get_api_key(
        &self,
        tenant_id: &str,
        user_id: &str,
        key_id: &str,
    ) -> Result<Option<ApiKey>> {
        self.api_key_store.get_api_key(tenant_id, user_id, key_id)
    }
}
