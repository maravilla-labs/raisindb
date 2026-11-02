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

//! The stable person key of a `raisin:User`: its home path segment.
//!
//! A user's home is `/users/internal/<user_key>` in `raisin:access_control`,
//! where `user_key` is `u_` followed by 22 random base62 characters (24 in
//! total, URL-safe). The key is minted by the engine when the user node is
//! created and is never derived from the email address or the identity UUID.
//!
//! Why the path and not the node id: node ids are regenerated when content is
//! recreated or installed from hand-written YAML, while a path survives package
//! installs and backup restores unchanged. The identity UUID (`user_id`) is
//! rebound on re-registration and lives in tenant storage, outside repository
//! backups. The home path is already what `auth.home`, the JWT `home` claim
//! and every reference to a user's home carry, so making it opaque and
//! immutable gives one key everywhere.
//!
//! Users created before keys existed keep their old (email slug) homes; for
//! them [`user_key_from_home`] returns `None` and `auth.user_key` is absent.

/// Prefix of every user key.
pub const USER_KEY_PREFIX: &str = "u_";

/// Number of random base62 characters after the prefix.
pub const USER_KEY_RANDOM_LEN: usize = 22;

/// Total length of a user key (`u_` + 22).
pub const USER_KEY_LEN: usize = 24;

/// The folder that holds keyed user homes.
pub const USER_HOMES_ROOT: &str = "/users/internal";

/// The node type of a user home.
pub const USER_NODE_TYPE: &str = "raisin:User";

/// The workspace user homes live in.
pub const USER_WORKSPACE: &str = "raisin:access_control";

const BASE62: [char; 62] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I',
    'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', 'a', 'b',
    'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u',
    'v', 'w', 'x', 'y', 'z',
];

/// Whether `s` is a well-formed user key: `u_` + exactly 22 ASCII base62
/// characters.
pub fn is_user_key(s: &str) -> bool {
    s.len() == USER_KEY_LEN
        && s.starts_with(USER_KEY_PREFIX)
        && s[USER_KEY_PREFIX.len()..]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric())
}

/// Mint a fresh user key (`u_` + 22 random base62 characters, about 131 bits).
pub fn mint_user_key() -> String {
    format!(
        "{}{}",
        USER_KEY_PREFIX,
        nanoid::nanoid!(USER_KEY_RANDOM_LEN, &BASE62)
    )
}

/// The user key of a home path, when its last segment is one.
///
/// Only the exact shape `/users/internal/<key>` gives the key: the parent is
/// exactly [`USER_HOMES_ROOT`], one segment, in the key format. A legacy
/// email-slug home, a visitor home, `/users/<key>`, a key-looking segment
/// under any other parent or deeper (`/users/internal/x/u_…`) gives `None`.
pub fn user_key_from_home(home: &str) -> Option<&str> {
    let rest = home.strip_prefix(USER_HOMES_ROOT)?.strip_prefix('/')?;
    is_user_key(rest).then_some(rest)
}

/// The keyed home path for `key`: `/users/internal/<key>`.
pub fn user_home_for_key(key: &str) -> String {
    format!("{}/{}", USER_HOMES_ROOT, key)
}

/// Whether a node of `node_type` created in `workspace` under `parent_path`
/// is a keyed user home (a `raisin:User` directly under
/// [`USER_HOMES_ROOT`] in [`USER_WORKSPACE`]).
///
/// Users elsewhere (the built-in `/users/system/anonymous`, homes a package
/// places in another folder) are not keyed homes and keep their names.
pub fn is_keyed_home_location(workspace: &str, node_type: &str, parent_path: &str) -> bool {
    workspace == USER_WORKSPACE
        && node_type == USER_NODE_TYPE
        && parent_path.trim_end_matches('/') == USER_HOMES_ROOT
}

/// Name a new user home by a freshly minted key.
///
/// Applies only to a keyed home location (see [`is_keyed_home_location`]);
/// the parent is taken from `node.path` when set, else from `parent_path`.
/// Sets `node.name` and, when the path is known, `node.path`. Returns the key
/// when the node was (re)named, `None` when nothing changed.
///
/// A name or path supplied by the caller is NEVER adopted, even when it looks
/// like a key: whoever can influence a created node's name must not pick, or
/// re-claim, a person's key. Only `adopt_supplied_key` (a content import by a
/// system principal: package install, where preserving the key is the point)
/// keeps the path the content carries (a key, or a legacy home). The storage
/// write path is the one place this runs for every create
/// (`raisin_rocksdb::user_homes`).
pub fn assign_user_key_name(
    workspace: &str,
    node: &mut crate::nodes::Node,
    parent_path: Option<&str>,
    adopt_supplied_key: bool,
) -> Option<String> {
    let parent = if node.path.is_empty() {
        parent_path?.to_string()
    } else {
        match node.path.rsplit_once('/') {
            Some(("", _)) => "/".to_string(),
            Some((p, _)) => p.to_string(),
            None => return None,
        }
    };
    if !is_keyed_home_location(workspace, &node.node_type, &parent) {
        return None;
    }
    if adopt_supplied_key {
        // An import keeps the path it carries: a key, or a legacy home that
        // predates keys (renaming it would orphan the content below it).
        return None;
    }
    let key = mint_user_key();
    node.name = key.clone();
    if !node.path.is_empty() {
        node.path = format!("{}/{}", parent.trim_end_matches('/'), key);
    }
    Some(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::Node;

    fn user(path: &str, name: &str) -> Node {
        Node {
            node_type: USER_NODE_TYPE.into(),
            path: path.into(),
            name: name.into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_new_home_under_users_internal_gets_a_key() {
        let mut n = user("/users/internal/alice-at-x-com", "alice-at-x-com");
        let key = assign_user_key_name(USER_WORKSPACE, &mut n, None, false).expect("renamed");
        assert!(is_user_key(&key));
        assert_eq!(n.name, key);
        assert_eq!(n.path, format!("/users/internal/{key}"));

        // Path not known yet: the parent comes from the caller.
        let mut n = user("", "bob");
        let key =
            assign_user_key_name(USER_WORKSPACE, &mut n, Some("/users/internal"), false).unwrap();
        assert_eq!(n.name, key);
        assert!(n.path.is_empty());
    }

    #[test]
    fn a_supplied_key_is_never_adopted_outside_an_import() {
        let key = "u_0123456789abcdefABCDEF";
        let mut n = user(&format!("/users/internal/{key}"), key);
        let minted = assign_user_key_name(USER_WORKSPACE, &mut n, None, false).expect("minted");
        assert_ne!(minted, key, "a key-shaped name is replaced");
        assert_eq!(n.path, format!("/users/internal/{minted}"));

        // An import keeps the path it carries, a key or a legacy home.
        let mut n = user(&format!("/users/internal/{key}"), key);
        assert_eq!(
            assign_user_key_name(USER_WORKSPACE, &mut n, None, true),
            None
        );
        assert_eq!(n.path, format!("/users/internal/{key}"));
        let mut n = user("/users/internal/alice", "alice");
        assert_eq!(
            assign_user_key_name(USER_WORKSPACE, &mut n, None, true),
            None
        );
        assert_eq!(n.path, "/users/internal/alice");
    }

    #[test]
    fn other_places_and_other_types_are_left_alone() {
        let mut n = user("/users/system/anonymous", "anonymous");
        assert_eq!(
            assign_user_key_name(USER_WORKSPACE, &mut n, None, false),
            None
        );

        let mut n = user("/users/internal/alice", "alice");
        assert_eq!(assign_user_key_name("people", &mut n, None, false), None);

        let mut n = user("/users/internal/inbox", "inbox");
        n.node_type = "raisin:Folder".into();
        assert_eq!(
            assign_user_key_name(USER_WORKSPACE, &mut n, None, false),
            None
        );
    }

    #[test]
    fn minted_keys_are_well_formed_and_distinct() {
        let a = mint_user_key();
        let b = mint_user_key();
        assert!(is_user_key(&a), "{a}");
        assert!(is_user_key(&b), "{b}");
        assert_eq!(a.len(), USER_KEY_LEN);
        assert_ne!(a, b);
    }

    #[test]
    fn is_user_key_rejects_everything_else() {
        assert!(is_user_key("u_0123456789abcdefABCDEF"));
        assert!(!is_user_key("u_0123456789abcdefABCDE")); // 21
        assert!(!is_user_key("u_0123456789abcdefABCDEFG")); // 23
        assert!(!is_user_key("x_0123456789abcdefABCDEF"));
        assert!(!is_user_key("u_0123456789abcdef-BCDEF"));
        assert!(!is_user_key("u_0123456789abcdef_BCDEF"));
        assert!(!is_user_key("u_0123456789abcdefABCDé"));
        assert!(!is_user_key("alice-at-example-com"));
        assert!(!is_user_key(""));
    }

    #[test]
    fn key_from_home_takes_only_the_exact_keyed_home() {
        let key = "u_0123456789abcdefABCDEF";
        assert_eq!(user_key_from_home(&user_home_for_key(key)), Some(key));
        assert_eq!(user_key_from_home("/users/internal/alice-at-x-com"), None);
        assert_eq!(
            user_key_from_home("/users/internal/guest0e1-at-guest-invalid"),
            None
        );
        assert_eq!(
            user_key_from_home("/users/internal/u_0123456789abcdefABCDEF/inbox"),
            None
        );
        assert_eq!(user_key_from_home(""), None);
        // Only /users/internal/<key>, nothing deeper, nothing elsewhere.
        assert_eq!(user_key_from_home(&format!("/users/{key}")), None);
        assert_eq!(
            user_key_from_home(&format!("/users/internal/x/{key}")),
            None
        );
        assert_eq!(user_key_from_home(&format!("/other/internal/{key}")), None);
        assert_eq!(user_key_from_home(&format!("/visitors/{key}")), None);
        assert_eq!(user_key_from_home(&format!("users/internal/{key}")), None);
    }
}
