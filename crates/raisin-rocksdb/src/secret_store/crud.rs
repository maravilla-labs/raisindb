//! `put` / `get` / `list` / `delete` / `rotate`.

use chrono::Utc;
use raisin_crypto::{Envelope, SecretContext};
use raisin_hlc::HLC;
use raisin_models::secret_ref::validate_secret_name;
use rocksdb::{Direction, IteratorMode};

use super::keys::{branch_secrets_prefix, name_from_key, revision_from_key};
use super::lifecycle::{self, StoredSecret};
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
        let stored = self.append(scope, name, plaintext, owner, false)?;
        Ok((stored.name, stored.record.version))
    }

    /// [`SecretStore::put`], returning the record as written.
    ///
    /// The write paths need this: what replicates is the sealed record AND the
    /// storage revision that keys it (which is not inside the record), so a peer
    /// can write the identical key rather than minting a divergent one.
    pub fn put_recorded(
        &self,
        scope: &SecretScope,
        name: &str,
        plaintext: &[u8],
        owner: &SecretOwner,
    ) -> Result<StoredSecret> {
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
        let stored = self.append(scope, name, new_plaintext, owner, true)?;
        Ok((stored.name, stored.record.version))
    }

    /// Append a tombstone version. Prior versions are left intact.
    ///
    /// Returns the tombstone's ordinal. Deleting an unknown name is not an
    /// error — the tombstone is what a converging replica needs to see.
    pub fn delete(&self, scope: &SecretScope, name: &str, owner: &SecretOwner) -> Result<u64> {
        Ok(self.delete_recorded(scope, name, owner)?.record.version)
    }

    /// [`SecretStore::delete`], returning the tombstone as written — see
    /// [`SecretStore::put_recorded`] for why the caller needs it.
    pub fn delete_recorded(
        &self,
        scope: &SecretScope,
        name: &str,
        owner: &SecretOwner,
    ) -> Result<StoredSecret> {
        validate_secret_name(name)?;
        let record = SecretRecord {
            ciphertext: Vec::new(),
            // Deliberately meaningless: there is nothing to open, and `get`
            // reports `Gone` before it consults the keyring. See
            // `lifecycle::tombstone_of`, which mints the same shape without a
            // keyring at all.
            key_id: 0,
            version: self.next_ordinal(scope, name)?,
            created_at: Utc::now().to_rfc3339(),
            created_by: owner.actor_or_anonymous(),
            rotated_at: None,
            owner_node: owner.node.clone(),
            owner_field: owner.field.clone(),
            deleted: true,
        };
        self.write_record(scope, name, &record)
    }

    /// Write one record verbatim at an explicit revision.
    ///
    /// The **replication** entry point: a peer's sealed bytes are written under
    /// the peer's `key_id`, at the peer's revision, so the two nodes hold
    /// byte-identical keys and a redelivery is idempotent. Never re-seals —
    /// that would need the plaintext and would change the `key_id`.
    pub fn put_verbatim(
        &self,
        scope: &SecretScope,
        name: &str,
        revision: &raisin_hlc::HLC,
        record: &SecretRecord,
    ) -> Result<()> {
        validate_secret_name(name)?;
        lifecycle::write_record(&self.db, scope, name, revision, record)
    }

    fn append(
        &self,
        scope: &SecretScope,
        name: &str,
        plaintext: &[u8],
        owner: &SecretOwner,
        is_rotation: bool,
    ) -> Result<StoredSecret> {
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
        self.write_record(scope, name, &record)
    }

    fn write_record(
        &self,
        scope: &SecretScope,
        name: &str,
        record: &SecretRecord,
    ) -> Result<StoredSecret> {
        let revision = self.hlc.tick();
        lifecycle::write_record(&self.db, scope, name, &revision, record)?;
        Ok(StoredSecret {
            name: name.to_string(),
            revision,
            record: record.clone(),
        })
    }

    /// One past the newest ordinal, or 1 for a name with no versions.
    ///
    /// A racing derivation cannot lose data: the ordinal is not in the key, so
    /// two writers that both compute `3` produce two distinct records that both
    /// claim ordinal 3, and `get` resolves newest-first. See [`SecretRecord`].
    fn next_ordinal(&self, scope: &SecretScope, name: &str) -> Result<u64> {
        Ok(match self.newest(scope, name)? {
            Some(s) => s.record.version.saturating_add(1),
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

        let record = lifecycle::version_of(&self.db, scope, name, version)?
            .ok_or_else(|| SecretError::Pending {
                name: name.to_string(),
                version,
            })?
            .record;

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
            .map(|s| SecretMetadata::from_record(s.name, s.revision, &s.record))
            .collect())
    }

    /// The stored record for one version, ciphertext included and unopened.
    ///
    /// The **promotion** entry point: a cross-branch copy moves this verbatim
    /// into the target branch. `None` when the version is absent.
    pub fn read_verbatim(
        &self,
        scope: &SecretScope,
        name: &str,
        version: Option<u64>,
    ) -> Result<Option<StoredSecret>> {
        validate_secret_name(name)?;
        lifecycle::version_of(&self.db, scope, name, version)
    }

    // ---- internals ------------------------------------------------------

    fn cf(&self) -> Result<&rocksdb::ColumnFamily> {
        lifecycle::secrets_cf(&self.db)
    }

    /// The newest version, i.e. the FIRST entry of a forward prefix scan.
    fn newest(&self, scope: &SecretScope, name: &str) -> Result<Option<StoredSecret>> {
        lifecycle::newest_version(&self.db, scope, name)
    }

    fn scan_versions(
        &self,
        scope: &SecretScope,
        name: &str,
        limit: Option<usize>,
    ) -> Result<Vec<StoredSecret>> {
        lifecycle::read_versions(&self.db, scope, name, limit)
    }
}
