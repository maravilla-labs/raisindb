// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Durable [`ClientStore`]: registered OAuth clients survive a restart.

use raisin_auth::authserver::{AuthServerResult, ClientStore, OAuthClient};

use super::RocksDbOAuthStore;

/// Key discriminator for client records.
const CLIENT_KIND: &str = "oauth_client";

impl ClientStore for RocksDbOAuthStore {
    async fn put_client(&self, client: OAuthClient) -> AuthServerResult<()> {
        let key = Self::build_key(&client.tenant_id, CLIENT_KIND, &client.client_id);
        self.put_record(&key, &client)
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
