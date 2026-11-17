// SPDX-License-Identifier: BSL-1.1

//! API key validation trait and mock implementation.

use async_trait::async_trait;
use raisin_models::auth::AuthContext;

/// A validated key, with what a SCOPED key is limited to.
#[derive(Debug, Clone)]
pub struct ValidatedApiKey {
    pub user_id: String,
    pub tenant_id: String,
    /// `Some(repo)` for a scoped key: it may connect to that repository only.
    pub scope_repo: Option<String>,
    /// A scoped key's RLS context (its roles in `scope_repo`).
    pub key_auth: Option<AuthContext>,
}

/// Trait for validating API keys and checking permissions
///
/// This trait abstracts the authentication backend to allow for testing
/// and future integration with different authentication systems.
#[async_trait]
pub trait ApiKeyValidator: Send + Sync {
    /// Validate an API key and return the associated user_id and tenant_id
    ///
    /// # Returns
    /// * `Ok(Some((user_id, tenant_id)))` - Valid key with user and tenant
    /// * `Ok(None)` - Invalid or revoked key
    /// * `Err(error)` - Storage or other error during validation
    async fn validate_api_key(&self, api_key: &str) -> Result<Option<(String, String)>, String>;

    /// Check if a user has pgwire access permission
    ///
    /// # Returns
    /// * `Ok(true)` - User has pgwire access
    /// * `Ok(false)` - User does not have pgwire access
    /// * `Err(error)` - Error checking permissions
    async fn has_pgwire_access(&self, tenant_id: &str, user_id: &str) -> Result<bool, String>;

    /// Validate a key including its scope. The default treats every key as
    /// unscoped (an admin's own key), which is what a validator that knows
    /// nothing of scopes means. The server's validator overrides it.
    async fn validate_api_key_scoped(
        &self,
        api_key: &str,
    ) -> Result<Option<ValidatedApiKey>, String> {
        Ok(self
            .validate_api_key(api_key)
            .await?
            .map(|(user_id, tenant_id)| ValidatedApiKey {
                user_id,
                tenant_id,
                scope_repo: None,
                key_auth: None,
            }))
    }
}

/// Simple in-memory API key validator for testing
#[cfg(test)]
pub struct MockApiKeyValidator {
    valid_keys: std::collections::HashMap<String, (String, String)>,
    pgwire_access: std::collections::HashMap<(String, String), bool>,
}

#[cfg(test)]
impl MockApiKeyValidator {
    /// Create a new mock validator
    pub fn new() -> Self {
        Self {
            valid_keys: std::collections::HashMap::new(),
            pgwire_access: std::collections::HashMap::new(),
        }
    }

    /// Add a valid API key
    pub fn add_key(&mut self, api_key: String, user_id: String, tenant_id: String) {
        self.valid_keys.insert(api_key, (user_id, tenant_id));
    }

    /// Grant pgwire access to a user
    pub fn grant_pgwire_access(&mut self, tenant_id: String, user_id: String) {
        self.pgwire_access.insert((tenant_id, user_id), true);
    }
}

#[cfg(test)]
#[async_trait]
impl ApiKeyValidator for MockApiKeyValidator {
    async fn validate_api_key(&self, api_key: &str) -> Result<Option<(String, String)>, String> {
        Ok(self.valid_keys.get(api_key).cloned())
    }

    async fn has_pgwire_access(&self, tenant_id: &str, user_id: &str) -> Result<bool, String> {
        Ok(self
            .pgwire_access
            .get(&(tenant_id.to_string(), user_id.to_string()))
            .copied()
            .unwrap_or(false))
    }
}
