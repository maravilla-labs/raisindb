//! The value stored in `cf::SECRETS`, and the metadata projection of it.

use raisin_hlc::HLC;
use serde::{Deserialize, Serialize};

/// One version of one secret.
///
/// Serialized with MessagePack (`rmp_serde`), matching every neighbouring CF in
/// this crate.
///
/// # `version` is an ordinal, not the ordering
///
/// The **HLC in the key** is what orders versions. `version` is a small monotone
/// integer that exists purely so a human-facing reference can say
/// `secret://name@3`. The next ordinal is derived by reading the newest record
/// and adding one.
///
/// Because the ordinal is *not* part of the key, a racing derivation cannot
/// overwrite anything: two writers that both read ordinal 2 mint two records
/// with distinct HLCs that both claim ordinal 3. The worst case is a duplicated
/// ordinal, which the read path resolves deterministically — `get` scans
/// newest-first and returns the first match, i.e. the later of the two writes.
/// The alternative (ordinal in the key) would make that race a silent
/// **overwrite** of a live secret, which is why it is not the key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretRecord {
    /// The sealed envelope. Never leaves the store.
    pub ciphertext: Vec<u8>,
    /// Which master key sealed it.
    ///
    /// Stored so a peer whose keyring lacks the id fails **loudly, at read**,
    /// naming the missing id — rather than surfacing as an opaque
    /// authentication failure that reads like corruption.
    pub key_id: u16,
    /// The human-facing ordinal (`secret://name@{version}`), from 1.
    pub version: u64,
    /// RFC 3339, stamped at write.
    pub created_at: String,
    /// The actor that wrote this version, or `"anonymous"`.
    pub created_by: String,
    /// Set only by [`super::SecretStore::rotate`]; `None` for a plain `put`.
    pub rotated_at: Option<String>,
    /// The node this secret backs, when it is a vaulted schema field.
    pub owner_node: Option<String>,
    /// The dot path of the field it backs.
    pub owner_field: Option<String>,
    /// Tombstone marker. A tombstone carries an EMPTY `ciphertext`; prior
    /// versions are never removed, so MVCC time-travel reads of older node
    /// revisions still resolve.
    pub deleted: bool,
}

impl SecretRecord {
    /// Whether this record is a well-formed tombstone.
    pub fn is_tombstone(&self) -> bool {
        self.deleted
    }
}

/// Everything about a secret version EXCEPT the secret.
///
/// This is what `list` returns and what every listing surface (HTTP, console,
/// CLI) is allowed to see. It deliberately has no field that could hold
/// ciphertext or plaintext; the type, not a careful caller, is what enforces
/// that.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretMetadata {
    pub name: String,
    pub version: u64,
    pub key_id: u16,
    pub created_at: String,
    pub created_by: String,
    pub rotated_at: Option<String>,
    pub owner_node: Option<String>,
    pub owner_field: Option<String>,
    pub deleted: bool,
    /// The storage revision. Ordering truth; the ordinal is only a label.
    pub revision: HLC,
    /// Byte length of the sealed envelope. Useful for diagnostics and quota,
    /// and safe to publish: it is the ciphertext's size, not its content.
    pub ciphertext_len: usize,
}

impl SecretMetadata {
    pub(super) fn from_record(name: String, revision: HLC, r: &SecretRecord) -> Self {
        Self {
            name,
            version: r.version,
            key_id: r.key_id,
            created_at: r.created_at.clone(),
            created_by: r.created_by.clone(),
            rotated_at: r.rotated_at.clone(),
            owner_node: r.owner_node.clone(),
            owner_field: r.owner_field.clone(),
            deleted: r.deleted,
            revision,
            ciphertext_len: r.ciphertext.len(),
        }
    }
}

/// Who and what a secret belongs to, supplied by the caller of `put`/`rotate`.
#[derive(Debug, Clone, Default)]
pub struct SecretOwner {
    /// Resolved actor id; `None` becomes `"anonymous"`, matching how the node
    /// write layer stamps `created_by`.
    pub actor: Option<String>,
    /// The node this secret backs, if any.
    pub node: Option<String>,
    /// The field path this secret backs, if any.
    pub field: Option<String>,
}

impl SecretOwner {
    /// An operator-created secret with no node behind it.
    pub fn actor(actor: impl Into<String>) -> Self {
        Self {
            actor: Some(actor.into()),
            ..Default::default()
        }
    }

    /// A secret backing a vaulted schema field on a node.
    pub fn node_field(
        actor: impl Into<String>,
        node: impl Into<String>,
        field: impl Into<String>,
    ) -> Self {
        Self {
            actor: Some(actor.into()),
            node: Some(node.into()),
            field: Some(field.into()),
        }
    }

    pub(super) fn actor_or_anonymous(&self) -> String {
        self.actor
            .clone()
            .unwrap_or_else(|| "anonymous".to_string())
    }
}
