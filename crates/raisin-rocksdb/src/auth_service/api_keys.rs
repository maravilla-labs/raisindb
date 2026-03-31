//! API key management methods

use super::AuthService;
use raisin_error::Result;
use raisin_models::api_key::ApiKey;

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
