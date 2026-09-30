// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Repository authentication configuration, read at request time.
//!
//! Every identity entry point (password register/login, magic link, OAuth,
//! OIDC) needs the same two answers about a repository: may an unknown caller
//! self-register, and what roles does a freshly provisioned `raisin:User` node
//! get. Both live on the repo's `raisin:RepoAuthConfig` node at
//! `/config/repos/{repo}` in the `raisin:system` workspace.
//!
//! Before this module the answers were inconsistent: only magic link consulted
//! `allow_registration`, and the default roles were the hardcoded literal
//! `["viewer", "authenticated_user"]` at six call sites — never the configured
//! `default_roles`. `viewer` is `{ path: "**", operations: ["read"] }` with no
//! workspace, i.e. read on EVERY workspace, so every self-registered identity
//! silently held a blanket cross-workspace read until (on Studio) a trigger
//! stripped it ~250ms later, and permanently on any repo without that trigger.
//!
//! This module is the single reader, so the endpoints agree.

#[cfg(feature = "storage-rocksdb")]
use raisin_models::auth::AuthContext;
#[cfg(feature = "storage-rocksdb")]
use raisin_models::auth::RepoAuthConfig;
#[cfg(feature = "storage-rocksdb")]
use raisin_models::nodes::properties::PropertyValue;

#[cfg(feature = "storage-rocksdb")]
use crate::state::AppState;

/// The `raisin:RepoAuthConfig` node lives on the system workspace's main branch.
const CONFIG_WORKSPACE: &str = "raisin:system";
const CONFIG_BRANCH: &str = "main";

/// The two repo-level auth answers the identity endpoints need.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedRepoAuth {
    /// May an unknown caller create an account against this repo?
    pub allow_registration: bool,
    /// Roles a freshly provisioned `raisin:User` node is granted. Never
    /// contains `viewer` (see [`sanitize_default_roles`]).
    pub default_roles: Vec<String>,
}

/// Strip the blanket-read `viewer` role and guarantee a non-empty result.
///
/// `viewer` grants read on every workspace, so it must never be handed to a
/// user automatically at provisioning time — that is the whole vulnerability
/// this module closes. An operator who genuinely wants a self-registered user
/// to read everything grants a role for it explicitly after the fact; the
/// automatic default never does. If stripping empties the list we fall back to
/// `authenticated_user`, the baseline every real user needs and the role the
/// Studio profile/draft roles inherit.
pub fn sanitize_default_roles(mut roles: Vec<String>) -> Vec<String> {
    roles.retain(|r| r != "viewer");
    roles.dedup();
    if roles.is_empty() {
        roles.push("authenticated_user".to_string());
    }
    roles
}

/// Read a repo's `raisin:RepoAuthConfig`, falling back to the model defaults.
///
/// A missing node, a read error, or a wrong node type all yield the
/// [`RepoAuthConfig`] defaults, so a repo that never stored a config behaves
/// like the documented default rather than failing closed in a surprising way.
/// The resolved `default_roles` are always run through [`sanitize_default_roles`].
#[cfg(feature = "storage-rocksdb")]
pub async fn resolve_repo_auth(state: &AppState, tenant_id: &str, repo: &str) -> ResolvedRepoAuth {
    let defaults = RepoAuthConfig::default();

    let service = state.node_service_for_context(
        tenant_id,
        repo,
        CONFIG_BRANCH,
        CONFIG_WORKSPACE,
        Some(AuthContext::system()),
    );

    let node = service
        .get_by_path(&format!("/config/repos/{repo}"))
        .await
        .ok()
        .flatten()
        .filter(|n| n.node_type == "raisin:RepoAuthConfig");

    let Some(node) = node else {
        return ResolvedRepoAuth {
            allow_registration: defaults.allow_registration,
            default_roles: sanitize_default_roles(defaults.default_roles),
        };
    };

    let allow_registration = match node.properties.get("allow_registration") {
        Some(PropertyValue::Boolean(allowed)) => *allowed,
        _ => defaults.allow_registration,
    };

    let default_roles = match node.properties.get("default_roles") {
        Some(PropertyValue::Array(items)) => {
            let roles: Vec<String> = items
                .iter()
                .filter_map(|v| match v {
                    PropertyValue::String(s) => Some(s.clone()),
                    _ => None,
                })
                .collect();
            if roles.is_empty() {
                defaults.default_roles.clone()
            } else {
                roles
            }
        }
        _ => defaults.default_roles.clone(),
    };

    ResolvedRepoAuth {
        allow_registration,
        default_roles: sanitize_default_roles(default_roles),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_viewer() {
        assert_eq!(
            sanitize_default_roles(vec!["viewer".to_string(), "authenticated_user".to_string()]),
            vec!["authenticated_user".to_string()]
        );
    }

    #[test]
    fn sanitize_falls_back_when_only_viewer() {
        assert_eq!(
            sanitize_default_roles(vec!["viewer".to_string()]),
            vec!["authenticated_user".to_string()]
        );
    }

    #[test]
    fn sanitize_falls_back_on_empty() {
        assert_eq!(
            sanitize_default_roles(vec![]),
            vec!["authenticated_user".to_string()]
        );
    }

    #[test]
    fn sanitize_keeps_explicit_roles() {
        assert_eq!(
            sanitize_default_roles(vec![
                "authenticated_user".to_string(),
                "studio_member".to_string()
            ]),
            vec![
                "authenticated_user".to_string(),
                "studio_member".to_string()
            ]
        );
    }
}
