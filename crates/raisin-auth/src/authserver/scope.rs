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

//! Mapping between OAuth scopes and the resource owner's access-control grants.
//!
//! RaisinDB authorization is role/group + path RBAC resolved per request from
//! the `raisin:access_control` workspace; there is no standalone "scope" notion
//! in the data model. To keep an OAuth access token honest, the server treats a
//! scope string as the name of a role or group the identity already holds.
//!
//! The MCP transport derives a caller's granted scopes from
//! `roles ∪ groups` (see the HTTP transport's MCP identity bridge), so emitting
//! the consented role/group names as the token `scope` claim makes the token's
//! scopes line up exactly with the scopes dispatch gates on and with the RLS
//! the same roles drive. Consent therefore *narrows* — never widens — what the
//! identity could already do.

use std::collections::BTreeSet;

/// The grants an identity holds, as resolved from `raisin:access_control`.
///
/// This is the minimal projection the scope mapper needs; the HTTP layer fills
/// it from a [`ResolvedPermissions`](raisin_models::permissions::ResolvedPermissions)
/// (its `effective_roles` and `groups`).
#[derive(Debug, Clone, Default)]
pub struct IdentityGrants {
    /// Effective role ids the identity holds (direct + inherited + via groups).
    pub roles: Vec<String>,
    /// Group ids the identity belongs to.
    pub groups: Vec<String>,
    /// Whether the identity is a system administrator (holds every grant).
    pub is_system_admin: bool,
}

impl IdentityGrants {
    /// Construct from role and group lists.
    pub fn new(roles: Vec<String>, groups: Vec<String>, is_system_admin: bool) -> Self {
        Self {
            roles,
            groups,
            is_system_admin,
        }
    }

    /// The full set of scope strings the identity could be granted, namely the
    /// union of its roles and groups.
    pub fn grantable_scopes(&self) -> BTreeSet<String> {
        self.roles.iter().chain(self.groups.iter()).cloned().collect()
    }
}

/// Parse a space-delimited OAuth `scope` string into distinct tokens.
pub fn parse_scope(scope: &str) -> Vec<String> {
    scope.split_whitespace().map(str::to_string).collect()
}

/// Join scope tokens back into the canonical space-delimited form.
pub fn join_scope<I, S>(scopes: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    scopes
        .into_iter()
        .map(|s| s.as_ref().to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Compute the scopes that may be *granted* for a request.
///
/// `requested` is what the client asked for (already validated against the
/// client's registered scope set by the caller). `grants` is what the identity
/// actually holds. The result is the intersection — every granted scope is one
/// the identity can back with a real role/group — preserving insertion order of
/// `requested` so the token scope reflects the client's ordering.
///
/// A system administrator is granted everything requested, because such an
/// identity holds all roles implicitly.
pub fn grant_scopes(requested: &[String], grants: &IdentityGrants) -> Vec<String> {
    if grants.is_system_admin {
        return requested.to_vec();
    }
    let holdable = grants.grantable_scopes();
    requested
        .iter()
        .filter(|s| holdable.contains(*s))
        .cloned()
        .collect()
}

/// Validate that every requested scope is one the client may request.
///
/// Returns the offending scope when `requested` contains a token outside
/// `registered`. An empty `registered` set means the client placed no scope
/// restriction on itself, so any request is structurally allowed (it is still
/// narrowed to the identity's grants by [`grant_scopes`]).
pub fn check_requested_against_client<'a>(
    requested: &'a [String],
    registered: &[String],
) -> Result<(), &'a str> {
    if registered.is_empty() {
        return Ok(());
    }
    let allowed: BTreeSet<&str> = registered.iter().map(String::as_str).collect();
    for scope in requested {
        if !allowed.contains(scope.as_str()) {
            return Err(scope.as_str());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_is_intersection_of_request_and_holdings() {
        let grants = IdentityGrants::new(
            vec!["reader".to_string(), "editor".to_string()],
            vec!["staff".to_string()],
            false,
        );
        let requested = parse_scope("reader admin staff");
        let granted = grant_scopes(&requested, &grants);
        assert_eq!(granted, vec!["reader".to_string(), "staff".to_string()]);
    }

    #[test]
    fn system_admin_gets_everything_requested() {
        let grants = IdentityGrants::new(vec![], vec![], true);
        let requested = parse_scope("anything goes");
        let granted = grant_scopes(&requested, &grants);
        assert_eq!(granted, requested);
    }

    #[test]
    fn client_scope_restriction_is_enforced() {
        let requested = parse_scope("reader editor");
        let registered = parse_scope("reader");
        let bad = check_requested_against_client(&requested, &registered)
            .expect_err("editor exceeds the client's registered scope");
        assert_eq!(bad, "editor");
    }

    #[test]
    fn empty_client_scope_allows_any_request() {
        let requested = parse_scope("reader editor");
        check_requested_against_client(&requested, &[])
            .expect("no client restriction permits any request");
    }

    #[test]
    fn join_round_trips_parse() {
        let scopes = parse_scope("a b c");
        assert_eq!(join_scope(&scopes), "a b c");
    }
}
