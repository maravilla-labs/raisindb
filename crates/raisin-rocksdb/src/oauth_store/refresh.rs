// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Durable [`RefreshTokenStore`], including the rotation-family index that
//! makes replay revocation a prefix scan rather than a full table scan.

use raisin_auth::authserver::{AuthServerResult, RefreshToken, RefreshTokenStore};

use super::RocksDbOAuthStore;

/// Key discriminator for refresh-token records.
const REFRESH_KIND: &str = "oauth_refresh";
/// Key discriminator for the family index.
pub(super) const FAMILY_KIND: &str = "oauth_refresh_fam";

impl RocksDbOAuthStore {
    /// The family-index key for one member of a rotation chain.
    fn family_member_key(tenant_id: &str, family_id: &str, token_hash: &str) -> Vec<u8> {
        let mut key = Self::build_family_prefix(tenant_id, family_id);
        key.extend_from_slice(token_hash.as_bytes());
        key
    }
}

impl RefreshTokenStore for RocksDbOAuthStore {
    async fn put_refresh_token(&self, token: RefreshToken) -> AuthServerResult<()> {
        let key = Self::build_key(&token.tenant_id, REFRESH_KIND, &token.token_hash);
        let index_key =
            Self::family_member_key(&token.tenant_id, &token.family_id, &token.token_hash);

        self.put_record(&key, &token)?;
        // The index value is unused — membership is carried by the key itself.
        self.put_record(&index_key, &())
    }

    async fn consume_refresh_token(
        &self,
        tenant_id: &str,
        token_hash: &str,
        now: i64,
    ) -> AuthServerResult<Option<RefreshToken>> {
        let key = Self::build_key(tenant_id, REFRESH_KIND, token_hash);

        // Read and mark must not interleave: two concurrent redemptions that
        // both saw `consumed_at == None` would each believe they were first,
        // and the replay would go undetected.
        let _guard = self.locks().lock(key.clone()).await;

        let Some(record) = self.get_record::<RefreshToken>(&key)? else {
            return Ok(None);
        };

        if record.consumed_at.is_none() {
            let mut marked = record.clone();
            marked.consumed_at = Some(now);
            self.put_record(&key, &marked)?;
        }

        // Return the state as it was BEFORE the mark, so the caller can tell a
        // first redemption from a replay.
        Ok(Some(record))
    }

    async fn revoke_refresh_family(
        &self,
        tenant_id: &str,
        family_id: &str,
    ) -> AuthServerResult<usize> {
        let cf = self.cf()?;
        let prefix = Self::build_family_prefix(tenant_id, family_id);

        // Seek explicitly rather than using `prefix_iterator_cf`: these keys are
        // longer than any configured prefix extractor, and a seek-and-stop is
        // correct regardless of how the column family is configured.
        let iter = self.db().iterator_cf(
            cf,
            rocksdb::IteratorMode::From(&prefix, rocksdb::Direction::Forward),
        );

        // Collect first: deleting while iterating the same range is not sound.
        let mut hashes = Vec::new();
        for item in iter {
            let (key, _) = item.map_err(|e| {
                raisin_auth::authserver::AuthServerError::ServerError(format!(
                    "refresh family scan failed: {e}"
                ))
            })?;
            if !key.starts_with(&prefix) {
                break;
            }
            let hash = String::from_utf8_lossy(&key[prefix.len()..]).into_owned();
            hashes.push(hash);
        }

        let mut revoked = 0;
        for hash in &hashes {
            self.delete_record(&Self::build_key(tenant_id, REFRESH_KIND, hash))?;
            self.delete_record(&Self::family_member_key(tenant_id, family_id, hash))?;
            revoked += 1;
        }
        Ok(revoked)
    }
}
