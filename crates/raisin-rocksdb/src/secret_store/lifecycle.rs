// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Node-lifecycle primitives over `cf::SECRETS`: **no keyring required**.
//!
//! # Why these are free functions and not [`SecretStore`] methods
//!
//! [`SecretStore`] owns a [`SecretBox`](raisin_crypto::SecretBox), i.e. a master
//! keyring, because sealing and opening need one. Everything in this file moves,
//! copies or retires *sealed bytes* without ever looking inside them, so none of
//! it needs a key — and the paths that call it must not be able to fail for want
//! of one:
//!
//! - the node **delete** funnel ([`crate::tombstones`]) runs on every delete in
//!   the system, including on a deployment that has no keyring configured;
//! - **cross-branch promotion** copies the record verbatim precisely so the
//!   ciphertext and `key_id` are preserved — decrypting and re-sealing would
//!   need the plaintext and would change the `key_id`;
//! - the **replication applicator** writes what a peer sealed, under the peer's
//!   `key_id`, which this node may not even hold.
//!
//! The one lifecycle operation that DOES need a keyring is a node **copy**
//! (a new node id means a new secret name, so the value must be re-sealed under
//! it); that one lives with the copy path and goes through `SecretStore`.
//!
//! # One scan, not several
//!
//! [`read_versions`] is the only version scan in the crate — [`SecretStore`]'s
//! own reads delegate to it. A second scan that located the descending-HLC
//! trailer differently would read a different set of versions than the writer
//! wrote, which is the drift the key module's header warns about.

use chrono::Utc;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::secret_ref::{node_field_secret_name, SecretRef};
use rocksdb::{ColumnFamily, Direction, IteratorMode, WriteBatch, DB};

use super::keys::{name_from_key, revision_from_key, secret_versions_prefix};
use super::{Result, SecretError, SecretRecord, SecretScope};
use crate::cf;
use crate::indexing::property_walk::walk_properties;

/// One stored version: the record, plus the storage revision that keys it.
///
/// The revision is not inside the record — it is the key's trailer — so
/// anything that wants to rewrite a version verbatim (promotion, replication)
/// has to carry it alongside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSecret {
    pub name: String,
    pub revision: HLC,
    pub record: SecretRecord,
}

/// The `cf::SECRETS` handle, or a typed "not open" error.
pub fn secrets_cf(db: &DB) -> Result<&ColumnFamily> {
    db.cf_handle(cf::SECRETS)
        .ok_or(SecretError::ColumnFamilyMissing(cf::SECRETS))
}

/// Every version of one secret, **newest first**, up to `limit`.
///
/// THE one version scan (see the module header).
pub fn read_versions(
    db: &DB,
    scope: &SecretScope,
    name: &str,
    limit: Option<usize>,
) -> Result<Vec<StoredSecret>> {
    let handle = secrets_cf(db)?;
    let prefix = secret_versions_prefix(&scope.tenant, &scope.repo, &scope.branch, name);
    scan_from(db, handle, &prefix, limit, |_| true)
}

/// The newest version of one secret, or `None` if the name has none.
pub fn newest_version(db: &DB, scope: &SecretScope, name: &str) -> Result<Option<StoredSecret>> {
    Ok(read_versions(db, scope, name, Some(1))?.into_iter().next())
}

/// The newest record carrying ordinal `version`, or the newest of all when
/// `version` is `None`.
///
/// Newest-first resolution is what makes a duplicated ordinal (see
/// [`SecretRecord`]) deterministic rather than arbitrary.
pub fn version_of(
    db: &DB,
    scope: &SecretScope,
    name: &str,
    version: Option<u64>,
) -> Result<Option<StoredSecret>> {
    match version {
        None => newest_version(db, scope, name),
        Some(v) => Ok(read_versions(db, scope, name, None)?
            .into_iter()
            .find(|s| s.record.version == v)),
    }
}

/// The newest version of every secret whose NAME starts with `name_prefix`.
///
/// This is how a node's secrets are found on delete: the vaulted-field naming
/// convention is `node/{node_id}/{field.path}`, so `node/{node_id}/` is an exact
/// key prefix and no property walk is needed. That matters — a node being
/// deleted may hold a secret its *current* properties no longer reference (a
/// field cleared, then the node deleted), and a walk would miss it.
pub fn newest_with_name_prefix(
    db: &DB,
    scope: &SecretScope,
    name_prefix: &str,
) -> Result<Vec<StoredSecret>> {
    let handle = secrets_cf(db)?;
    // `secret_versions_prefix` appends the name/revision separator, which is
    // exactly what we must NOT do here: we want every name that *begins* with
    // this text, not one exact name.
    let mut prefix = crate::keys::KeyBuilder::new()
        .push(&scope.tenant)
        .push(&scope.repo)
        .push(&scope.branch)
        .build_prefix();
    prefix.extend_from_slice(name_prefix.as_bytes());

    let mut seen: Option<String> = None;
    scan_from(db, handle, &prefix, None, move |s: &StoredSecret| {
        // Versions of one name are contiguous and newest-first, so the FIRST
        // key for a name is its newest version.
        if seen.as_deref() == Some(s.name.as_str()) {
            return false;
        }
        seen = Some(s.name.clone());
        true
    })
}

/// Forward prefix scan, deserializing each record and offering it to `keep`.
fn scan_from(
    db: &DB,
    handle: &ColumnFamily,
    prefix: &[u8],
    limit: Option<usize>,
    mut keep: impl FnMut(&StoredSecret) -> bool,
) -> Result<Vec<StoredSecret>> {
    let mut out = Vec::new();
    for item in db.iterator_cf(handle, IteratorMode::From(prefix, Direction::Forward)) {
        let (key, value) = item.map_err(|e| SecretError::Storage(e.to_string()))?;
        if !key.starts_with(prefix) {
            break;
        }
        let (Some(name), Some(revision)) = (name_from_key(&key), revision_from_key(&key)) else {
            continue;
        };
        let record: SecretRecord = rmp_serde::from_slice(&value)
            .map_err(|e| SecretError::Encoding(format!("deserialize secret record: {e}")))?;
        let stored = StoredSecret {
            name,
            revision,
            record,
        };
        if !keep(&stored) {
            continue;
        }
        out.push(stored);
        if let Some(n) = limit {
            if out.len() >= n {
                break;
            }
        }
    }
    Ok(out)
}

/// Stage one record, verbatim, at an explicit revision.
///
/// **Verbatim** is the contract: same ciphertext, same `key_id`, same ordinal.
/// Every caller of this is moving bytes someone else sealed.
pub fn write_record_to_batch(
    batch: &mut WriteBatch,
    handle: &ColumnFamily,
    scope: &SecretScope,
    name: &str,
    revision: &HLC,
    record: &SecretRecord,
) -> Result<()> {
    let key = super::keys::secret_key(&scope.tenant, &scope.repo, &scope.branch, name, revision);
    let value = rmp_serde::to_vec_named(record)
        .map_err(|e| SecretError::Encoding(format!("serialize secret record: {e}")))?;
    batch.put_cf(handle, key, value);
    Ok(())
}

/// Write one record verbatim, immediately (no batch). Used by the replication
/// applicator, which has no batch to join.
pub fn write_record(
    db: &DB,
    scope: &SecretScope,
    name: &str,
    revision: &HLC,
    record: &SecretRecord,
) -> Result<()> {
    let handle = secrets_cf(db)?;
    let key = super::keys::secret_key(&scope.tenant, &scope.repo, &scope.branch, name, revision);
    let value = rmp_serde::to_vec_named(record)
        .map_err(|e| SecretError::Encoding(format!("serialize secret record: {e}")))?;
    db.put_cf(handle, &key, &value)
        .map_err(|e| SecretError::Storage(e.to_string()))
}

/// The tombstone that retires `previous`.
///
/// A tombstone carries an EMPTY ciphertext and prior versions are left intact,
/// so a pinned `secret://name@N` on an older node revision still resolves —
/// which is what makes time-travel reads of a deleted node's history work.
///
/// `key_id` is `0` and deliberately meaningless: there is nothing to open, and
/// [`SecretStore::get`](super::SecretStore::get) reports `Gone` before it ever
/// consults the keyring. A tombstone must be writable on a node with no keyring
/// at all — see the module header.
pub fn tombstone_of(previous: &StoredSecret, actor: Option<&str>) -> SecretRecord {
    SecretRecord {
        ciphertext: Vec::new(),
        key_id: 0,
        version: previous.record.version.saturating_add(1),
        created_at: Utc::now().to_rfc3339(),
        created_by: actor.unwrap_or("system").to_string(),
        rotated_at: None,
        owner_node: previous.record.owner_node.clone(),
        owner_field: previous.record.owner_field.clone(),
        deleted: true,
    }
}

/// A revision guaranteed to sort as NEWER than `previous` under the descending
/// key encoding.
///
/// The caller's natural candidate is the node revision that triggered the
/// change, which is almost always later. Almost: the secret store and the node
/// revision allocator may be different [`NodeHLCState`](raisin_hlc::NodeHLCState)
/// instances (they share one inside the server, but not in every test or tool),
/// and a tombstone that does not sort ahead of the version it retires is a
/// tombstone that never takes effect — the deleted secret would keep resolving.
pub fn strictly_after(candidate: HLC, previous: HLC) -> HLC {
    if candidate > previous {
        candidate
    } else {
        HLC::new(previous.timestamp_ms, previous.counter.saturating_add(1))
    }
}

/// Every `secret://` reference in a node's property tree, with its dot path.
///
/// Uses the ONE property walker, so it addresses exactly what the vaulter
/// rewrote — a second traversal here is how a promotion copies a different set
/// of secrets than the write path minted.
pub fn secret_refs(node: &Node) -> Vec<(String, SecretRef)> {
    secret_refs_in(&node.properties)
}

/// [`secret_refs`] over a bare property map.
pub fn secret_refs_in(
    properties: &std::collections::HashMap<String, PropertyValue>,
) -> Vec<(String, SecretRef)> {
    walk_properties(properties, |_, v| match v {
        PropertyValue::String(s) if SecretRef::is_secret_ref(s) => Some(s.as_str()),
        _ => None,
    })
    .into_iter()
    .filter_map(|(path, raw)| SecretRef::parse(raw).ok().map(|r| (path, r)))
    .collect()
}

/// The names of secrets this node OWNS: `node/{node_id}/…`.
pub fn owned_secret_names(
    node_id: &str,
    properties: &std::collections::HashMap<String, PropertyValue>,
) -> Vec<String> {
    let prefix = node_field_secret_name(node_id, "");
    secret_refs_in(properties)
        .into_iter()
        .map(|(_, r)| r.name)
        .filter(|n| n.starts_with(&prefix))
        .collect()
}

/// Secret names the OLD revision of a node owned and the NEW one no longer
/// references — i.e. an `encrypted` field that was removed or set null.
///
/// Restricted to `node/{id}/…` names on purpose: a node may reference an
/// operator-created secret by name, and dropping that reference must not
/// destroy a secret the node never owned.
pub fn cleared_secret_names(old: &Node, new: &Node) -> Vec<String> {
    let kept: std::collections::HashSet<String> = owned_secret_names(&new.id, &new.properties)
        .into_iter()
        .collect();
    let mut cleared: Vec<String> = owned_secret_names(&old.id, &old.properties)
        .into_iter()
        .filter(|n| !kept.contains(n))
        .collect();
    cleared.sort();
    cleared.dedup();
    cleared
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn props(pairs: &[(&str, &str)]) -> HashMap<String, PropertyValue> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), PropertyValue::String((*v).to_string())))
            .collect()
    }

    fn cleared(old: &[(&str, &str)], new: &[(&str, &str)]) -> Vec<String> {
        let kept: std::collections::HashSet<String> =
            owned_secret_names("n1", &props(new)).into_iter().collect();
        let mut out: Vec<String> = owned_secret_names("n1", &props(old))
            .into_iter()
            .filter(|n| !kept.contains(n))
            .collect();
        out.sort();
        out
    }

    #[test]
    fn owned_names_exclude_foreign_secrets() {
        let p = props(&[
            ("password", "secret://node/n1/password@2"),
            ("shared", "secret://stripe_key"),
            ("plain", "hello"),
        ]);
        assert_eq!(owned_secret_names("n1", &p), vec!["node/n1/password"]);
    }

    #[test]
    fn clearing_a_field_reports_its_name_and_keeps_the_others() {
        assert_eq!(
            cleared(
                &[
                    ("password", "secret://node/n1/password@2"),
                    ("token", "secret://node/n1/token@1"),
                ],
                &[("token", "secret://node/n1/token@1")],
            ),
            vec!["node/n1/password"]
        );
    }

    /// Rewriting a field mints a NEW version under the SAME name, so nothing is
    /// cleared. Reporting it would tombstone the value that was just written.
    #[test]
    fn rewriting_a_field_clears_nothing() {
        assert!(cleared(
            &[("password", "secret://node/n1/password@2")],
            &[("password", "secret://node/n1/password@3")],
        )
        .is_empty());
    }

    /// Dropping a reference to a secret the node does not own must not retire it.
    #[test]
    fn dropping_a_foreign_reference_clears_nothing() {
        assert!(cleared(&[("shared", "secret://stripe_key@1")], &[]).is_empty());
    }

    #[test]
    fn a_tombstone_sorts_ahead_of_the_version_it_retires() {
        let previous = HLC::new(1_000, 5);
        // A candidate that is NOT later must still produce a later revision, or
        // the tombstone never shadows anything.
        let bumped = strictly_after(HLC::new(900, 0), previous);
        assert!(bumped > previous);
        assert_eq!(
            strictly_after(HLC::new(2_000, 0), previous),
            HLC::new(2_000, 0)
        );
    }
}
