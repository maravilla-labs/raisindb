//! Identity and session operation handlers for replication
//!
//! This module contains operation handlers for:
//! - apply_upsert_identity
//! - apply_delete_identity
//! - apply_create_session
//! - apply_revoke_session
//! - apply_revoke_all_identity_sessions

use super::super::OperationApplicator;
use super::db_helpers::delete_key;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_models::auth::{Identity, Session};
use raisin_replication::Operation;

/// Apply an identity upsert operation
pub(super) async fn apply_upsert_identity(
    applicator: &OperationApplicator,
    tenant_id: &str,
    identity_id: &str,
    identity: &Identity,
    op: &Operation,
) -> Result<()> {
    tracing::info!(
        "📥 Applying identity upsert: {}/{} from node {}",
        tenant_id,
        identity_id,
        op.cluster_node_id
    );

    // Write to IDENTITIES column family
    let key = keys::identity_key(tenant_id, identity_id);
    let cf = cf_handle(&applicator.db, cf::IDENTITIES)?;

    let value = rmp_serde::to_vec_named(identity)
        .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

    applicator
        .db
        .put_cf(cf, &key, &value)
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

    // Update email index
    let cf_email = cf_handle(&applicator.db, cf::IDENTITY_EMAIL_INDEX)?;
    let email_key = keys::identity_email_index_key(tenant_id, &identity.email.to_lowercase());

    applicator
        .db
        .put_cf(cf_email, &email_key, identity_id.as_bytes())
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

    tracing::info!(
        "✅ Identity upserted successfully: {}/{}",
        tenant_id,
        identity_id
    );
    Ok(())
}

/// Apply an identity delete operation
pub(super) async fn apply_delete_identity(
    applicator: &OperationApplicator,
    tenant_id: &str,
    identity_id: &str,
    op: &Operation,
) -> Result<()> {
    tracing::info!(
        "📥 Applying identity delete: {}/{} from node {}",
        tenant_id,
        identity_id,
        op.cluster_node_id
    );

    let cf = cf_handle(&applicator.db, cf::IDENTITIES)?;
    let cf_email = cf_handle(&applicator.db, cf::IDENTITY_EMAIL_INDEX)?;

    // Get identity to find email for index cleanup
    let key = keys::identity_key(tenant_id, identity_id);
    if let Ok(Some(bytes)) = applicator.db.get_cf(cf, &key) {
        if let Ok(identity) = rmp_serde::from_slice::<Identity>(&bytes) {
            // Delete email index
            let email_key =
                keys::identity_email_index_key(tenant_id, &identity.email.to_lowercase());
            let _ = applicator.db.delete_cf(cf_email, &email_key);
        }
    }

    // Delete identity
    delete_key(
        &applicator.db,
        cf,
        key,
        &format!("apply_delete_identity_{}/{}", tenant_id, identity_id),
    )?;

    tracing::info!(
        "✅ Identity deleted successfully: {}/{}",
        tenant_id,
        identity_id
    );
    Ok(())
}

/// Apply a session create operation
pub(super) async fn apply_create_session(
    applicator: &OperationApplicator,
    tenant_id: &str,
    session_id: &str,
    session: &Session,
    op: &Operation,
) -> Result<()> {
    tracing::info!(
        "📥 Applying session create: {}/{} from node {}",
        tenant_id,
        session_id,
        op.cluster_node_id
    );

    let cf = cf_handle(&applicator.db, cf::SESSIONS)?;

    // Write session
    let key = keys::session_key(tenant_id, session_id);
    let value = rmp_serde::to_vec_named(session)
        .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

    applicator
        .db
        .put_cf(cf, &key, &value)
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

    // Write identity sessions index
    let index_key = keys::identity_sessions_index_key(tenant_id, &session.identity_id, session_id);
    applicator
        .db
        .put_cf(cf, &index_key, b"")
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

    tracing::info!(
        "✅ Session created successfully: {}/{}",
        tenant_id,
        session_id
    );
    Ok(())
}

/// Apply a session revoke operation
pub(super) async fn apply_revoke_session(
    applicator: &OperationApplicator,
    tenant_id: &str,
    session_id: &str,
    op: &Operation,
) -> Result<()> {
    tracing::info!(
        "📥 Applying session revoke: {}/{} from node {}",
        tenant_id,
        session_id,
        op.cluster_node_id
    );

    let cf = cf_handle(&applicator.db, cf::SESSIONS)?;
    let key = keys::session_key(tenant_id, session_id);

    // Get and update session
    if let Ok(Some(bytes)) = applicator.db.get_cf(cf, &key) {
        if let Ok(mut session) = rmp_serde::from_slice::<Session>(&bytes) {
            session.revoke("replicated revocation");

            let updated_bytes = rmp_serde::to_vec_named(&session)
                .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

            applicator
                .db
                .put_cf(cf, &key, &updated_bytes)
                .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        }
    }

    tracing::info!(
        "✅ Session revoked successfully: {}/{}",
        tenant_id,
        session_id
    );
    Ok(())
}

/// Apply a revoke all identity sessions operation
pub(super) async fn apply_revoke_all_identity_sessions(
    applicator: &OperationApplicator,
    tenant_id: &str,
    identity_id: &str,
    op: &Operation,
) -> Result<()> {
    tracing::info!(
        "📥 Applying revoke all sessions for identity: {}/{} from node {}",
        tenant_id,
        identity_id,
        op.cluster_node_id
    );

    let cf = cf_handle(&applicator.db, cf::SESSIONS)?;
    let prefix = keys::identity_sessions_prefix(tenant_id, identity_id);

    let iter = applicator.db.prefix_iterator_cf(cf, &prefix);

    for (key, _) in iter.flatten() {
        if !key.starts_with(&prefix) {
            break;
        }

        // Extract session_id from key and revoke
        let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
        if parts.len() >= 4 {
            if let Ok(session_id) = String::from_utf8(parts[3].to_vec()) {
                // Inline revocation to avoid async recursion
                let session_key = keys::session_key(tenant_id, &session_id);
                if let Ok(Some(bytes)) = applicator.db.get_cf(cf, &session_key) {
                    if let Ok(mut session) = rmp_serde::from_slice::<Session>(&bytes) {
                        session.revoke("bulk revocation - replicated");
                        if let Ok(updated_bytes) = rmp_serde::to_vec_named(&session) {
                            let _ = applicator.db.put_cf(cf, &session_key, &updated_bytes);
                        }
                    }
                }
            }
        }
    }

    tracing::info!(
        "✅ All sessions revoked for identity: {}/{}",
        tenant_id,
        identity_id
    );
    Ok(())
}

/// Apply a rotate refresh token operation
pub(super) async fn apply_rotate_refresh_token(
    applicator: &OperationApplicator,
    tenant_id: &str,
    session_id: &str,
    new_generation: u32,
    op: &Operation,
) -> Result<()> {
    tracing::info!(
        "📥 Applying rotate refresh token: {}/{} gen={} from node {}",
        tenant_id,
        session_id,
        new_generation,
        op.cluster_node_id
    );

    let cf = cf_handle(&applicator.db, cf::SESSIONS)?;
    let key = keys::session_key(tenant_id, session_id);

    // Get and update session
    if let Ok(Some(bytes)) = applicator.db.get_cf(cf, &key) {
        if let Ok(mut session) = rmp_serde::from_slice::<Session>(&bytes) {
            // Update token generation and last activity
            session.token_generation = new_generation;
            session.touch();

            let updated_bytes = rmp_serde::to_vec_named(&session)
                .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

            applicator
                .db
                .put_cf(cf, &key, &updated_bytes)
                .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        }
    }

    tracing::info!(
        "✅ Refresh token rotated for session: {}/{} to gen={}",
        tenant_id,
        session_id,
        new_generation
    );
    Ok(())
}
