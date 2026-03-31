//! Replication apply methods for session operations.
//!
//! These methods are used by the replication applicator to apply
//! session operations received from other cluster nodes.

use super::SessionRepository;
use crate::keys;
use raisin_error::Result;
use raisin_models::auth::Session;
use tracing::info;

impl SessionRepository {
    /// Apply a replicated session creation (for replication applicator).
    pub fn apply_create_session(&self, tenant_id: &str, session: &Session) -> Result<()> {
        let cf = self.cf_sessions()?;

        // Write session
        let key = keys::session_key(tenant_id, &session.session_id);
        let bytes = rmp_serde::to_vec_named(session)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        self.db
            .put_cf(cf, &key, &bytes)
            .map_err(|e| raisin_error::Error::storage(format!("Failed to apply session: {}", e)))?;

        // Write identity sessions index
        let index_key =
            keys::identity_sessions_index_key(tenant_id, &session.identity_id, &session.session_id);
        self.db.put_cf(cf, &index_key, b"").map_err(|e| {
            raisin_error::Error::storage(format!("Failed to apply session index: {}", e))
        })?;

        info!(
            "Applied replicated session {} for tenant {}",
            session.session_id, tenant_id
        );
        Ok(())
    }

    /// Apply a replicated session revocation (for replication applicator).
    pub fn apply_revoke_session(&self, tenant_id: &str, session_id: &str) -> Result<()> {
        let cf = self.cf_sessions()?;
        let key = keys::session_key(tenant_id, session_id);

        // Get and update session
        if let Ok(Some(bytes)) = self.db.get_cf(cf, &key) {
            if let Ok(mut session) = rmp_serde::from_slice::<Session>(&bytes) {
                session.revoke("replicated revocation");
                let updated_bytes = rmp_serde::to_vec_named(&session).map_err(|e| {
                    raisin_error::Error::storage(format!("Serialization error: {}", e))
                })?;
                self.db.put_cf(cf, &key, &updated_bytes).map_err(|e| {
                    raisin_error::Error::storage(format!("Failed to apply revocation: {}", e))
                })?;
            }
        }

        info!(
            "Applied replicated session revocation {} for tenant {}",
            session_id, tenant_id
        );
        Ok(())
    }

    /// Apply a replicated bulk session revocation (for replication applicator).
    pub fn apply_revoke_all_identity_sessions(
        &self,
        tenant_id: &str,
        identity_id: &str,
    ) -> Result<()> {
        let cf = self.cf_sessions()?;
        let prefix = keys::identity_sessions_prefix(tenant_id, identity_id);

        let iter = self.db.prefix_iterator_cf(cf, &prefix);

        for (key, _) in iter.flatten() {
            if !key.starts_with(&prefix) {
                break;
            }

            // Extract session_id and revoke
            let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
            if parts.len() >= 4 {
                if let Ok(session_id) = String::from_utf8(parts[3].to_vec()) {
                    let _ = self.apply_revoke_session(tenant_id, &session_id);
                }
            }
        }

        info!(
            "Applied replicated bulk revocation for identity {} in tenant {}",
            identity_id, tenant_id
        );
        Ok(())
    }
}
