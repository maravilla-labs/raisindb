// SPDX-License-Identifier: BSL-1.1

//! Who may invoke a function from a client.
//!
//! The WebSocket invoke paths found the function node without looking at the
//! caller at all, so any connection — anonymous included — could run any
//! function by path, including `execution_context: "system"` ones that bypass
//! row-level security. HTTP invoke looked the node up under the caller's
//! row-level security, so being able to READ a function meant being able to
//! run it.
//!
//! Invoking is now its own permission, [`Operation::Execute`], granted on the
//! function node (workspace `functions`, the function's path) like any other
//! operation — a Unix `x` bit. `read` does not imply it and it does not imply
//! `read`. The system and `system_admin` (admin console, API keys, CLI) hold it
//! everywhere; everyone else needs a grant, the anonymous principal through the
//! `anonymous` role. Every CLIENT entry point asks this before loading any
//! code. Internal runs (triggers, flows, schedules, agent tool calls,
//! function-to-function calls) are not client invokes and never come here.

use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use raisin_models::permissions::{Operation, PermissionScope};

/// The workspace functions live in.
pub const FUNCTIONS_WORKSPACE: &str = "functions";

/// Why an invoke was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvokeRefusal {
    /// Not signed in, and no grant lets an anonymous caller run it.
    Unauthenticated,
    /// Signed in, without `execute` on this function.
    Forbidden,
}

impl std::fmt::Display for InvokeRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unauthenticated => "this function may not be invoked without signing in",
            Self::Forbidden => "not allowed to execute this function",
        })
    }
}

/// The decision. `auth` is the invoking caller as the transport resolved it
/// (`None`: no identity at all); `function` is the `raisin:Function` node as
/// stored; `branch` the branch it was found on.
pub fn authorize_invoke(
    auth: Option<&AuthContext>,
    function: &Node,
    branch: &str,
) -> Result<(), InvokeRefusal> {
    authorize_invoke_in(auth, function, FUNCTIONS_WORKSPACE, branch)
}

/// [`authorize_invoke`] for a function found in `workspace` (SQL `INVOKE`
/// names the workspace; everything else uses `functions`).
pub fn authorize_invoke_in(
    auth: Option<&AuthContext>,
    function: &Node,
    workspace: &str,
    branch: &str,
) -> Result<(), InvokeRefusal> {
    let Some(auth) = auth else {
        return Err(InvokeRefusal::Unauthenticated);
    };
    let scope = PermissionScope::new(workspace, branch);
    if super::rls_filter::can_perform(function, Operation::Execute, auth, &scope) {
        return Ok(());
    }
    Err(if auth.is_anonymous_principal() {
        InvokeRefusal::Unauthenticated
    } else {
        InvokeRefusal::Forbidden
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::permissions::{Permission, ResolvedPermissions};

    fn function(path: &str) -> Node {
        Node {
            id: format!("fn{path}"),
            name: path.rsplit('/').next().unwrap_or_default().to_string(),
            path: path.to_string(),
            node_type: "raisin:Function".to_string(),
            workspace: Some(FUNCTIONS_WORKSPACE.to_string()),
            ..Default::default()
        }
    }

    fn with_grants(user: &str, grants: Vec<Permission>) -> AuthContext {
        let mut p = ResolvedPermissions::empty(user);
        p.permissions = grants;
        AuthContext::for_user(user).with_permissions(p)
    }

    fn grant(path: &str, ops: Vec<Operation>) -> Permission {
        Permission::new(path, ops).with_workspace(FUNCTIONS_WORKSPACE)
    }

    const F: &str = "/lib/studio/collect-publish-tree";

    #[test]
    fn execute_granted_invokes() {
        let editor = with_grants(
            "ed",
            vec![grant("/lib/studio/**", vec![Operation::Execute])],
        );
        assert_eq!(
            authorize_invoke(Some(&editor), &function(F), "main"),
            Ok(())
        );
        // The grant is by path: another tree stays closed.
        assert_eq!(
            authorize_invoke(Some(&editor), &function("/lib/other/x"), "main"),
            Err(InvokeRefusal::Forbidden)
        );
    }

    /// `read` is not `execute`: seeing the code does not let you run it.
    #[test]
    fn read_only_is_refused() {
        let reader = with_grants("rd", vec![grant("/**", vec![Operation::Read])]);
        assert_eq!(
            authorize_invoke(Some(&reader), &function(F), "main"),
            Err(InvokeRefusal::Forbidden)
        );
        // ...and execute does not need read.
        let runner = with_grants("rn", vec![grant("/**", vec![Operation::Execute])]);
        assert_eq!(
            authorize_invoke(Some(&runner), &function(F), "main"),
            Ok(())
        );
    }

    #[test]
    fn anonymous_needs_an_explicit_grant() {
        let bare = AuthContext::anonymous_user("anon")
            .with_permissions(ResolvedPermissions::anonymous(vec![]));
        for who in [Some(&bare), None, Some(&AuthContext::deny_all())] {
            assert_eq!(
                authorize_invoke(who, &function(F), "main"),
                Err(InvokeRefusal::Unauthenticated)
            );
        }
        let public =
            AuthContext::anonymous_user("anon").with_permissions(ResolvedPermissions::anonymous(
                vec![grant("/lib/site/public/**", vec![Operation::Execute])],
            ));
        assert_eq!(
            authorize_invoke(Some(&public), &function("/lib/site/public/search"), "main"),
            Ok(())
        );
        assert_eq!(
            authorize_invoke(Some(&public), &function(F), "main"),
            Err(InvokeRefusal::Unauthenticated)
        );
    }

    #[test]
    fn administrators_and_the_system_invoke_anything() {
        let admin =
            AuthContext::for_user("root").with_permissions(ResolvedPermissions::system_admin());
        for who in [admin, AuthContext::system()] {
            assert_eq!(authorize_invoke(Some(&who), &function(F), "main"), Ok(()));
        }
    }

    /// The shape Studio's editor roles use: top-level functions by `*`, whole
    /// groups by `**`, and nothing under a group that is not listed.
    #[test]
    fn a_single_segment_grant_does_not_reach_into_groups() {
        let editor = with_grants(
            "ed",
            vec![
                grant("/lib/studio/*", vec![Operation::Execute]),
                grant("/lib/studio/commerce/**", vec![Operation::Execute]),
            ],
        );
        for ok in [F, "/lib/studio/commerce/receive-stock"] {
            assert_eq!(
                authorize_invoke(Some(&editor), &function(ok), "main"),
                Ok(()),
                "{ok}"
            );
        }
        for no in [
            "/lib/studio/builder/execute-function",
            "/lib/studio/automations/verify-automation",
            "/lib/other/studio/x",
        ] {
            assert_eq!(
                authorize_invoke(Some(&editor), &function(no), "main"),
                Err(InvokeRefusal::Forbidden),
                "{no}"
            );
        }
    }

    #[test]
    fn execute_is_a_known_operation() {
        assert_eq!(Operation::parse("execute"), Some(Operation::Execute));
        assert_eq!(Operation::Execute.to_string(), "execute");
        assert!(Operation::all().contains(&Operation::Execute));
    }
}
