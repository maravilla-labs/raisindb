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

//! The retired user keys ledger: a deleted person never comes back.
//!
//! When a keyed user home (`/users/internal/<user_key>` in
//! `raisin:access_control`, see `raisin_models::auth::user_key`) is deleted,
//! its key is recorded here, with the time and NOTHING else (no email, no
//! name, no repository). The ledger lives at TENANT level, next to the
//! identities (`cf::IDENTITIES`, key `{tenant}\0retired_user_keys\0{key}`),
//! not in repository content: deleting, recreating or restoring a repository
//! of the tenant does not lose it.
//!
//! - Every delete path stages the entry in the SAME batch as the node's
//!   tombstones (`crate::tombstones`), so a delete and its ledger entry commit
//!   together and a rolled-back delete retires nothing.
//! - Imports refuse a retired home with its subtree and its relations:
//!   package install (`jobs::handlers::package_install`), backup restore
//!   (`management::backup`) and any write that would keep a carried key
//!   (`crate::user_homes`). A minted key is never a retired one.
//! - Backups export the ledger (`retired_user_keys.jsonl`) and a restore
//!   MERGES it into the tenant ledger (union; never replaces, never removes).
//! - The only way past it is an explicit admin override on an install or a
//!   restore (deliberate disaster recovery), logged at warn as an audit event.
//!
//! A person who signs up again after being deleted gets a new home with a new
//! key: a new person, by design. What the ledger refuses is bringing back the
//! OLD home under the old key.

use crate::{cf, cf_handle, keys::KeyBuilder};
use raisin_error::{Error, Result};
use raisin_models::auth::user_key::{user_key_from_home, USER_NODE_TYPE, USER_WORKSPACE};
use raisin_models::nodes::Node;
use rocksdb::{WriteBatch, DB};
use serde::{Deserialize, Serialize};

/// Version of an exported ledger line.
pub const RETIRED_KEY_LINE_VERSION: u32 = 1;

/// One ledger entry: the key and when it was retired. Nothing else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetiredUserKey {
    /// Line format version (exports).
    #[serde(default = "line_version")]
    pub v: u32,
    /// The retired user key (`u_` + 22 base62).
    pub key: String,
    /// When it was retired (RFC 3339, UTC).
    pub retired_at: String,
}

fn line_version() -> u32 {
    RETIRED_KEY_LINE_VERSION
}

#[derive(Serialize, Deserialize)]
struct StoredEntry {
    retired_at: String,
}

fn ledger_key(tenant_id: &str, user_key: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push("retired_user_keys")
        .push(user_key)
        .build()
}

fn ledger_prefix(tenant_id: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push("retired_user_keys")
        .build_prefix()
}

/// The key of `node` when it is a keyed user home in `workspace`.
pub fn home_key<'a>(workspace: &str, node: &'a Node) -> Option<&'a str> {
    if workspace != USER_WORKSPACE || node.node_type != USER_NODE_TYPE {
        return None;
    }
    user_key_from_home(&node.path)
}

/// Is `user_key` retired in `tenant_id`?
pub fn is_retired(db: &DB, tenant_id: &str, user_key: &str) -> Result<bool> {
    let cf = cf_handle(db, cf::IDENTITIES)?;
    Ok(db
        .get_cf(cf, ledger_key(tenant_id, user_key))
        .map_err(|e| Error::storage(format!("retired key lookup: {e}")))?
        .is_some())
}

/// Stage the ledger entry for a deleted node into `batch` (no-op unless it is
/// a keyed user home). Keeps the first retirement time of a key.
pub fn stage_retire(
    batch: &mut WriteBatch,
    db: &DB,
    tenant_id: &str,
    workspace: &str,
    node: &Node,
) -> Result<()> {
    let Some(key) = home_key(workspace, node) else {
        return Ok(());
    };
    if is_retired(db, tenant_id, key)? {
        return Ok(());
    }
    let cf = cf_handle(db, cf::IDENTITIES)?;
    let value = serde_json::to_vec(&StoredEntry {
        retired_at: chrono::Utc::now().to_rfc3339(),
    })
    .map_err(|e| Error::storage(format!("retired key encode: {e}")))?;
    batch.put_cf(cf, ledger_key(tenant_id, key), value);
    tracing::info!(user_key = %key, "User key retired");
    Ok(())
}

/// Refuse writing `node` when it is a user home (or anything below one) whose
/// key is retired: used by the branch copy and merge writers, which bypass
/// the create funnel.
pub fn reject_retired_home(db: &DB, tenant_id: &str, workspace: &str, node: &Node) -> Result<()> {
    if workspace != USER_WORKSPACE {
        return Ok(());
    }
    if let Some(key) = raisin_models::auth::user_key::user_key_of_path(&node.path) {
        if is_retired(db, tenant_id, key)? {
            return Err(Error::Conflict(format!(
                "user key {key} is retired (its person was deleted): the home cannot come back"
            )));
        }
    }
    Ok(())
}

/// Every retired key of `tenant_id`.
pub fn list(db: &DB, tenant_id: &str) -> Result<Vec<RetiredUserKey>> {
    let cf = cf_handle(db, cf::IDENTITIES)?;
    let prefix = ledger_prefix(tenant_id);
    let mut out = Vec::new();
    for item in crate::prefix_scan(db, cf, &prefix) {
        let (k, v) = item.map_err(|e| Error::storage(format!("retired key scan: {e}")))?;
        if !k.starts_with(&prefix) {
            break;
        }
        let key = String::from_utf8_lossy(&k[prefix.len()..])
            .trim_end_matches('\0')
            .to_string();
        let entry: StoredEntry = serde_json::from_slice(&v)
            .map_err(|e| Error::storage(format!("retired key decode: {e}")))?;
        out.push(RetiredUserKey {
            v: RETIRED_KEY_LINE_VERSION,
            key,
            retired_at: entry.retired_at,
        });
    }
    Ok(out)
}

/// Merge `entries` into the tenant ledger (union: an existing entry keeps its
/// time, nothing is removed). Malformed keys are refused. Returns how many
/// entries were new.
pub fn merge(db: &DB, tenant_id: &str, entries: &[RetiredUserKey]) -> Result<usize> {
    let cf = cf_handle(db, cf::IDENTITIES)?;
    let mut batch = WriteBatch::default();
    let mut added = 0;
    for e in entries {
        if !raisin_models::auth::is_user_key(&e.key) {
            return Err(Error::Validation(format!(
                "retired key ledger: '{}' is not a user key",
                e.key
            )));
        }
        if is_retired(db, tenant_id, &e.key)? {
            continue;
        }
        let value = serde_json::to_vec(&StoredEntry {
            retired_at: e.retired_at.clone(),
        })
        .map_err(|e| Error::storage(format!("retired key encode: {e}")))?;
        batch.put_cf(cf, ledger_key(tenant_id, &e.key), value);
        added += 1;
    }
    db.write(batch)
        .map_err(|e| Error::storage(format!("retired key merge: {e}")))?;
    Ok(added)
}

/// Log the admin override of the ledger (an audit event).
pub fn audit_override(tenant_id: &str, what: &str, actor: &str, keys: &[String]) {
    tracing::warn!(
        target: "raisin::audit",
        tenant_id = %tenant_id,
        actor = %actor,
        operation = %what,
        restored_retired_keys = ?keys,
        "Retired user keys restored by an explicit admin override (deleted persons brought back)"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> (tempfile::TempDir, crate::RocksDBStorage) {
        let dir = tempfile::tempdir().unwrap();
        let storage = crate::RocksDBStorage::new(dir.path()).unwrap();
        (dir, storage)
    }

    fn home(key: &str) -> Node {
        Node {
            id: "n".into(),
            node_type: USER_NODE_TYPE.into(),
            path: format!("/users/internal/{key}"),
            ..Default::default()
        }
    }

    const KEY: &str = "u_0123456789abcdefABCDEF";

    #[test]
    fn a_deleted_home_is_retired_with_its_key_and_time_only() {
        let (_d, s) = db();
        let mut b = WriteBatch::default();
        stage_retire(&mut b, s.db(), "t1", USER_WORKSPACE, &home(KEY)).unwrap();
        s.db().write(b).unwrap();
        assert!(is_retired(s.db(), "t1", KEY).unwrap());
        assert!(!is_retired(s.db(), "t2", KEY).unwrap(), "per tenant");
        let all = list(s.db(), "t1").unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].key, KEY);
        let json = serde_json::to_string(&all[0]).unwrap();
        assert!(!json.contains('@'), "no personal data: {json}");

        // Other nodes are not keys.
        let mut b = WriteBatch::default();
        let mut legacy = home(KEY);
        legacy.path = "/users/internal/alice-at-x-com".into();
        stage_retire(&mut b, s.db(), "t1", USER_WORKSPACE, &legacy).unwrap();
        stage_retire(
            &mut b,
            s.db(),
            "t1",
            "content",
            &home("u_ZZZZZZZZZZZZZZZZZZZZZZ"),
        )
        .unwrap();
        s.db().write(b).unwrap();
        assert_eq!(list(s.db(), "t1").unwrap().len(), 1);
    }

    #[test]
    fn merge_is_a_union_that_keeps_first_times() {
        let (_d, s) = db();
        let first = RetiredUserKey {
            v: 1,
            key: KEY.into(),
            retired_at: "2026-01-01T00:00:00Z".into(),
        };
        assert_eq!(merge(s.db(), "t", &[first.clone()]).unwrap(), 1);
        let later = RetiredUserKey {
            retired_at: "2026-06-01T00:00:00Z".into(),
            ..first.clone()
        };
        let other = RetiredUserKey {
            key: "u_ZZZZZZZZZZZZZZZZZZZZZZ".into(),
            ..first.clone()
        };
        assert_eq!(merge(s.db(), "t", &[later, other]).unwrap(), 1);
        let all = list(s.db(), "t").unwrap();
        assert_eq!(all.len(), 2);
        assert!(all
            .iter()
            .any(|e| e.key == KEY && e.retired_at == first.retired_at));
        let bad = RetiredUserKey {
            key: "alice".into(),
            ..first
        };
        assert!(merge(s.db(), "t", &[bad]).is_err());
    }
}
