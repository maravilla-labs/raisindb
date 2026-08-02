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
/// Key discriminator for the revoked-family tombstone.
const REVOKED_FAMILY_KIND: &str = "oauth_rt_revoked";

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
        // Refuse to (re)create a member of a family that was revoked.
        let tombstone = Self::build_key(&token.tenant_id, REVOKED_FAMILY_KIND, &token.family_id);
        if self.get_record::<String>(&tombstone)?.is_some() {
            return Err(raisin_auth::authserver::AuthServerError::InvalidGrant(
                "refresh token family has been revoked".to_string(),
            ));
        }

        let key = Self::build_key(&token.tenant_id, REFRESH_KIND, &token.token_hash);
        let index_key =
            Self::family_member_key(&token.tenant_id, &token.family_id, &token.token_hash);

        self.put_record(&key, &token)?;
        // The index value is unused — membership is carried by the key itself.
        self.put_record(&index_key, &())?;

        if let Some(capture) = self.capture() {
            if let Err(e) = capture
                .capture_upsert_oauth_refresh_token(
                    token.tenant_id.clone(),
                    token.clone(),
                    "system".to_string(),
                )
                .await
            {
                tracing::error!(
                    error = %e,
                    "failed to capture refresh token for replication; it exists on this node only"
                );
            }
        }
        Ok(())
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

        // Tombstone the family BEFORE removing members. A rotation captured
        // moments before the replay was noticed can still be in flight; the
        // tombstone is what stops it from resurrecting the family when it
        // lands, here and on every other node (see `oauth_operations.rs`).
        self.put_record(
            &Self::build_key(tenant_id, REVOKED_FAMILY_KIND, family_id),
            &family_id.to_string(),
        )?;

        let mut revoked = 0;
        for hash in &hashes {
            self.delete_record(&Self::build_key(tenant_id, REFRESH_KIND, hash))?;
            self.delete_record(&Self::family_member_key(tenant_id, family_id, hash))?;
            revoked += 1;
        }

        if let Some(capture) = self.capture() {
            if let Err(e) = capture
                .capture_revoke_oauth_refresh_family(
                    tenant_id.to_string(),
                    family_id.to_string(),
                    "system".to_string(),
                )
                .await
            {
                // A revocation that does not replicate leaves a leaked token
                // usable on the other nodes, so this is louder than the others.
                tracing::error!(
                    error = %e,
                    tenant_id,
                    family_id,
                    "failed to replicate refresh-family revocation; \
                     the leaked family may remain usable on other nodes"
                );
            }
        }
        Ok(revoked)
    }
}
