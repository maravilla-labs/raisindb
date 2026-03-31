//! Replication apply methods for identity operations.
//!
//! These methods are used by the replication applicator to apply
//! identity operations received from other cluster nodes.

use super::IdentityRepository;
use crate::keys;
use raisin_error::Result;
use raisin_models::auth::Identity;
use tracing::info;

impl IdentityRepository {
    /// Apply a replicated identity upsert (for replication applicator).
    ///
    /// Writes directly to RocksDB without capturing to OpLog (to prevent loops).
    pub fn apply_upsert_identity(&self, tenant_id: &str, identity: &Identity) -> Result<()> {
        let cf = self.cf_identities()?;
        let cf_email = self.cf_email_index()?;

        // Serialize identity
        let key = keys::identity_key(tenant_id, &identity.identity_id);
        let bytes = rmp_serde::to_vec_named(identity)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        // Write identity
        self.db.put_cf(cf, &key, &bytes).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to apply identity: {}", e))
        })?;

        // Write email index
        let email_key = keys::identity_email_index_key(tenant_id, &identity.email.to_lowercase());
        self.db
            .put_cf(cf_email, &email_key, identity.identity_id.as_bytes())
            .map_err(|e| {
                raisin_error::Error::storage(format!("Failed to apply email index: {}", e))
            })?;

        info!(
            "Applied replicated identity {} for tenant {}",
            identity.identity_id, tenant_id
        );
        Ok(())
    }

    /// Apply a replicated identity deletion (for replication applicator).
    pub fn apply_delete_identity(&self, tenant_id: &str, identity_id: &str) -> Result<()> {
        let cf = self.cf_identities()?;
        let cf_email = self.cf_email_index()?;

        // Get identity for email index cleanup (sync read for replication)
        let key = keys::identity_key(tenant_id, identity_id);
        if let Ok(Some(bytes)) = self.db.get_cf(cf, &key) {
            if let Ok(identity) = rmp_serde::from_slice::<Identity>(&bytes) {
                let email_key =
                    keys::identity_email_index_key(tenant_id, &identity.email.to_lowercase());
                let _ = self.db.delete_cf(cf_email, &email_key);
            }
        }

        // Delete identity
        self.db.delete_cf(cf, &key).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to apply identity delete: {}", e))
        })?;

        info!(
            "Applied replicated identity deletion {} for tenant {}",
            identity_id, tenant_id
        );
        Ok(())
    }
}
