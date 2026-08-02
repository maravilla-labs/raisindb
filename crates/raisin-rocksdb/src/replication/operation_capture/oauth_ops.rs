//! Capture helpers for OAuth authorization-server and API-key state.
//!
//! All of these use the `system`/`main` pseudo repo+branch, like the admin-user
//! and tenant captures: the records are deployment-wide auth state, not content
//! in any user repository.

use super::OperationCapture;
use raisin_error::Result;
use raisin_models::api_key::ApiKey;
use raisin_models::auth::{OAuthClient, RefreshToken};
use raisin_replication::{OpType, Operation};

/// Repo/branch used for deployment-wide auth records.
const SYSTEM_REPO: &str = "system";
const SYSTEM_BRANCH: &str = "main";

impl OperationCapture {
    /// Capture a registered OAuth client so every node can serve `/authorize`
    /// for it.
    pub async fn capture_upsert_oauth_client(
        &self,
        tenant_id: String,
        client: OAuthClient,
        actor: String,
    ) -> Result<Operation> {
        self.capture_operation(
            tenant_id,
            SYSTEM_REPO.to_string(),
            SYSTEM_BRANCH.to_string(),
            OpType::UpsertOAuthClient {
                client_id: client.client_id.clone(),
                client,
            },
            actor,
            None,
            false,
        )
        .await
    }

    /// Capture the deletion of a registered OAuth client.
    pub async fn capture_delete_oauth_client(
        &self,
        tenant_id: String,
        client_id: String,
        actor: String,
    ) -> Result<Operation> {
        self.capture_operation(
            tenant_id,
            SYSTEM_REPO.to_string(),
            SYSTEM_BRANCH.to_string(),
            OpType::DeleteOAuthClient { client_id },
            actor,
            None,
            false,
        )
        .await
    }

    /// Capture a refresh token (issue or rotation).
    pub async fn capture_upsert_oauth_refresh_token(
        &self,
        tenant_id: String,
        token: RefreshToken,
        actor: String,
    ) -> Result<Operation> {
        self.capture_operation(
            tenant_id,
            SYSTEM_REPO.to_string(),
            SYSTEM_BRANCH.to_string(),
            OpType::UpsertOAuthRefreshToken {
                token_hash: token.token_hash.clone(),
                token,
            },
            actor,
            None,
            false,
        )
        .await
    }

    /// Capture a refresh-family revocation (replay detected).
    pub async fn capture_revoke_oauth_refresh_family(
        &self,
        tenant_id: String,
        family_id: String,
        actor: String,
    ) -> Result<Operation> {
        self.capture_operation(
            tenant_id,
            SYSTEM_REPO.to_string(),
            SYSTEM_BRANCH.to_string(),
            OpType::RevokeOAuthRefreshFamily { family_id },
            actor,
            None,
            false,
        )
        .await
    }

    /// Capture an API key on creation or revocation.
    ///
    /// **Never call this from the validation path.** `validate_api_key` rewrites
    /// the record on every successful call to stamp `last_used_at`; capturing
    /// there would emit one replicated operation per authenticated connection,
    /// and `last_used_at` would become a last-writer-wins register flapping
    /// between nodes for no benefit.
    pub async fn capture_upsert_api_key(
        &self,
        tenant_id: String,
        api_key: ApiKey,
        actor: String,
    ) -> Result<Operation> {
        self.capture_operation(
            tenant_id,
            SYSTEM_REPO.to_string(),
            SYSTEM_BRANCH.to_string(),
            OpType::UpsertApiKey {
                key_id: api_key.key_id.clone(),
                api_key,
            },
            actor,
            None,
            false,
        )
        .await
    }
}
