// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Durable [`AuthCodeStore`]: single-use authorization codes.
//!
//! Codes are short-lived, so persisting them is not about surviving a restart —
//! it is about a load-balanced deployment where the `/token` call can land on a
//! different replica than the `/authorize` that issued the code.

use raisin_auth::authserver::{AuthCodeStore, AuthServerResult, AuthorizationCode};

use super::RocksDbOAuthStore;

/// Key discriminator for authorization codes.
const CODE_KIND: &str = "oauth_code";

impl AuthCodeStore for RocksDbOAuthStore {
    async fn put_code(&self, code: AuthorizationCode) -> AuthServerResult<()> {
        let key = Self::build_key(&code.tenant_id, CODE_KIND, &code.code);
        self.put_record(&key, &code)
    }

    async fn take_code(
        &self,
        tenant_id: &str,
        code: &str,
    ) -> AuthServerResult<Option<AuthorizationCode>> {
        let key = Self::build_key(tenant_id, CODE_KIND, code);

        // Hold the per-key lock across read + delete so two concurrent
        // redemptions of the same code cannot both observe it as present.
        let _guard = self.locks().lock(key.clone()).await;

        let Some(record) = self.get_record::<AuthorizationCode>(&key)? else {
            return Ok(None);
        };
        // Consume it whether or not it turns out to be usable: an expired code
        // is dead either way, and leaving it behind only grows the keyspace.
        self.delete_record(&key)?;

        if record.is_expired(chrono::Utc::now().timestamp()) {
            return Ok(None);
        }
        Ok(Some(record))
    }
}
