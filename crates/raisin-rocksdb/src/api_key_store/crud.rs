//! CRUD operations for API key storage.

use super::ApiKeyStore;
use crate::cf;
use raisin_error::Result;
use raisin_models::api_key::ApiKey;

impl ApiKeyStore {
    /// Create a new API key
    ///
    /// Returns the ApiKey and the raw token (raw token only shown once!)
    pub fn create_api_key(
        &self,
        tenant_id: &str,
        user_id: &str,
        name: &str,
    ) -> Result<(ApiKey, String)> {
        let cf = self.db.cf_handle(cf::ADMIN_USERS).ok_or_else(|| {
            raisin_error::Error::Backend("admin_users column family not found".to_string())
        })?;

        // Generate key ID and token
        let key_id = uuid::Uuid::new_v4().to_string();
        let (raw_token, key_hash, key_prefix) = Self::generate_token();

        // Create API key object
        let api_key = ApiKey::new(
            key_id.clone(),
            user_id.to_string(),
            tenant_id.to_string(),
            name.to_string(),
            key_hash.clone(),
            key_prefix,
        );

        // Store the API key
        let key = Self::build_key(tenant_id, user_id, &key_id);
        let value = rmp_serde::to_vec(&api_key).map_err(|e| {
            raisin_error::Error::Backend(format!("Failed to serialize API key: {}", e))
        })?;

        self.db
            .put_cf(cf, &key, &value)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        // Store hash index for fast lookup
        // Value contains tenant_id, user_id, key_id for reverse lookup
        let hash_index_key = Self::build_hash_index_key(&key_hash);
        let hash_index_value = format!("{}\0{}\0{}", tenant_id, user_id, key_id);
        self.db
            .put_cf(cf, &hash_index_key, hash_index_value.as_bytes())
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        Ok((api_key, raw_token))
    }

    /// Get an API key by ID
    pub fn get_api_key(
        &self,
        tenant_id: &str,
        user_id: &str,
        key_id: &str,
    ) -> Result<Option<ApiKey>> {
        let cf = self.db.cf_handle(cf::ADMIN_USERS).ok_or_else(|| {
            raisin_error::Error::Backend("admin_users column family not found".to_string())
        })?;

        let key = Self::build_key(tenant_id, user_id, key_id);

        match self
            .db
            .get_cf(cf, &key)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?
        {
            Some(value) => {
                let api_key: ApiKey = rmp_serde::from_slice(&value).map_err(|e| {
                    raisin_error::Error::Backend(format!("Failed to deserialize API key: {}", e))
                })?;
                Ok(Some(api_key))
            }
            None => Ok(None),
        }
    }

    /// List all API keys for a user
    pub fn list_user_api_keys(&self, tenant_id: &str, user_id: &str) -> Result<Vec<ApiKey>> {
        let cf = self.db.cf_handle(cf::ADMIN_USERS).ok_or_else(|| {
            raisin_error::Error::Backend("admin_users column family not found".to_string())
        })?;

        let prefix = Self::build_user_prefix(tenant_id, user_id);
        let mut keys = Vec::new();

        let iter = self.db.prefix_iterator_cf(cf, &prefix);

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Check if key still matches our prefix
            if !key.starts_with(&prefix) {
                break;
            }

            let api_key: ApiKey = rmp_serde::from_slice(&value).map_err(|e| {
                raisin_error::Error::Backend(format!("Failed to deserialize API key: {}", e))
            })?;

            keys.push(api_key);
        }

        Ok(keys)
    }

    /// Revoke an API key
    pub fn revoke_api_key(&self, tenant_id: &str, user_id: &str, key_id: &str) -> Result<()> {
        let cf = self.db.cf_handle(cf::ADMIN_USERS).ok_or_else(|| {
            raisin_error::Error::Backend("admin_users column family not found".to_string())
        })?;

        let key = Self::build_key(tenant_id, user_id, key_id);

        // Get existing key to update
        let mut api_key = self
            .get_api_key(tenant_id, user_id, key_id)?
            .ok_or_else(|| raisin_error::Error::NotFound("API key not found".to_string()))?;

        // Revoke the key
        api_key.revoke();

        // Update in store
        let value = rmp_serde::to_vec(&api_key).map_err(|e| {
            raisin_error::Error::Backend(format!("Failed to serialize API key: {}", e))
        })?;

        self.db
            .put_cf(cf, &key, &value)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        // Note: We don't delete the hash index - the validation will check is_active

        Ok(())
    }

    /// Validate a raw API token and return the associated ApiKey if valid
    ///
    /// This also updates the last_used_at timestamp
    pub fn validate_api_key(&self, raw_token: &str) -> Result<Option<ApiKey>> {
        let cf = self.db.cf_handle(cf::ADMIN_USERS).ok_or_else(|| {
            raisin_error::Error::Backend("admin_users column family not found".to_string())
        })?;

        // Hash the token
        let key_hash = Self::hash_token(raw_token);

        // Look up the hash index
        let hash_index_key = Self::build_hash_index_key(&key_hash);

        let index_value = match self
            .db
            .get_cf(cf, &hash_index_key)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?
        {
            Some(v) => v,
            None => return Ok(None),
        };

        // Parse the index value: tenant_id\0user_id\0key_id
        let index_str = String::from_utf8(index_value.to_vec())
            .map_err(|e| raisin_error::Error::Backend(e.to_string()))?;

        let parts: Vec<&str> = index_str.split('\0').collect();
        if parts.len() != 3 {
            return Err(raisin_error::Error::Backend(
                "Invalid API key hash index format".to_string(),
            ));
        }

        let (tenant_id, user_id, key_id) = (parts[0], parts[1], parts[2]);

        // Get the actual API key
        let mut api_key = match self.get_api_key(tenant_id, user_id, key_id)? {
            Some(k) => k,
            None => return Ok(None),
        };

        // Check if active
        if !api_key.is_active {
            return Ok(None);
        }

        // Update last used timestamp
        api_key.record_usage();
        let key = Self::build_key(tenant_id, user_id, key_id);
        let value = rmp_serde::to_vec(&api_key).map_err(|e| {
            raisin_error::Error::Backend(format!("Failed to serialize API key: {}", e))
        })?;
        self.db
            .put_cf(cf, &key, &value)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        Ok(Some(api_key))
    }
}
