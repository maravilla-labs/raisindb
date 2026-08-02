// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Durable [`ClientStore`]: registered OAuth clients survive a restart.

use raisin_auth::authserver::{AuthServerResult, ClientStore, OAuthClient};

use super::RocksDbOAuthStore;

/// Key discriminator for client records.
const CLIENT_KIND: &str = "oauth_client";

impl RocksDbOAuthStore {
    /// Remove a registered client.
    ///
    /// No live path calls this yet — client revocation has no UI — but the
    /// replication operation exists, so the apply side needs somewhere to land.
    pub fn delete_client(&self, tenant_id: &str, client_id: &str) -> AuthServerResult<()> {
        self.delete_record(&Self::build_key(tenant_id, CLIENT_KIND, client_id))
    }
}

impl ClientStore for RocksDbOAuthStore {
    async fn put_client(&self, client: OAuthClient) -> AuthServerResult<()> {
        let key = Self::build_key(&client.tenant_id, CLIENT_KIND, &client.client_id);
        self.put_record(&key, &client)?;

        // Replicate. A registration that reaches only the node that served it
        // is the multi-node form of the bug this whole store exists to fix:
        // the MCP host caches the client_id, then any other node answers
        // `/authorize` with `invalid_client`.
        if let Some(capture) = self.capture() {
            if let Err(e) = capture
                .capture_upsert_oauth_client(
                    client.tenant_id.clone(),
                    client.clone(),
                    "system".to_string(),
                )
                .await
            {
                // The local write succeeded; failing the registration now would
                // be worse than converging late.
                tracing::error!(
                    error = %e,
                    client_id = %client.client_id,
                    "failed to capture OAuth client for replication; it exists on this node only"
                );
            }
        }
        Ok(())
    }

    async fn get_client(
        &self,
        tenant_id: &str,
        client_id: &str,
    ) -> AuthServerResult<Option<OAuthClient>> {
        let key = Self::build_key(tenant_id, CLIENT_KIND, client_id);
        self.get_record(&key)
    }
}
