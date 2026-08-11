//! `put` / `get` / `list` / `delete` / `rotate`.

use chrono::Utc;
use raisin_crypto::{Envelope, SecretContext};
use raisin_hlc::HLC;
use raisin_models::secret_ref::validate_secret_name;
use rocksdb::{Direction, IteratorMode};

use super::keys::{
    branch_secrets_prefix, name_from_key, revision_from_key, secret_versions_prefix,
};
use super::{
    Result, SecretError, SecretMetadata, SecretOwner, SecretRecord, SecretScope, SecretStore,
};
use crate::cf;

/// The key id a sealed blob was written under — or a refusal.
///
/// The node-secret family is `V1Policy::Reject`, so a v1 (context-free) blob
/// stored here could never be read back: it would be write-only data, and the
/// failure would surface much later, at reveal time, looking like corruption.
/// `SecretBox::seal` emits v1 until `RAISIN_CRYPTO_EMIT_V2` is set — the
/// deliberate rolling-upgrade gate for the *pre-existing* families. This store
/// neither fights that gate nor writes through it; it refuses and names the
/// variable.
///
/// Split out as a free function purely so the refusal is testable without
/// mutating process-wide environment from a parallel test.
pub(super) fn key_id_or_refuse(sealed: &[u8]) -> Result<u16> {
    match raisin_crypto::envelope::parse(sealed)? {
        Envelope::V2 { key_id, .. } => Ok(key_id),
        Envelope::V1 { .. } => Err(SecretError::V2EmissionRequired),
    }
}

impl SecretStore {
    // ---- write ----------------------------------------------------------

    /// Append a new version of `name` and return `(name, version_ordinal)`.
    ///
    /// Never overwrites: the key carries a freshly ticked HLC, so a previous
    /// version stays readable through `get(.., Some(v))` forever.
    pub fn put(
        &self,
        scope: &SecretScope,
        name: &str,
        plaintext: &[u8],
        owner: &SecretOwner,
    ) -> Result<(String, u64)> {
        self.append(scope, name, plaintext, owner, false)
    }

    /// Same as [`SecretStore::put`], but stamps `rotated_at`.
    ///
    /// A rotation is deliberately an append and not an in-place replacement:
    /// anything still holding a pinned `secret://name@N` reference — an older
    /// node revision, a running flow — keeps resolving.
    pub fn rotate(
        &self,
        scope: &SecretScope,
        name: &str,
        new_plaintext: &[u8],
        owner: &SecretOwner,
    ) -> Result<(String, u64)> {
        self.append(scope, name, new_plaintext, owner, true)
    }

    /// Append a tombstone version. Prior versions are left intact.
    ///
    /// Returns the tombstone's ordinal. Deleting an unknown name is not an
    /// error — the tombstone is what a converging replica needs to see.
    pub fn delete(&self, scope: &SecretScope, name: &str, owner: &SecretOwner) -> Result<u64> {
        validate_secret_name(name)?;
        let version = self.next_ordinal(scope, name)?;
        let record = SecretRecord {
            ciphertext: Vec::new(),
            key_id: self.secret_box.keys().active().0,
            version,
            created_at: Utc::now().to_rfc3339(),
            created_by: owner.actor_or_anonymous(),
            rotated_at: None,
            owner_node: owner.node.clone(),
            owner_field: owner.field.clone(),
            deleted: true,
        };
        self.write_record(scope, name, &record)?;
        Ok(version)
    }

    fn append(
        &self,
        scope: &SecretScope,
        name: &str,
        plaintext: &[u8],
        owner: &SecretOwner,
        is_rotation: bool,
    ) -> Result<(String, u64)> {
        validate_secret_name(name)?;

        let ctx = SecretContext::node_secret(&scope.tenant, &scope.repo)?;
        let sealed = self.secret_box.seal(&ctx, plaintext)?;

        let key_id = key_id_or_refuse(&sealed)?;

        let now = Utc::now().to_rfc3339();
        let record = SecretRecord {
            ciphertext: sealed,
            key_id,
            version: self.next_ordinal(scope, name)?,
            created_at: now.clone(),
            created_by: owner.actor_or_anonymous(),
            rotated_at: is_rotation.then_some(now),
            owner_node: owner.node.clone(),
            owner_field: owner.field.clone(),
            deleted: false,
        };
        let version = record.version;
        self.write_record(scope, name, &record)?;
        Ok((name.to_string(), version))
    }

    fn write_record(&self, scope: &SecretScope, name: &str, record: &SecretRecord) -> Result<()> {
        let handle = self.cf()?;
        let revision = self.hlc.tick();
        let key =
            super::keys::secret_key(&scope.tenant, &scope.repo, &scope.branch, name, &revision);
        let value = rmp_serde::to_vec_named(record)
            .map_err(|e| SecretError::Encoding(format!("serialize secret record: {e}")))?;
        self.db
            .put_cf(handle, &key, &value)
            .map_err(|e| SecretError::Storage(e.to_string()))
    }

    /// One past the newest ordinal, or 1 for a name with no versions.
    ///
    /// A racing derivation cannot lose data: the ordinal is not in the key, so
    /// two writers that both compute `3` produce two distinct records that both
    /// claim ordinal 3, and `get` resolves newest-first. See [`SecretRecord`].
    fn next_ordinal(&self, scope: &SecretScope, name: &str) -> Result<u64> {
        Ok(match self.newest(scope, name)? {
            Some((_, r)) => r.version.saturating_add(1),
            None => 1,
        })
    }

    // ---- read -----------------------------------------------------------

    /// The plaintext of `name`. **The only place plaintext exists.**
    ///
    /// `version = None` reads the newest version. A tombstone yields
    /// [`SecretError::Gone`]; an absent version yields [`SecretError::Pending`]
    /// — see the module docs for why those are not the same error.
    pub fn get(&self, scope: &SecretScope, name: &str, version: Option<u64>) -> Result<Vec<u8>> {
        validate_secret_name(name)?;

        let (_, record) = match version {
            None => self
                .newest(scope, name)?
                .ok_or_else(|| SecretError::Pending {
                    name: name.to_string(),
                    version: None,
                })?,
            Some(v) => self
                .version(scope, name, v)?
                .ok_or_else(|| SecretError::Pending {
                    name: name.to_string(),
                    version: Some(v),
                })?,
        };

        if record.deleted {
            return Err(SecretError::Gone {
                name: name.to_string(),
                version: record.version,
            });
        }

        // Checked before the decrypt so a keyring gap is reported as a keyring
        // gap, not as an authentication failure indistinguishable from
        // corruption.
        if self.secret_box.keys().get(record.key_id).is_none() {
            return Err(SecretError::UnknownKeyId {
                name: name.to_string(),
                version: record.version,
                key_id: record.key_id,
                available: self.secret_box.keys().available_ids(),
            });
        }

        let ctx = SecretContext::node_secret(&scope.tenant, &scope.repo)?;
        Ok(self.secret_box.open(&ctx, &record.ciphertext)?)
    }

    /// Metadata for the newest version of every secret in the branch.
    ///
    /// **Never returns ciphertext or plaintext** — [`SecretMetadata`] has no
    /// field that could hold either. Tombstoned secrets are included, flagged
    /// `deleted`, so an operator can see that a name was retired rather than
    /// concluding it never existed.
    pub fn list(&self, scope: &SecretScope) -> Result<Vec<SecretMetadata>> {
        let handle = self.cf()?;
        let prefix = branch_secrets_prefix(&scope.tenant, &scope.repo, &scope.branch);

        let mut out: Vec<SecretMetadata> = Vec::new();
        let mut current: Option<String> = None;

        for item in self
            .db
            .iterator_cf(handle, IteratorMode::From(&prefix, Direction::Forward))
        {
            let (key, value) = item.map_err(|e| SecretError::Storage(e.to_string()))?;
            if !key.starts_with(&prefix) {
                break;
            }
            let Some(name) = name_from_key(&key) else {
                continue;
            };
            // Versions of one name are contiguous and newest-first, so the
            // first key for a name IS its newest version.
            if current.as_deref() == Some(name.as_str()) {
                continue;
            }
            let Some(revision) = revision_from_key(&key) else {
                continue;
            };
            let record: SecretRecord = rmp_serde::from_slice(&value)
                .map_err(|e| SecretError::Encoding(format!("deserialize secret record: {e}")))?;
            current = Some(name.clone());
            out.push(SecretMetadata::from_record(name, revision, &record));
        }
        Ok(out)
    }

    /// Metadata for every version of one secret, newest first.
    pub fn list_versions(&self, scope: &SecretScope, name: &str) -> Result<Vec<SecretMetadata>> {
        validate_secret_name(name)?;
        Ok(self
            .scan_versions(scope, name, None)?
            .into_iter()
            .map(|(rev, r)| SecretMetadata::from_record(name.to_string(), rev, &r))
            .collect())
    }

    // ---- internals ------------------------------------------------------

    fn cf(&self) -> Result<&rocksdb::ColumnFamily> {
        self.db
            .cf_handle(cf::SECRETS)
            .ok_or(SecretError::ColumnFamilyMissing(cf::SECRETS))
    }

    /// The newest version, i.e. the FIRST entry of a forward prefix scan.
    fn newest(&self, scope: &SecretScope, name: &str) -> Result<Option<(HLC, SecretRecord)>> {
        Ok(self.scan_versions(scope, name, Some(1))?.into_iter().next())
    }

    /// The newest record carrying ordinal `version`.
    ///
    /// Newest-first resolution is what makes a duplicated ordinal (see
    /// [`SecretRecord`]) deterministic rather than arbitrary.
    fn version(
        &self,
        scope: &SecretScope,
        name: &str,
        version: u64,
    ) -> Result<Option<(HLC, SecretRecord)>> {
        Ok(self
            .scan_versions(scope, name, None)?
            .into_iter()
            .find(|(_, r)| r.version == version))
    }

    fn scan_versions(
        &self,
        scope: &SecretScope,
        name: &str,
        limit: Option<usize>,
    ) -> Result<Vec<(HLC, SecretRecord)>> {
        let handle = self.cf()?;
        let prefix = secret_versions_prefix(&scope.tenant, &scope.repo, &scope.branch, name);

        let mut out = Vec::new();
        for item in self
            .db
            .iterator_cf(handle, IteratorMode::From(&prefix, Direction::Forward))
        {
            let (key, value) = item.map_err(|e| SecretError::Storage(e.to_string()))?;
            if !key.starts_with(&prefix) {
                break;
            }
            let Some(revision) = revision_from_key(&key) else {
                continue;
            };
            let record: SecretRecord = rmp_serde::from_slice(&value)
                .map_err(|e| SecretError::Encoding(format!("deserialize secret record: {e}")))?;
            out.push((revision, record));
            if let Some(n) = limit {
                if out.len() >= n {
                    break;
                }
            }
        }
        Ok(out)
    }
}
