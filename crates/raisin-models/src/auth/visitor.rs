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

//! Visitor sessions: the ephemeral home of an ANONYMOUS chat.
//!
//! A website visitor has no `raisin:User` and therefore no home, while the
//! chat pipeline (`{home}/outbox` → delivery → the agent → the reply in
//! `{home}/inbox/chats/…`) is built around homes. A visitor session gives an
//! anonymous WebSocket session exactly that, and nothing more:
//!
//! ```text
//! raisin:access_control
//!   /visitors                     (raisin:AclFolder, created on first use)
//!     /<key>                      (raisin:VisitorSession — the session home)
//!       /outbox/msg-…             written by the server, never by the visitor
//!       /inbox/chats/<conv-id>/…  delivered by the pipeline
//! ```
//!
//! `<key>` is derived from a random session secret that only the visitor's
//! browser holds (see the WebSocket transport). The path therefore names the
//! session without revealing the credential that proves it.
//!
//! # The isolation rule
//!
//! Everything under [`VISITOR_ROOT`] in [`VISITOR_WORKSPACE`] is decided by
//! [`visitor_zone_decision`], BEFORE any role grant is consulted:
//!
//! * the system and `system_admin` may do anything (decided by the caller);
//! * the session whose [`AuthContext::visitor_home`] is that home may READ it
//!   and everything below it;
//! * nobody else may do anything — not another visitor, not a logged-in user,
//!   not a role that happens to grant `read` on `raisin:access_control/**`.
//!
//! Writes are never granted to a visitor. The server writes the session's
//! outbox and home on its behalf (as the system), after the checks the agent's
//! anonymous configuration asks for; the pipeline delivers into it as the
//! system. So a visitor cannot bypass a rate limit by writing an outbox node
//! directly, and no grant anywhere can open one visitor's conversation to
//! another.
//!
//! [`AuthContext::visitor_home`]: crate::auth::AuthContext::visitor_home

use crate::permissions::Operation;

/// The workspace visitor homes live in (the same one as user homes, so the
/// messaging pipeline treats both alike).
pub const VISITOR_WORKSPACE: &str = "raisin:access_control";

/// The folder every visitor home lives under.
pub const VISITOR_ROOT: &str = "/visitors";

/// The node type of a visitor home.
pub const VISITOR_SESSION_NODE_TYPE: &str = "raisin:VisitorSession";

/// Prefix of a visitor's participant id in conversations (`visitor:<key>`).
pub const VISITOR_ID_PREFIX: &str = "visitor:";

/// The home path of the visitor session `key`.
pub fn visitor_home_path(key: &str) -> String {
    format!("{VISITOR_ROOT}/{key}")
}

/// The participant id of the visitor session `key`.
pub fn visitor_participant_id(key: &str) -> String {
    format!("{VISITOR_ID_PREFIX}{key}")
}

/// Whether a session key has the shape the server mints (lowercase hex,
/// 16–128 chars). Anything else is refused before it can reach a path.
pub fn is_valid_visitor_key(key: &str) -> bool {
    (16..=128).contains(&key.len())
        && key
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Whether `path` in `workspace` is inside the visitor zone.
pub fn is_visitor_path(workspace: &str, path: &str) -> bool {
    workspace == VISITOR_WORKSPACE
        && (path == VISITOR_ROOT
            || path
                .strip_prefix(VISITOR_ROOT)
                .is_some_and(|rest| rest.starts_with('/')))
}

/// Whether `path` is `home` or below it.
fn within(home: &str, path: &str) -> bool {
    path == home
        || path
            .strip_prefix(home)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The visitor-zone rule for one operation on one path.
///
/// `None` when the path is outside the zone: the ordinary grants decide.
/// `Some(allowed)` when it is inside: the answer is final and no role grant
/// may widen it. The caller has already let the system and `system_admin`
/// through.
pub fn visitor_zone_decision(
    workspace: &str,
    path: &str,
    operation: Operation,
    visitor_home: Option<&str>,
) -> Option<bool> {
    if !is_visitor_path(workspace, path) {
        return None;
    }
    let owns = visitor_home
        .filter(|home| is_visitor_path(workspace, home) && *home != VISITOR_ROOT)
        .is_some_and(|home| within(home, path));
    Some(owns && operation == Operation::Read)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME_A: &str = "/visitors/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const HOME_B: &str = "/visitors/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const WS: &str = VISITOR_WORKSPACE;

    #[test]
    fn outside_the_zone_the_ordinary_grants_decide() {
        assert_eq!(
            visitor_zone_decision(WS, "/users/alice", Operation::Read, None),
            None
        );
        assert_eq!(
            visitor_zone_decision("content", "/visitors/x", Operation::Read, None),
            None
        );
        // A sibling that merely shares the prefix is not in the zone.
        assert_eq!(
            visitor_zone_decision(WS, "/visitorsx/a", Operation::Read, None),
            None
        );
    }

    #[test]
    fn a_session_reads_its_own_home_and_below() {
        for path in [
            HOME_A.to_string(),
            format!("{HOME_A}/inbox/chats/vchat-1"),
            format!("{HOME_A}/inbox/chats/vchat-1/msg-1"),
        ] {
            assert_eq!(
                visitor_zone_decision(WS, &path, Operation::Read, Some(HOME_A)),
                Some(true),
                "{path}"
            );
        }
    }

    #[test]
    fn a_session_never_writes_even_its_own_home() {
        for op in [Operation::Create, Operation::Update, Operation::Delete] {
            assert_eq!(
                visitor_zone_decision(WS, &format!("{HOME_A}/outbox/m"), op, Some(HOME_A)),
                Some(false)
            );
        }
    }

    #[test]
    fn another_session_and_everyone_else_see_nothing() {
        let chat = format!("{HOME_A}/inbox/chats/vchat-1");
        assert_eq!(
            visitor_zone_decision(WS, &chat, Operation::Read, Some(HOME_B)),
            Some(false)
        );
        assert_eq!(
            visitor_zone_decision(WS, &chat, Operation::Read, None),
            Some(false)
        );
        // The root lists every session: nobody but the system reads it.
        assert_eq!(
            visitor_zone_decision(WS, VISITOR_ROOT, Operation::Read, Some(HOME_A)),
            Some(false)
        );
    }

    /// A home that is a prefix of another key must not reach into it.
    #[test]
    fn a_home_does_not_reach_a_longer_sibling() {
        let short = "/visitors/aaaaaaaaaaaaaaaa";
        let long = "/visitors/aaaaaaaaaaaaaaaab/inbox";
        assert_eq!(
            visitor_zone_decision(WS, long, Operation::Read, Some(short)),
            Some(false)
        );
    }

    /// A visitor_home that is not a visitor home (a forged or buggy value)
    /// opens nothing.
    #[test]
    fn a_bogus_home_opens_nothing() {
        let chat = format!("{HOME_A}/inbox");
        for bogus in ["/visitors", "/", "/users/alice", ""] {
            assert_eq!(
                visitor_zone_decision(WS, &chat, Operation::Read, Some(bogus)),
                Some(false),
                "{bogus:?}"
            );
        }
    }

    #[test]
    fn only_minted_keys_are_valid() {
        assert!(is_valid_visitor_key("0123456789abcdef0123456789abcdef"));
        assert!(!is_valid_visitor_key("short"));
        assert!(!is_valid_visitor_key("../../users/alice/xxxxxxxxxxxx"));
        assert!(!is_valid_visitor_key("0123456789ABCDEF0123456789ABCDEF"));
    }
}
