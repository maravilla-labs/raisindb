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

//! Keyed user homes on the write path (see `raisin_models::auth::user_key`).
//!
//! A `raisin:User` created directly under `/users/internal` in
//! `raisin:access_control` is named by a minted user key; its path is the
//! person's stable key and is never moved or renamed. Two rules, each applied
//! by the low-level writers so no entry point can miss them:
//!
//! - [`name_new_user_home`]: on CREATE, a home is named by a freshly minted
//!   key. A supplied name is never adopted, even when it looks like a key
//!   (nobody may pick or re-claim a person's key), except in a content import
//!   by a system principal (package install), which keeps the key it carries;
//!   [`reject_user_home_takeover`] refuses an import whose key is already the
//!   home of a different person. Backup restore writes records as they are.
//! - [`reject_user_home_move`]: a move or rename of a user home, or of the
//!   folders that hold every home (`/users`, `/users/internal`), is refused,
//!   except inside a content import (a package migration).
//!
//! Neither applies on the replication apply path, for the reason
//! `crate::immutability` gives: the peer already accepted the write.

use raisin_error::{Error, Result};
use raisin_models::auth::user_key::{
    assign_user_key_name, USER_HOMES_ROOT, USER_NODE_TYPE, USER_WORKSPACE,
};
use raisin_models::nodes::Node;

/// Name a new user home by a freshly minted key that is not retired.
///
/// `adopt_supplied_key` (a content import by a system principal) keeps the
/// path the import carries; nothing else does. A carried key that is retired
/// (`crate::retired_keys`) is refused unless `allow_retired` (the explicit
/// admin override of an install or restore) is set.
pub(crate) fn name_new_user_home(
    db: &rocksdb::DB,
    tenant_id: &str,
    workspace: &str,
    node: &mut Node,
    adopt_supplied_key: bool,
    allow_retired: bool,
) -> Result<()> {
    let before = node.path.clone();
    if adopt_supplied_key {
        assign_user_key_name(workspace, node, None, true);
        if let Some(key) = crate::retired_keys::home_key(workspace, node) {
            if !allow_retired && crate::retired_keys::is_retired(db, tenant_id, key)? {
                return Err(Error::Validation(format!(
                    "user key {key} is retired (its person was deleted) and cannot be restored"
                )));
            }
        }
        return Ok(());
    }
    // A minted key is never a retired one (131 random bits: a retry is
    // practically never needed, the check is what guarantees it).
    if assign_user_key_name(workspace, node, None, false).is_none() {
        return Ok(());
    }
    let key = mint_unretired(
        |k| crate::retired_keys::is_retired(db, tenant_id, k),
        raisin_models::auth::mint_user_key,
    )?;
    let parent = node
        .path
        .rsplit_once('/')
        .map(|(p, _)| p.to_string())
        .unwrap_or_default();
    node.name = key.clone();
    if !node.path.is_empty() {
        node.path = format!("{parent}/{key}");
    }
    tracing::info!(
        requested_path = %before,
        user_path = %node.path,
        user_key = %key,
        "User home named by its minted key"
    );
    Ok(())
}

/// Mint keys until one is not retired (at most 8 tries).
fn mint_unretired(
    is_retired: impl Fn(&str) -> Result<bool>,
    mut mint: impl FnMut() -> String,
) -> Result<String> {
    for _ in 0..8 {
        let key = mint();
        if !is_retired(&key)? {
            return Ok(key);
        }
    }
    Err(Error::storage(
        "could not mint a user key that is not retired".to_string(),
    ))
}

/// Refuse an import that writes a keyed home over the home of a DIFFERENT
/// person (another email) at the same key: a package must not re-claim a
/// key. The same person (same email, or no email to compare) is an update.
pub(crate) fn reject_user_home_takeover(
    workspace: &str,
    prior: &Node,
    incoming: &Node,
    content_import: bool,
) -> Result<()> {
    if !content_import
        || workspace != USER_WORKSPACE
        || prior.node_type != USER_NODE_TYPE
        || raisin_models::auth::user_key::user_key_from_home(&prior.path).is_none()
    {
        return Ok(());
    }
    let email = |n: &Node| match n.properties.get("email") {
        Some(raisin_models::nodes::properties::PropertyValue::String(s)) => {
            Some(s.trim().to_lowercase())
        }
        _ => None,
    };
    match (email(prior), email(incoming)) {
        (Some(a), Some(b)) if a != b => Err(Error::Conflict(format!(
            "user key collision: '{}' is the home of another person; an import cannot \
             re-claim it",
            prior.path
        ))),
        _ => Ok(()),
    }
}

/// Refuse moving or renaming `node` (in `workspace`) to `new_path` when it is
/// a user home or a folder holding every home.
pub(crate) fn reject_user_home_move(
    workspace: &str,
    node: &Node,
    new_path: &str,
    content_import: bool,
) -> Result<()> {
    if content_import || workspace != USER_WORKSPACE || node.path == new_path {
        return Ok(());
    }
    let holds_homes = node.path == "/users" || node.path == USER_HOMES_ROOT;
    if node.node_type == USER_NODE_TYPE || holds_homes {
        return Err(Error::Validation(format!(
            "'{}' cannot be moved or renamed: a user's home path is the user's stable key \
             (raisin:User homes are immutable; delete and recreate the user instead)",
            node.path
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::auth::user_key::is_user_key;

    fn node(path: &str, node_type: &str) -> Node {
        Node {
            id: "n".into(),
            path: path.into(),
            name: path.rsplit('/').next().unwrap_or_default().into(),
            node_type: node_type.into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_created_home_is_always_freshly_keyed_unless_an_import_carries_a_key() {
        let dir = tempfile::tempdir().unwrap();
        let storage = crate::RocksDBStorage::new(dir.path()).unwrap();
        let db = storage.db().as_ref();
        let mut n = node("/users/internal/alice", USER_NODE_TYPE);
        name_new_user_home(db, "t", USER_WORKSPACE, &mut n, false, false).unwrap();
        assert!(is_user_key(&n.name), "{}", n.name);
        assert_eq!(n.path, format!("/users/internal/{}", n.name));

        let key = "u_0123456789abcdefABCDEF";
        let mut n = node(&format!("/users/internal/{key}"), USER_NODE_TYPE);
        name_new_user_home(db, "t", USER_WORKSPACE, &mut n, false, false).unwrap();
        assert_ne!(n.name, key, "a supplied key is not adopted");

        let mut n = node(&format!("/users/internal/{key}"), USER_NODE_TYPE);
        name_new_user_home(db, "t", USER_WORKSPACE, &mut n, true, false).unwrap();
        assert_eq!(n.name, key, "an import keeps its key");

        // A retired key is refused on import, unless the admin override.
        let mut b = rocksdb::WriteBatch::default();
        crate::retired_keys::stage_retire(&mut b, db, "t", USER_WORKSPACE, &n).unwrap();
        db.write(b).unwrap();
        let mut again = node(&format!("/users/internal/{key}"), USER_NODE_TYPE);
        assert!(name_new_user_home(db, "t", USER_WORKSPACE, &mut again, true, false).is_err());
        assert!(name_new_user_home(db, "t", USER_WORKSPACE, &mut again, true, true).is_ok());
    }

    #[test]
    fn a_retired_key_is_never_minted() {
        let retired = "u_0123456789abcdefABCDEF";
        let mut seq = vec![retired.to_string(), "u_ZZZZZZZZZZZZZZZZZZZZZZ".to_string()].into_iter();
        let key = mint_unretired(|k| Ok(k == retired), || seq.next().unwrap()).unwrap();
        assert_eq!(key, "u_ZZZZZZZZZZZZZZZZZZZZZZ");
        assert!(mint_unretired(|_| Ok(true), || retired.to_string()).is_err());
    }

    #[test]
    fn an_import_cannot_reclaim_another_persons_key() {
        use raisin_models::nodes::properties::PropertyValue;
        let with_email = |e: &str| {
            let mut n = node("/users/internal/u_0123456789abcdefABCDEF", USER_NODE_TYPE);
            n.properties
                .insert("email".into(), PropertyValue::String(e.into()));
            n
        };
        let prior = with_email("a@x.com");
        assert!(
            reject_user_home_takeover(USER_WORKSPACE, &prior, &with_email("b@x.com"), true)
                .is_err()
        );
        assert!(
            reject_user_home_takeover(USER_WORKSPACE, &prior, &with_email("A@X.com"), true).is_ok()
        );
        assert!(
            reject_user_home_takeover(USER_WORKSPACE, &prior, &with_email("b@x.com"), false)
                .is_ok()
        );
    }

    #[test]
    fn homes_and_their_folders_do_not_move() {
        let home = node("/users/internal/u_0123456789abcdefABCDEF", USER_NODE_TYPE);
        assert!(reject_user_home_move(USER_WORKSPACE, &home, "/users/x", false).is_err());
        assert!(
            reject_user_home_move(USER_WORKSPACE, &home, "/users/internal/other", false).is_err()
        );
        let legacy = node("/users/internal/alice-at-x-com", USER_NODE_TYPE);
        assert!(
            reject_user_home_move(USER_WORKSPACE, &legacy, "/users/internal/b", false).is_err()
        );
        let internal = node(USER_HOMES_ROOT, "raisin:AclFolder");
        assert!(reject_user_home_move(USER_WORKSPACE, &internal, "/people", false).is_err());

        // Imports (package migrations), other nodes and other workspaces move.
        assert!(reject_user_home_move(USER_WORKSPACE, &home, "/users/x", true).is_ok());
        let inbox = node(
            "/users/internal/u_0123456789abcdefABCDEF/inbox/a",
            "raisin:Message",
        );
        assert!(reject_user_home_move(
            USER_WORKSPACE,
            &inbox,
            "/users/internal/u_0123456789abcdefABCDEF/inbox/b",
            false
        )
        .is_ok());
        let page = node("/users/internal/x", USER_NODE_TYPE);
        assert!(reject_user_home_move("content", &page, "/y", false).is_ok());
    }
}
