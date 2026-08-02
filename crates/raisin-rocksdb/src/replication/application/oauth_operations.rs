//! OAuth authorization-server and API-key operation handlers for replication.
//!
//! These make three kinds of auth state cluster-wide:
//!
//! - **OAuth clients** — the reason this module exists. An MCP host registers
//!   once via RFC 7591 and caches the issued `client_id` forever, so a node that
//!   never saw the registration answers `/authorize` with `invalid_client` and
//!   the user can only recover by deleting and re-adding the connector.
//! - **Refresh tokens** — so a rotation performed on one node is visible on the
//!   others. Note that *replay detection* does not depend on this converging:
//!   the authorization server marks consumption with a lock lease, which is
//!   immediate everywhere.
//! - **API keys** — create and revoke only. Validation stamps `last_used_at` on
//!   every call and is deliberately NOT captured; that field stays node-local.
//!
//! # Every write here goes through the local store
//!
//! Nothing in this module composes a storage key or serializes a record itself.
//! It builds the same `RocksDbOAuthStore` / `ApiKeyStore` the live write path
//! uses and calls their methods, so there is exactly ONE key layout and ONE
//! encoding per record type.
//!
//! That is deliberate. A replica that composed `oauth_client` keys one byte
//! differently, or used positional MessagePack where the writer used named,
//! would produce a node that silently fails to resolve clients it *does* hold —
//! a divergence no single-node test can see. The stores are constructed
//! **without** capture, so applying a replicated operation cannot re-emit it.

use super::super::OperationApplicator;
use crate::api_key_store::ApiKeyStore;
use crate::oauth_store::RocksDbOAuthStore;
use raisin_auth::authserver::{AuthServerError, ClientStore, RefreshTokenStore};
use raisin_error::Result;
use raisin_models::api_key::ApiKey;
use raisin_models::auth::{OAuthClient, RefreshToken};
use raisin_replication::Operation;

/// The OAuth store over this node's database, with capture disabled.
fn oauth_store(applicator: &OperationApplicator) -> RocksDbOAuthStore {
    RocksDbOAuthStore::new(applicator.db().clone())
}

/// Translate a store error into a replication error.
fn store_err(e: AuthServerError) -> raisin_error::Error {
    raisin_error::Error::storage(e.to_string())
}

/// Apply an OAuth client upsert.
pub(super) async fn apply_upsert_oauth_client(
    applicator: &OperationApplicator,
    tenant_id: &str,
    client_id: &str,
    client: &OAuthClient,
    op: &Operation,
) -> Result<()> {
    tracing::info!(
        "📥 Applying OAuth client upsert: {}/{} from node {}",
        tenant_id,
        client_id,
        op.cluster_node_id
    );
    oauth_store(applicator)
        .put_client(client.clone())
        .await
        .map_err(store_err)
}

/// Apply an OAuth client deletion.
pub(super) async fn apply_delete_oauth_client(
    applicator: &OperationApplicator,
    tenant_id: &str,
    client_id: &str,
    _op: &Operation,
) -> Result<()> {
    oauth_store(applicator)
        .delete_client(tenant_id, client_id)
        .map_err(store_err)
}

/// Apply a refresh-token upsert.
///
/// The store refuses a token belonging to a revoked family, which is what stops
/// a rotation captured moments before a replay was detected from resurrecting
/// that family when it lands here afterwards. A refusal is a normal outcome, not
/// an apply failure — returning `Err` would make the operation redeliver
/// forever.
pub(super) async fn apply_upsert_oauth_refresh_token(
    applicator: &OperationApplicator,
    tenant_id: &str,
    token_hash: &str,
    token: &RefreshToken,
    _op: &Operation,
) -> Result<()> {
    match oauth_store(applicator)
        .put_refresh_token(token.clone())
        .await
    {
        Ok(()) => Ok(()),
        Err(AuthServerError::InvalidGrant(_)) => {
            tracing::warn!(
                tenant_id,
                token_hash,
                family_id = %token.family_id,
                "ignoring replicated refresh token for a revoked rotation family"
            );
            Ok(())
        }
        Err(e) => Err(store_err(e)),
    }
}

/// Apply a refresh-family revocation: tombstone the family and drop its members.
pub(super) async fn apply_revoke_oauth_refresh_family(
    applicator: &OperationApplicator,
    tenant_id: &str,
    family_id: &str,
    _op: &Operation,
) -> Result<()> {
    let revoked = oauth_store(applicator)
        .revoke_refresh_family(tenant_id, family_id)
        .await
        .map_err(store_err)?;
    tracing::info!(
        tenant_id,
        family_id,
        revoked,
        "📥 Applied refresh-token family revocation"
    );
    Ok(())
}

/// Apply an API-key upsert — create or revoke.
///
/// Revocation arrives here too: the local store revokes by clearing
/// `is_active` and rewriting the record, so a revoked key is simply an upsert
/// whose flag is off.
pub(super) async fn apply_upsert_api_key(
    applicator: &OperationApplicator,
    tenant_id: &str,
    key_id: &str,
    api_key: &ApiKey,
    _op: &Operation,
) -> Result<()> {
    tracing::info!("📥 Applying API key upsert: {}/{}", tenant_id, key_id);
    ApiKeyStore::new(applicator.db().clone()).put_replicated(api_key)
}
