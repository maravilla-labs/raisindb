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

//! The ONE rule for "which `raisin:User` node is this identity?".
//!
//! Every place that resolves a person from an identity id goes through
//! [`resolve_user_node`]: the permission service (and with it the HTTP,
//! WebSocket and pgwire auth contexts and `auth.home`), the SQL ACL
//! statements, identity provisioning and the identity cascade. Two different
//! orders would be a differential: a node that one place takes for the person
//! and another does not.
//!
//! 1. By the `user_id` property, `raisin:User` nodes only. Exactly one match
//!    is the answer; MORE THAN ONE is an error (logged, never "the first").
//! 2. Only when there is none: the legacy path `/users/<user_id>`, and only
//!    when the node there is a `raisin:User` whose `user_id` property is the
//!    requested id. Anything else at that path is no match.
//!
//! Keyed homes (`/users/internal/<user_key>`) are found by step 1 like any
//! other home; the path is never used to guess the person.

use raisin_error::{Error, Result};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::{NodeRepository, Storage, StorageScope};

/// The node type of a user home.
pub const USER_NODE_TYPE: &str = "raisin:User";

fn user_id_of(node: &Node) -> Option<&str> {
    match node.properties.get("user_id") {
        Some(PropertyValue::String(s)) => Some(s.as_str()),
        _ => None,
    }
}

/// The decision of [`resolve_user_node`] over the candidates it found, kept
/// pure so it can be tested without storage.
pub fn choose_user_node(
    user_id: &str,
    by_property: Vec<Node>,
    at_legacy_path: Option<Node>,
) -> Result<Option<Node>> {
    let mut matches: Vec<Node> = by_property
        .into_iter()
        .filter(|n| n.node_type == USER_NODE_TYPE && user_id_of(n) == Some(user_id))
        .collect();
    match matches.len() {
        0 => {}
        1 => return Ok(matches.pop()),
        n => {
            let paths: Vec<&str> = matches.iter().map(|m| m.path.as_str()).collect();
            tracing::error!(
                user_id = %user_id,
                count = n,
                paths = ?paths,
                "More than one raisin:User node carries this user_id; refusing to pick one"
            );
            return Err(Error::Conflict(format!(
                "{n} user nodes carry user_id '{user_id}'; the identity is ambiguous"
            )));
        }
    }
    Ok(at_legacy_path.filter(|n| n.node_type == USER_NODE_TYPE && user_id_of(n) == Some(user_id)))
}

/// Resolve the `raisin:User` node of `user_id` in `scope` (see the module
/// documentation for the rule).
pub async fn resolve_user_node<S: Storage>(
    storage: &S,
    scope: StorageScope<'_>,
    user_id: &str,
) -> Result<Option<Node>> {
    let by_property = storage
        .nodes()
        .find_by_property(
            scope,
            "user_id",
            &PropertyValue::String(user_id.to_string()),
        )
        .await?;
    let legacy = if by_property
        .iter()
        .any(|n| n.node_type == USER_NODE_TYPE && user_id_of(n) == Some(user_id))
    {
        None
    } else {
        storage
            .nodes()
            .get_by_path(scope, &format!("/users/{user_id}"), None)
            .await?
    };
    choose_user_node(user_id, by_property, legacy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(path: &str, node_type: &str, user_id: Option<&str>) -> Node {
        let mut n = Node {
            id: path.to_string(),
            path: path.to_string(),
            node_type: node_type.to_string(),
            ..Default::default()
        };
        if let Some(u) = user_id {
            n.properties
                .insert("user_id".into(), PropertyValue::String(u.into()));
        }
        n
    }

    const KEYED: &str = "/users/internal/u_0123456789abcdefABCDEF";

    #[test]
    fn the_property_decides_and_is_unique() {
        let keyed = node(KEYED, USER_NODE_TYPE, Some("idn-1"));
        let got = choose_user_node("idn-1", vec![keyed.clone()], None).unwrap();
        assert_eq!(got.unwrap().path, KEYED);

        // A legacy-path decoy does not outrank the property match.
        let decoy = node("/users/idn-1", USER_NODE_TYPE, Some("someone-else"));
        let got = choose_user_node("idn-1", vec![keyed.clone()], Some(decoy)).unwrap();
        assert_eq!(got.unwrap().path, KEYED);

        // Two nodes with the same user_id: an error, never the first.
        let twin = node(
            "/users/internal/u_ZZZZZZZZZZZZZZZZZZZZZZ",
            USER_NODE_TYPE,
            Some("idn-1"),
        );
        assert!(choose_user_node("idn-1", vec![keyed, twin], None).is_err());

        // A node of another type with that user_id is not the person.
        let folder = node("/users/x", "raisin:Folder", Some("idn-1"));
        assert_eq!(choose_user_node("idn-1", vec![folder], None).unwrap(), None);
    }

    #[test]
    fn the_legacy_path_counts_only_for_that_persons_user_node() {
        let legacy = node("/users/idn-1", USER_NODE_TYPE, Some("idn-1"));
        let got = choose_user_node("idn-1", vec![], Some(legacy)).unwrap();
        assert_eq!(got.unwrap().path, "/users/idn-1");

        // A decoy of another type at the legacy path.
        let decoy = node("/users/idn-1", "raisin:Folder", Some("idn-1"));
        assert_eq!(
            choose_user_node("idn-1", vec![], Some(decoy)).unwrap(),
            None
        );
        // A raisin:User at the legacy path that belongs to someone else.
        let other = node("/users/idn-1", USER_NODE_TYPE, Some("idn-2"));
        assert_eq!(
            choose_user_node("idn-1", vec![], Some(other)).unwrap(),
            None
        );
        // A raisin:User at the legacy path without a user_id.
        let bare = node("/users/idn-1", USER_NODE_TYPE, None);
        assert_eq!(choose_user_node("idn-1", vec![], Some(bare)).unwrap(), None);
    }
}
