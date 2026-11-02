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
//! - Only a real deletion of a person retires a key: a delete of the home on
//!   the authoritative branch (`main`, where `raisin:access_control` lives)
//!   through the repository, the transaction or a cascade
//!   ([`stage_retire_on_person_delete`]), staged in the SAME batch as the
//!   tombstones, so a rolled-back delete retires nothing. Branch prunes,
//!   merges, displacements, deletes on other branches and the replication
//!   apply never retire. And a key is never retired while a live home with it
//!   exists anywhere in the tenant ([`live_home_exists`]).
//! - Imports refuse a retired home that has no live home in the tenant,
//!   with its subtree and its relations:
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
        if retired_and_gone(db, tenant_id, key)? {
            return Err(Error::Conflict(format!(
                "user key {key} is retired (its person was deleted): the home cannot come back"
            )));
        }
    }
    Ok(())
}

/// Is there a live home with `user_key` anywhere in the tenant (any branch of
/// any repository), other than `except` (`(repo, branch, node_id)`, the node
/// being deleted)? Reads the newest PATH_INDEX entry of
/// `/users/internal/<key>` in `raisin:access_control` per branch.
pub fn live_home_exists(
    db: &DB,
    tenant_id: &str,
    user_key: &str,
    except: Option<(&str, &str, &str)>,
) -> Result<bool> {
    let cf_branches = cf_handle(db, cf::BRANCHES)?;
    let cf_path = cf_handle(db, cf::PATH_INDEX)?;
    let tenant_prefix = KeyBuilder::new().push(tenant_id).build_prefix();
    let home = raisin_models::auth::user_key::user_home_for_key(user_key);
    for item in crate::prefix_scan(db, cf_branches, &tenant_prefix) {
        let (k, _) = item.map_err(|e| Error::storage(format!("branch scan: {e}")))?;
        if !k.starts_with(&tenant_prefix) {
            break;
        }
        // {tenant}\0{repo}\0branches\0{branch}
        let parts: Vec<&[u8]> = k.split(|b| *b == 0).collect();
        if parts.len() < 4 || parts[2] != b"branches" {
            continue;
        }
        let repo = String::from_utf8_lossy(parts[1]).to_string();
        let branch = String::from_utf8_lossy(parts[3]).to_string();
        let prefix =
            crate::keys::path_index_key_prefix(tenant_id, &repo, &branch, USER_WORKSPACE, &home);
        let newest = crate::prefix_scan(db, cf_path, &prefix)
            .next()
            .transpose()
            .map_err(|e| Error::storage(format!("path scan: {e}")))?;
        let Some((pk, value)) = newest else {
            continue;
        };
        if !pk.starts_with(&prefix) || value.as_ref() == crate::keys::TOMBSTONE_VALUE {
            continue;
        }
        let node_id = String::from_utf8_lossy(&value).to_string();
        if except.is_some_and(|(r, b, id)| r == repo && b == branch && id == node_id) {
            continue;
        }
        return Ok(true);
    }
    Ok(false)
}

/// Stage the retirement of a deleted person's key: only for a delete of a
/// keyed home on `main` (where users live), and only when no other live home
/// with that key exists in the tenant (a copy on another branch or in another
/// repository is a living person, never erased by mistake).
#[allow(clippy::too_many_arguments)]
pub fn stage_retire_on_person_delete(
    batch: &mut WriteBatch,
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node: &Node,
) -> Result<()> {
    if branch != "main" {
        return Ok(());
    }
    let Some(key) = home_key(workspace, node) else {
        return Ok(());
    };
    if live_home_exists(db, tenant_id, key, Some((repo_id, branch, &node.id)))? {
        tracing::warn!(
            user_key = %key,
            "Deleted a user home whose key still has a live home in the tenant; not retired"
        );
        return Ok(());
    }
    stage_retire(batch, db, tenant_id, workspace, node)
}

/// Is `user_key` retired AND without any live home in the tenant? The test
/// an import applies before it leaves a home out: a stale or wrong ledger
/// entry can never erase a living person.
pub fn retired_and_gone(db: &DB, tenant_id: &str, user_key: &str) -> Result<bool> {
    if !is_retired(db, tenant_id, user_key)? {
        return Ok(false);
    }
    if live_home_exists(db, tenant_id, user_key, None)? {
        tracing::warn!(
            user_key = %user_key,
            "A retired user key has a live home in the tenant; it is not treated as retired"
        );
        return Ok(false);
    }
    Ok(true)
}

/// Remove `user_key` from the ledger (an administrator's correction of a
/// mistaken retirement; audit-logged with the key only). Returns whether the
/// key was retired.
pub fn unretire(db: &DB, tenant_id: &str, user_key: &str, actor: &str) -> Result<bool> {
    if !is_retired(db, tenant_id, user_key)? {
        return Ok(false);
    }
    let cf = cf_handle(db, cf::IDENTITIES)?;
    db.delete_cf(cf, ledger_key(tenant_id, user_key))
        .map_err(|e| Error::storage(format!("retired key delete: {e}")))?;
    tracing::warn!(
        target: "raisin::audit",
        tenant_id = %tenant_id,
        actor = %actor,
        user_key = %user_key,
        "Retired user key un-retired by an administrator"
    );
    Ok(true)
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

    fn put_live_home(db: &DB, tenant: &str, repo: &str, branch: &str, key: &str, id: &[u8]) {
        let cf_b = cf_handle(db, cf::BRANCHES).unwrap();
        let cf_p = cf_handle(db, cf::PATH_INDEX).unwrap();
        db.put_cf(cf_b, crate::keys::branch_key(tenant, repo, branch), b"x")
            .unwrap();
        db.put_cf(
            cf_p,
            crate::keys::path_index_key_versioned(
                tenant,
                repo,
                branch,
                USER_WORKSPACE,
                &raisin_models::auth::user_key::user_home_for_key(key),
                &raisin_hlc::HLC::new(1, 0),
            ),
            id,
        )
        .unwrap();
    }

    #[test]
    fn only_a_person_delete_on_main_without_a_live_copy_retires() {
        let (_d, s) = db();
        let db = s.db().as_ref();
        let n = home(KEY);

        // The generic tombstoner (prunes, merges, displacements, replication)
        // never retires.
        let mut b = WriteBatch::default();
        let ctx = crate::tombstones::TombstoneContext::new("t", "r1", "main", USER_WORKSPACE);
        let cfs = crate::tombstones::TombstoneColumnFamilies::from_db(db).unwrap();
        crate::tombstones::add_node_tombstones(
            &mut b,
            db,
            &ctx,
            &cfs,
            &n,
            &raisin_hlc::HLC::new(2, 0),
        )
        .unwrap();
        db.write(b).unwrap();
        assert!(!is_retired(db, "t", KEY).unwrap());

        // A delete on another branch (a scratch branch) does not retire.
        let mut b = WriteBatch::default();
        stage_retire_on_person_delete(&mut b, db, "t", "r1", "scratch", USER_WORKSPACE, &n)
            .unwrap();
        db.write(b).unwrap();
        assert!(!is_retired(db, "t", KEY).unwrap());

        // A live home with the key elsewhere in the tenant: not retired.
        put_live_home(db, "t", "r2", "main", KEY, b"other-id");
        let mut b = WriteBatch::default();
        stage_retire_on_person_delete(&mut b, db, "t", "r1", "main", USER_WORKSPACE, &n).unwrap();
        db.write(b).unwrap();
        assert!(!is_retired(db, "t", KEY).unwrap());

        // The home being deleted itself does not count as a live copy.
        let (_d2, s2) = super::tests::db();
        let db2 = s2.db().as_ref();
        put_live_home(db2, "t", "r1", "main", KEY, b"n");
        let mut b = WriteBatch::default();
        stage_retire_on_person_delete(&mut b, db2, "t", "r1", "main", USER_WORKSPACE, &n).unwrap();
        db2.write(b).unwrap();
        assert!(is_retired(db2, "t", KEY).unwrap());
    }

    #[test]
    fn a_live_home_with_a_retired_key_is_not_treated_as_gone_and_unretire_works() {
        let (_d, s) = db();
        let db = s.db().as_ref();
        merge(
            db,
            "t",
            &[RetiredUserKey {
                v: 1,
                key: KEY.into(),
                retired_at: "2026-01-01T00:00:00Z".into(),
            }],
        )
        .unwrap();
        assert!(retired_and_gone(db, "t", KEY).unwrap());
        put_live_home(db, "t", "r9", "main", KEY, b"alive");
        assert!(
            !retired_and_gone(db, "t", KEY).unwrap(),
            "a living person is never skipped"
        );

        assert!(unretire(db, "t", KEY, "admin").unwrap());
        assert!(!is_retired(db, "t", KEY).unwrap());
        assert!(!unretire(db, "t", KEY, "admin").unwrap());
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
