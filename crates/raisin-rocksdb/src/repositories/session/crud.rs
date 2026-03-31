//! CRUD operations for sessions.
//!
//! Provides create, read, update, revoke, delete, and list operations
//! for authentication sessions.

use super::SessionRepository;
use crate::keys;
use raisin_error::Result;
use raisin_models::auth::Session;
use raisin_replication::OpType;
use tracing::{debug, error, info};

impl SessionRepository {
    /// Create a new session.
    ///
    /// This method:
    /// 1. Stores the session in the SESSIONS column family
    /// 2. Creates an index entry for identity -> sessions lookup
    /// 3. Captures the operation to the OpLog for replication
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `session` - The session to create
    /// * `actor` - Actor performing the operation (for audit)
    pub async fn create(&self, tenant_id: &str, session: &Session, actor: &str) -> Result<()> {
        let cf = self.cf_sessions()?;

        // Serialize session using MessagePack
        let key = keys::session_key(tenant_id, &session.session_id);
        let bytes = rmp_serde::to_vec_named(session)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        // Write session
        self.db.put_cf(cf, &key, &bytes).map_err(|e| {
            error!("Failed to store session {}: {}", session.session_id, e);
            raisin_error::Error::storage(format!("Failed to write session: {}", e))
        })?;

        // Write identity sessions index (for listing all sessions for an identity)
        let index_key =
            keys::identity_sessions_index_key(tenant_id, &session.identity_id, &session.session_id);
        self.db.put_cf(cf, &index_key, b"").map_err(|e| {
            error!("Failed to write identity sessions index: {}", e);
            raisin_error::Error::storage(format!("Failed to write index: {}", e))
        })?;

        // Capture operation for replication
        self.operation_capture
            .capture_operation(
                tenant_id.to_string(),
                "_session".to_string(), // Special repo for session ops
                "main".to_string(),
                OpType::CreateSession {
                    session_id: session.session_id.clone(),
                    session: session.clone(),
                },
                actor.to_string(),
                None,
                true, // System operation
            )
            .await?;

        info!(
            "Created session {} for identity {} in tenant {}",
            session.session_id, session.identity_id, tenant_id
        );
        Ok(())
    }

    /// Update an existing session.
    ///
    /// Used for updating session state (e.g., refresh token rotation, activity updates).
    pub async fn update(&self, tenant_id: &str, session: &Session) -> Result<()> {
        let cf = self.cf_sessions()?;

        let key = keys::session_key(tenant_id, &session.session_id);
        let bytes = rmp_serde::to_vec_named(session)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        self.db.put_cf(cf, &key, &bytes).map_err(|e| {
            error!("Failed to update session {}: {}", session.session_id, e);
            raisin_error::Error::storage(format!("Failed to update session: {}", e))
        })?;

        debug!("Updated session {}", session.session_id);
        Ok(())
    }

    /// Get a session by ID.
    pub async fn get(&self, tenant_id: &str, session_id: &str) -> Result<Option<Session>> {
        let cf = self.cf_sessions()?;
        let key = keys::session_key(tenant_id, session_id);

        match self.db.get_cf(cf, &key) {
            Ok(Some(bytes)) => {
                let session: Session = rmp_serde::from_slice(&bytes).map_err(|e| {
                    raisin_error::Error::storage(format!("Deserialization error: {}", e))
                })?;
                debug!("Retrieved session {}", session_id);
                Ok(Some(session))
            }
            Ok(None) => {
                debug!("Session {} not found", session_id);
                Ok(None)
            }
            Err(e) => {
                error!("Failed to get session {}: {}", session_id, e);
                Err(raisin_error::Error::storage(format!(
                    "Failed to read session: {}",
                    e
                )))
            }
        }
    }

    /// Revoke a session.
    ///
    /// Marks the session as revoked and captures the operation for replication.
    pub async fn revoke(
        &self,
        tenant_id: &str,
        session_id: &str,
        reason: &str,
        actor: &str,
    ) -> Result<()> {
        // Get existing session
        let mut session = match self.get(tenant_id, session_id).await? {
            Some(s) => s,
            None => {
                debug!("Session {} not found for revocation", session_id);
                return Ok(());
            }
        };

        // Mark as revoked
        session.revoke(reason);

        // Update in storage
        let cf = self.cf_sessions()?;
        let key = keys::session_key(tenant_id, session_id);
        let bytes = rmp_serde::to_vec_named(&session)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        self.db.put_cf(cf, &key, &bytes).map_err(|e| {
            error!("Failed to revoke session {}: {}", session_id, e);
            raisin_error::Error::storage(format!("Failed to revoke session: {}", e))
        })?;

        // Capture operation for replication
        self.operation_capture
            .capture_operation(
                tenant_id.to_string(),
                "_session".to_string(),
                "main".to_string(),
                OpType::RevokeSession {
                    session_id: session_id.to_string(),
                },
                actor.to_string(),
                None,
                true,
            )
            .await?;

        info!("Revoked session {} in tenant {}", session_id, tenant_id);
        Ok(())
    }

    /// Revoke all sessions for an identity.
    ///
    /// Used when password is changed or identity is deactivated.
    pub async fn revoke_all_for_identity(
        &self,
        tenant_id: &str,
        identity_id: &str,
        reason: &str,
        actor: &str,
    ) -> Result<u32> {
        let sessions = self.list_for_identity(tenant_id, identity_id).await?;
        let count = sessions.len() as u32;

        for session in sessions {
            if !session.revoked {
                // Revoke without capturing individual operations
                let mut updated = session.clone();
                updated.revoke(reason);

                let cf = self.cf_sessions()?;
                let key = keys::session_key(tenant_id, &session.session_id);
                let bytes = rmp_serde::to_vec_named(&updated).map_err(|e| {
                    raisin_error::Error::storage(format!("Serialization error: {}", e))
                })?;

                self.db.put_cf(cf, &key, &bytes).map_err(|e| {
                    raisin_error::Error::storage(format!("Failed to revoke session: {}", e))
                })?;
            }
        }

        // Capture bulk operation for replication
        self.operation_capture
            .capture_operation(
                tenant_id.to_string(),
                "_session".to_string(),
                "main".to_string(),
                OpType::RevokeAllIdentitySessions {
                    identity_id: identity_id.to_string(),
                },
                actor.to_string(),
                None,
                true,
            )
            .await?;

        info!(
            "Revoked {} sessions for identity {} in tenant {}",
            count, identity_id, tenant_id
        );
        Ok(count)
    }

    /// List all sessions for an identity.
    pub async fn list_for_identity(
        &self,
        tenant_id: &str,
        identity_id: &str,
    ) -> Result<Vec<Session>> {
        let cf = self.cf_sessions()?;
        let prefix = keys::identity_sessions_prefix(tenant_id, identity_id);

        let iter = self.db.prefix_iterator_cf(cf, &prefix);

        let mut sessions = Vec::new();

        for item in iter {
            match item {
                Ok((key, _)) => {
                    // Verify key is within our prefix
                    if !key.starts_with(&prefix) {
                        break;
                    }

                    // Extract session_id from key
                    // Key format: {tenant}\0identity_sessions\0{identity_id}\0{session_id}
                    let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
                    if parts.len() >= 4 {
                        let session_id = String::from_utf8(parts[3].to_vec()).map_err(|e| {
                            raisin_error::Error::storage(format!("Invalid UTF-8 in key: {}", e))
                        })?;

                        if let Some(session) = self.get(tenant_id, &session_id).await? {
                            sessions.push(session);
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to iterate sessions: {}", e);
                    return Err(raisin_error::Error::storage(format!(
                        "Failed to iterate sessions: {}",
                        e
                    )));
                }
            }
        }

        Ok(sessions)
    }

    /// Delete a session completely (used for cleanup).
    pub async fn delete(&self, tenant_id: &str, session_id: &str) -> Result<()> {
        let cf = self.cf_sessions()?;

        // Get session to find identity for index cleanup
        if let Some(session) = self.get(tenant_id, session_id).await? {
            // Delete identity sessions index
            let index_key =
                keys::identity_sessions_index_key(tenant_id, &session.identity_id, session_id);
            self.db.delete_cf(cf, &index_key).map_err(|e| {
                error!("Failed to delete identity sessions index: {}", e);
                raisin_error::Error::storage(format!("Failed to delete index: {}", e))
            })?;
        }

        // Delete session
        let key = keys::session_key(tenant_id, session_id);
        self.db.delete_cf(cf, &key).map_err(|e| {
            error!("Failed to delete session {}: {}", session_id, e);
            raisin_error::Error::storage(format!("Failed to delete session: {}", e))
        })?;

        debug!("Deleted session {}", session_id);
        Ok(())
    }

    /// Rotate refresh token - increments the generation counter.
    ///
    /// This method:
    /// 1. Updates the session's token_generation
    /// 2. Updates last_activity_at
    /// 3. Captures the operation to the OpLog for replication
    pub async fn rotate_refresh_token(
        &self,
        tenant_id: &str,
        session_id: &str,
        new_generation: u32,
        actor: &str,
    ) -> Result<()> {
        let cf = self.cf_sessions()?;
        let key = keys::session_key(tenant_id, session_id);

        // Get and update session
        if let Some(bytes) = self
            .db
            .get_cf(cf, &key)
            .map_err(|e| raisin_error::Error::storage(format!("Failed to read session: {}", e)))?
        {
            let mut session: Session = rmp_serde::from_slice(&bytes).map_err(|e| {
                raisin_error::Error::storage(format!("Deserialization error: {}", e))
            })?;

            session.token_generation = new_generation;
            session.touch();

            let updated = rmp_serde::to_vec_named(&session)
                .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

            self.db.put_cf(cf, &key, &updated).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to update session: {}", e))
            })?;
        }

        // Capture operation for replication
        self.operation_capture
            .capture_operation(
                tenant_id.to_string(),
                "_session".to_string(),
                "main".to_string(),
                OpType::RotateRefreshToken {
                    session_id: session_id.to_string(),
                    new_generation,
                },
                actor.to_string(),
                None,
                true,
            )
            .await?;

        debug!(
            "Rotated refresh token for session {} to generation {}",
            session_id, new_generation
        );
        Ok(())
    }
}
