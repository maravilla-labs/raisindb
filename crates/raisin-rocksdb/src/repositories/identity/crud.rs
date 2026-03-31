//! CRUD operations for identities.
//!
//! Provides create/update (upsert), read (get, find_by_email), list,
//! and delete operations for authentication identities.

use super::IdentityRepository;
use crate::keys;
use raisin_error::Result;
use raisin_models::auth::Identity;
use raisin_replication::OpType;
use tracing::{debug, error, info};

impl IdentityRepository {
    /// Create or update an identity.
    ///
    /// This method:
    /// 1. Stores the identity in the IDENTITIES column family
    /// 2. Updates the email index for lookup
    /// 3. Captures the operation to the OpLog for replication
    pub async fn upsert(&self, tenant_id: &str, identity: &Identity, actor: &str) -> Result<()> {
        let cf = self.cf_identities()?;
        let cf_email = self.cf_email_index()?;

        // Check for existing identity to handle email changes
        let old_email = self
            .get(tenant_id, &identity.identity_id)
            .await?
            .map(|i| i.email);

        // Serialize identity using MessagePack
        let key = keys::identity_key(tenant_id, &identity.identity_id);
        let bytes = rmp_serde::to_vec_named(identity)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        // Write identity
        self.db.put_cf(cf, &key, &bytes).map_err(|e| {
            error!("Failed to store identity {}: {}", identity.identity_id, e);
            raisin_error::Error::storage(format!("Failed to write identity: {}", e))
        })?;

        // Update email index (delete old if email changed)
        if let Some(old_email) = old_email {
            if old_email != identity.email {
                let old_email_key =
                    keys::identity_email_index_key(tenant_id, &old_email.to_lowercase());
                self.db.delete_cf(cf_email, &old_email_key).map_err(|e| {
                    error!("Failed to delete old email index: {}", e);
                    raisin_error::Error::storage(format!("Failed to update email index: {}", e))
                })?;
            }
        }

        // Write email index (lowercase for case-insensitive lookup)
        let email_key = keys::identity_email_index_key(tenant_id, &identity.email.to_lowercase());
        self.db
            .put_cf(cf_email, &email_key, identity.identity_id.as_bytes())
            .map_err(|e| {
                error!("Failed to update email index: {}", e);
                raisin_error::Error::storage(format!("Failed to write email index: {}", e))
            })?;

        // Capture operation for replication
        self.operation_capture
            .capture_operation(
                tenant_id.to_string(),
                "_identity".to_string(),
                "main".to_string(),
                OpType::UpsertIdentity {
                    identity_id: identity.identity_id.clone(),
                    identity: identity.clone(),
                },
                actor.to_string(),
                None,
                true,
            )
            .await?;

        info!(
            "Upserted identity {} for tenant {}",
            identity.identity_id, tenant_id
        );
        Ok(())
    }

    /// Get an identity by ID.
    pub async fn get(&self, tenant_id: &str, identity_id: &str) -> Result<Option<Identity>> {
        let cf = self.cf_identities()?;
        let key = keys::identity_key(tenant_id, identity_id);

        match self.db.get_cf(cf, &key) {
            Ok(Some(bytes)) => {
                let identity: Identity = rmp_serde::from_slice(&bytes).map_err(|e| {
                    raisin_error::Error::storage(format!("Deserialization error: {}", e))
                })?;
                debug!("Retrieved identity {}", identity_id);
                Ok(Some(identity))
            }
            Ok(None) => {
                debug!("Identity {} not found", identity_id);
                Ok(None)
            }
            Err(e) => {
                error!("Failed to get identity {}: {}", identity_id, e);
                Err(raisin_error::Error::storage(format!(
                    "Failed to read identity: {}",
                    e
                )))
            }
        }
    }

    /// Find an identity by email address.
    ///
    /// Uses the email index for efficient O(1) lookup.
    pub async fn find_by_email(&self, tenant_id: &str, email: &str) -> Result<Option<Identity>> {
        let cf_email = self.cf_email_index()?;
        let email_key = keys::identity_email_index_key(tenant_id, &email.to_lowercase());

        match self.db.get_cf(cf_email, &email_key) {
            Ok(Some(identity_id_bytes)) => {
                let identity_id = String::from_utf8(identity_id_bytes.to_vec()).map_err(|e| {
                    raisin_error::Error::storage(format!("Invalid UTF-8 in email index: {}", e))
                })?;
                self.get(tenant_id, &identity_id).await
            }
            Ok(None) => {
                debug!("No identity found for email {}", email);
                Ok(None)
            }
            Err(e) => {
                error!("Failed to query email index: {}", e);
                Err(raisin_error::Error::storage(format!(
                    "Failed to query email index: {}",
                    e
                )))
            }
        }
    }

    /// Delete an identity.
    ///
    /// Also removes the email index entry and captures the operation for replication.
    pub async fn delete(&self, tenant_id: &str, identity_id: &str, actor: &str) -> Result<()> {
        let cf = self.cf_identities()?;
        let cf_email = self.cf_email_index()?;

        // Get the identity to find its email for index cleanup
        if let Some(identity) = self.get(tenant_id, identity_id).await? {
            let email_key =
                keys::identity_email_index_key(tenant_id, &identity.email.to_lowercase());
            self.db.delete_cf(cf_email, &email_key).map_err(|e| {
                error!("Failed to delete email index: {}", e);
                raisin_error::Error::storage(format!("Failed to delete email index: {}", e))
            })?;
        }

        // Delete identity
        let key = keys::identity_key(tenant_id, identity_id);
        self.db.delete_cf(cf, &key).map_err(|e| {
            error!("Failed to delete identity {}: {}", identity_id, e);
            raisin_error::Error::storage(format!("Failed to delete identity: {}", e))
        })?;

        // Capture operation for replication
        self.operation_capture
            .capture_operation(
                tenant_id.to_string(),
                "_identity".to_string(),
                "main".to_string(),
                OpType::DeleteIdentity {
                    identity_id: identity_id.to_string(),
                },
                actor.to_string(),
                None,
                true,
            )
            .await?;

        info!("Deleted identity {} from tenant {}", identity_id, tenant_id);
        Ok(())
    }

    /// List all identities in a tenant.
    pub async fn list(
        &self,
        tenant_id: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Identity>> {
        let cf = self.cf_identities()?;
        let prefix = keys::identity_prefix(tenant_id);

        let iter = self.db.prefix_iterator_cf(cf, &prefix);

        let mut identities = Vec::new();
        let mut count = 0;

        for item in iter {
            match item {
                Ok((key, value)) => {
                    if !key.starts_with(&prefix) {
                        break;
                    }

                    if count < offset {
                        count += 1;
                        continue;
                    }

                    if identities.len() >= limit {
                        break;
                    }

                    let identity: Identity = rmp_serde::from_slice(&value).map_err(|e| {
                        raisin_error::Error::storage(format!("Deserialization error: {}", e))
                    })?;
                    identities.push(identity);
                    count += 1;
                }
                Err(e) => {
                    error!("Failed to iterate identities: {}", e);
                    return Err(raisin_error::Error::storage(format!(
                        "Failed to iterate identities: {}",
                        e
                    )));
                }
            }
        }

        Ok(identities)
    }
}
