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
//! - [`name_new_user_home`]: on CREATE, a home whose name is not a key gets
//!   one. Every creator in the engine names the home itself first (so it
//!   knows the path it created); this is the net for any that does not.
//!   Content imports (package install, restore) keep their paths.
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

/// Name a new user home by a minted key (no-op for imports, other nodes, and
/// homes already named by a key).
pub(crate) fn name_new_user_home(workspace: &str, node: &mut Node, content_import: bool) {
    if content_import {
        return;
    }
    let before = node.path.clone();
    if let Some(key) = assign_user_key_name(workspace, node, None) {
        tracing::info!(
            requested_path = %before,
            user_path = %node.path,
            user_key = %key,
            "User home named by its minted key"
        );
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
    fn a_created_home_is_keyed_unless_imported() {
        let mut n = node("/users/internal/alice", USER_NODE_TYPE);
        name_new_user_home(USER_WORKSPACE, &mut n, false);
        assert!(is_user_key(&n.name), "{}", n.name);
        assert_eq!(n.path, format!("/users/internal/{}", n.name));

        let mut n = node("/users/internal/alice", USER_NODE_TYPE);
        name_new_user_home(USER_WORKSPACE, &mut n, true);
        assert_eq!(n.path, "/users/internal/alice");
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
